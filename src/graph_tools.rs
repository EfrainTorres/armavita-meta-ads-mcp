//! Small shared boundaries for the additional Graph object families.
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    bounded_json::{credential_key, credential_value, encode_nonempty_object, request_control_key},
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids,
    mutation_result::{ambiguous_mutation_result, mutation_error_without_blind_retry},
};

pub(crate) type Params = Vec<(String, String)>;
const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const VERIFY_WRITE: &str =
    "Read the affected object before retrying; Meta may have applied the change";

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadOptions {
    /// Simple field names; omit for compact defaults. No Graph field expansion.
    #[schemars(length(min = 1, max = 30), inner(length(min = 1, max = 64)))]
    pub fields: Option<Vec<String>>,
    /// One bounded page, default 25.
    #[schemars(range(min = 1, max = 100))]
    pub page_size: Option<u16>,
    /// Opaque next_cursor from the preceding result.
    #[schemars(length(min = 1, max = 2048))]
    pub page_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct GraphData {
    /// Selected object fields, one page of rows, or Meta's write acknowledgement.
    pub result: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

pub(crate) fn id(raw: &str, field: &str) -> Result<String, PublicError> {
    meta_ids::numeric_owned(raw)
        .ok_or_else(|| invalid(format!("{field} must be a numeric Meta ID")))
}

pub(crate) fn account(raw: &str) -> Result<String, PublicError> {
    meta_ids::ad_account(raw)
        .ok_or_else(|| invalid("ad_account_id must be a numeric Meta account ID"))
}

/// Meta requires Instagram Feed alongside explicitly selected Threads delivery.
pub(crate) fn validate_threads_placements(targeting: &Value) -> Result<(), PublicError> {
    let contains = |key: &str, value: &str| {
        targeting
            .get(key)
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(|item| item.as_str() == Some(value)))
    };
    let threads_selected = contains("publisher_platforms", "threads")
        || contains("threads_positions", "threads_stream");
    let missing_instagram_feed = (targeting
        .get("publisher_platforms")
        .is_some_and(|value| !value.is_null())
        && !contains("publisher_platforms", "instagram"))
        || (targeting
            .get("instagram_positions")
            .is_some_and(|value| !value.is_null())
            && !contains("instagram_positions", "stream"));
    if threads_selected && missing_instagram_feed {
        return Err(PublicError::invalid_input(
            "Threads delivery requires Instagram Feed in the same placement selection",
            "Include instagram in publisher_platforms and stream in instagram_positions, or leave placement selection automatic",
        ));
    }
    Ok(())
}

pub(crate) fn text(raw: &str, field: &str, max: usize) -> Result<String, PublicError> {
    let value = raw.trim();
    if value.is_empty()
        || value.chars().count() > max
        || value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
        || credential_value(value)
    {
        return Err(invalid(format!(
            "{field} is empty, oversized, or contains credentials"
        )));
    }
    Ok(value.to_owned())
}

/// Validate nested JSON without confusing provider data with HTTP control parameters.
pub(crate) fn json(value: &Value, field: &str) -> Result<String, PublicError> {
    let wrapper = Map::from_iter([("value".to_owned(), value.clone())]);
    encode_nonempty_object(&wrapper, field)?;
    reject_excluded_channel(value)?;
    serde_json::to_string(value).map_err(|_| invalid(format!("{field} must be JSON")))
}

/// Only documented top-level parameters may reach a new endpoint. Nested objects
/// still use the existing credential and shape checks (some contain legitimate
/// fields such as a rule's execution_spec.execution_options).
pub(crate) fn form_fields(
    fields: &Map<String, Value>,
    allowed: &[&str],
) -> Result<Params, PublicError> {
    if fields.is_empty() {
        return Ok(Vec::new());
    }
    encode_nonempty_object(fields, "fields")?;
    for (key, value) in fields {
        if crate::mutation_plan::whatsapp_text(key) {
            return Err(invalid(
                "This server does not support the requested messaging channel",
            ));
        }
        reject_excluded_channel(value)?;
    }
    fields
        .iter()
        .map(|(key, value)| {
            if !allowed.contains(&key.as_str()) || request_control_key(key) || value.is_null() {
                return Err(invalid(format!("Unsupported or empty parameter: {key}")));
            }
            let encoded = match value {
                Value::String(value) => value.clone(),
                _ => serde_json::to_string(value)
                    .map_err(|_| invalid("Parameters must be JSON-compatible"))?,
            };
            Ok((key.clone(), encoded))
        })
        .collect()
}

pub(crate) fn read_params(options: &ReadOptions, defaults: &str) -> Result<Params, PublicError> {
    let fields = match &options.fields {
        Some(fields) => {
            if fields.is_empty()
                || fields.len() > 30
                || fields.iter().any(|field| {
                    field.is_empty()
                        || field.len() > 64
                        || credential_key(field)
                        || !field
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
                })
            {
                return Err(invalid(
                    "Use 1–30 simple field names without credentials or nested edges",
                ));
            }
            let mut fields = fields.clone();
            fields.sort_unstable();
            fields.dedup();
            fields.join(",")
        }
        None => defaults.to_owned(),
    };
    let size = options.page_size.unwrap_or(25);
    if !(1..=100).contains(&size) {
        return Err(invalid("page_size must be between 1 and 100"));
    }
    let mut params = vec![("limit".to_owned(), size.to_string())];
    if !fields.is_empty() {
        params.push(("fields".to_owned(), fields));
    }
    if let Some(cursor) = &options.page_cursor {
        if cursor.is_empty() || cursor.len() > 2048 || cursor.chars().any(char::is_control) {
            return Err(invalid(
                "page_cursor must be the bounded cursor from the preceding result",
            ));
        }
        params.push(("after".to_owned(), cursor.clone()));
    }
    Ok(params)
}

pub(crate) async fn read(
    graph: &GraphClient,
    endpoint: &str,
    params: Params,
) -> ToolResponse<GraphData> {
    let page_size = params
        .iter()
        .find(|(key, _)| key == "limit")
        .and_then(|(_, size)| size.parse::<usize>().ok())
        .unwrap_or(100);
    let result = match graph.get_json(endpoint, &params).await {
        Ok(value) => normalize(value, page_size),
        Err(error) => Err(error.into()),
    };
    response(result)
}

pub(crate) async fn write(
    graph: &GraphClient,
    endpoint: &str,
    params: Params,
) -> ToolResponse<GraphData> {
    let result = graph
        .post_form_json(endpoint, &params)
        .await
        .map_err(|error| mutation_error_without_blind_retry(error, VERIFY_WRITE))
        .and_then(normalize_write);
    response(result)
}

pub(crate) async fn delete(
    graph: &GraphClient,
    endpoint: &str,
    params: Params,
) -> ToolResponse<GraphData> {
    let result = graph
        .delete_json(endpoint, &params)
        .await
        .map_err(|error| mutation_error_without_blind_retry(error, VERIFY_WRITE))
        .and_then(normalize_write);
    response(result)
}

pub(crate) fn response<T>(result: Result<T, PublicError>) -> ToolResponse<T> {
    match result {
        Ok(data) => ToolResponse::success(data),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) fn normalize_write(value: Value) -> Result<GraphData, PublicError> {
    if !(value == Value::Bool(true) || value.as_object().is_some_and(|object| !object.is_empty()))
        || value.get("success") == Some(&Value::Bool(false))
    {
        return Err(ambiguous_mutation_result(
            "Meta did not confirm the change",
            VERIFY_WRITE,
        ));
    }
    normalize(value, 100).map_err(|_| {
        ambiguous_mutation_result(
            "Meta's write response could not be safely returned",
            VERIFY_WRITE,
        )
    })
}

fn normalize(mut value: Value, page_size: usize) -> Result<GraphData, PublicError> {
    let next_cursor = value
        .pointer("/paging/cursors/after")
        .and_then(Value::as_str)
        .filter(|cursor| !cursor.is_empty())
        .map(str::to_owned);
    if next_cursor.as_ref().is_some_and(|cursor| {
        cursor.len() > 2048
            || cursor.chars().any(char::is_control)
            || sensitive_output_value(cursor)
    }) {
        return Err(PublicError::invalid_upstream(
            "Meta returned an invalid cursor",
        ));
    }
    let mut result = match value.as_object_mut() {
        Some(object) => {
            object.remove("paging");
            if object.get("data").is_some_and(Value::is_array) {
                object.remove("data").expect("data array exists")
            } else {
                value
            }
        }
        None => value,
    };
    if result
        .as_array()
        .is_some_and(|rows| rows.len() > page_size.min(100))
    {
        return Err(PublicError::invalid_upstream(
            "Meta returned more rows than requested",
        ));
    }
    sanitize(&mut result, 0)?;
    if serde_json::to_vec(&result).map_or(true, |json| json.len() > MAX_OUTPUT_BYTES) {
        return Err(PublicError {
            code: "RESPONSE_TOO_LARGE".to_owned(),
            message: "Selected Meta data exceeds the 64 KiB tool output limit".to_owned(),
            retryable: false,
            action: Some("Request fewer fields or a smaller page".to_owned()),
        });
    }
    Ok(GraphData {
        result,
        next_cursor,
    })
}

fn sanitize(value: &mut Value, depth: usize) -> Result<(), PublicError> {
    if depth > 12 {
        return Err(PublicError::invalid_upstream(
            "Meta returned excessively nested data",
        ));
    }
    match value {
        Value::Object(object) => {
            object.retain(|key, _| !credential_key(key) && key != "paging");
            for child in object.values_mut() {
                sanitize(child, depth + 1)?;
            }
        }
        Value::Array(items) => {
            for child in items {
                sanitize(child, depth + 1)?;
            }
        }
        Value::String(text) if sensitive_output_value(text) => *text = "[REDACTED]".to_owned(),
        _ => {}
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> PublicError {
    PublicError::invalid_input(
        message,
        "Use the documented fields and keep credentials in the server environment",
    )
}

fn sensitive_output_value(text: &str) -> bool {
    credential_value(text)
        || (text.len() >= 40
            && text.starts_with("EAA")
            && text.bytes().all(|byte| byte.is_ascii_alphanumeric()))
}

// Called only after the shared JSON depth/size limits have been checked.
fn reject_excluded_channel(value: &Value) -> Result<(), PublicError> {
    let excluded = match value {
        Value::Object(object) => {
            for (key, child) in object {
                if crate::mutation_plan::whatsapp_text(key) {
                    return Err(invalid(
                        "This server does not support the requested messaging channel",
                    ));
                }
                reject_excluded_channel(child)?;
            }
            false
        }
        Value::Array(items) => {
            for child in items {
                reject_excluded_channel(child)?;
            }
            false
        }
        Value::String(text) => crate::mutation_plan::whatsapp_text(text),
        _ => false,
    };
    if excluded {
        Err(invalid(
            "This server does not support the requested messaging channel",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn new_graph_families_share_input_and_output_security_boundaries() {
        assert!(validate_threads_placements(&json!({"publisher_platforms":["threads"]})).is_err());
        assert!(validate_threads_placements(&json!({"publisher_platforms":["threads","instagram"],"instagram_positions":["stream"]})).is_ok());
        assert!(
            validate_threads_placements(
                &json!({"publisher_platforms":["instagram"],"instagram_positions":["reels"]})
            )
            .is_ok()
        );
        assert!(id("12/leadgen_forms", "id").is_err());
        assert_eq!(account("12").unwrap(), "act_12");
        for field in [
            "access_token",
            "name,access_token",
            "data{access_token}",
            "private_key",
        ] {
            assert!(
                read_params(
                    &ReadOptions {
                        fields: Some(vec![field.into()]),
                        ..Default::default()
                    },
                    "id"
                )
                .is_err()
            );
        }
        let nested = json!({"execution_spec":{"execution_options":["PAUSE"]}});
        assert!(form_fields(nested.as_object().unwrap(), &["execution_spec"]).is_ok());
        assert!(
            form_fields(
                json!({"spec":"{\"password\":\"private\"}"})
                    .as_object()
                    .unwrap(),
                &["spec"]
            )
            .is_err()
        );
        assert!(
            form_fields(
                json!({"spec":{"publisher_platforms":["whatsapp"]}})
                    .as_object()
                    .unwrap(),
                &["spec"]
            )
            .is_err()
        );
        for input in [json!({"method":"DELETE"}), json!({"name":{"password":"x"}})] {
            assert!(form_fields(input.as_object().unwrap(), &["method", "name"]).is_err());
        }
        let page = normalize(json!({"data":[{"id":"1", "access_token":"x", "url":"https://example.test/?token=x"}], "paging":{"next":"https://example.test/?access_token=x", "cursors":{"after":"opaque"}}}), 1).unwrap();
        assert_eq!(page.result, json!([{"id":"1","url":"[REDACTED]"}]));
        assert_eq!(page.next_cursor.as_deref(), Some("opaque"));
        assert!(normalize(json!({"data":[{},{}]}), 1).is_err());
        let err = normalize_write(json!({"success":false})).unwrap_err();
        assert!(!err.retryable);
        assert_eq!(err.code, "AMBIGUOUS_MUTATION_RESULT");
        for value in [json!([]), json!("unexpected"), json!(2), Value::Null] {
            assert_eq!(
                normalize_write(value).unwrap_err().code,
                "AMBIGUOUS_MUTATION_RESULT"
            );
        }
        assert!(
            normalize(
                json!({"data":[],"paging":{"cursors":{"after":"Bearer private"}}}),
                25
            )
            .is_err()
        );
    }
}
