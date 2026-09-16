use reqwest::Url;
use serde_json::{Map, Value};

use crate::error::PublicError;

const MAX_BYTES: usize = 16 * 1024;
const MAX_DEPTH: usize = 8;
const MAX_NODES: usize = 1_024;
const MAX_KEYS: usize = 128;
const MAX_ARRAY_ITEMS: usize = 256;
const MAX_KEY_CHARS: usize = 64;
pub(crate) const MAX_STRING_CHARS: usize = 2_048;

/// Encode one nonempty provider object after enforcing common memory and secret boundaries.
pub(crate) fn encode_nonempty_object(
    object: &Map<String, Value>,
    field_name: &str,
) -> Result<String, PublicError> {
    if object.is_empty() {
        return Err(PublicError::invalid_input(
            format!("{field_name} cannot be empty"),
            "Omit the field or provide a nonempty object",
        ));
    }
    validate_object_tree(object, field_name)?;
    let encoded = serde_json::to_string(object).map_err(|_| {
        PublicError::invalid_input(
            format!("{field_name} could not be encoded"),
            "Use JSON-compatible values",
        )
    })?;
    if encoded.len() > MAX_BYTES {
        return Err(PublicError::invalid_input(
            format!("{field_name} exceeds the 16 KiB safety limit"),
            "Remove unused or oversized values",
        ));
    }
    Ok(encoded)
}

fn validate_object_tree(object: &Map<String, Value>, field_name: &str) -> Result<(), PublicError> {
    if object.len() > MAX_KEYS {
        return Err(invalid_shape(field_name));
    }
    let mut stack = Vec::with_capacity(object.len());
    for (key, child) in object {
        if !valid_key(key) {
            return Err(PublicError::invalid_input(
                format!("{field_name} contains an invalid or credential-like key"),
                "Use current provider field names and never include credentials",
            ));
        }
        stack.push((child, 2_usize));
    }
    let mut nodes = 1_usize;
    let mut keys = object.len();
    while let Some((value, depth)) = stack.pop() {
        nodes = nodes.saturating_add(1);
        if depth > MAX_DEPTH || nodes > MAX_NODES {
            return Err(invalid_shape(field_name));
        }
        match value {
            Value::Object(object) => {
                keys = keys.saturating_add(object.len());
                if object.len() > MAX_KEYS || keys > MAX_KEYS {
                    return Err(invalid_shape(field_name));
                }
                for (key, child) in object {
                    if !valid_key(key) {
                        return Err(PublicError::invalid_input(
                            format!("{field_name} contains an invalid or credential-like key"),
                            "Use current provider field names and never include credentials",
                        ));
                    }
                    stack.push((child, depth + 1));
                }
            }
            Value::Array(items) => {
                if items.len() > MAX_ARRAY_ITEMS {
                    return Err(invalid_shape(field_name));
                }
                stack.extend(items.iter().map(|item| (item, depth + 1)));
            }
            Value::String(text) => {
                if text.chars().count() > MAX_STRING_CHARS {
                    return Err(invalid_shape(field_name));
                }
                if credential_value(text) {
                    return Err(PublicError::invalid_input(
                        format!("{field_name} contains a credential-like value"),
                        "Keep credentials outside tool arguments",
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.chars().count() <= MAX_KEY_CHARS
        && !credential_key(key)
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

pub(crate) fn request_control_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().replace(['-', '.'], "_").as_str(),
        "method"
            | "http_method"
            | "_method"
            | "batch"
            | "relative_url"
            | "attached_files"
            | "depends_on"
            | "omit_response_on_success"
            | "execution_options"
            | "suppress_http_code"
    )
}

pub(crate) fn credential_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', '.'], "_");
    matches!(
        normalized.as_str(),
        "access_token"
            | "token"
            | "authorization"
            | "app_secret"
            | "appsecret_proof"
            | "client_secret"
            | "password"
            | "api_key"
            | "cookie"
            | "credential"
            | "credentials"
            | "secret"
            | "secret_key"
            | "private_key"
            | "passwd"
    ) || normalized.ends_with("_access_token")
        || normalized.ends_with("_client_secret")
        || normalized.ends_with("_password")
        || normalized.ends_with("_api_key")
        || normalized.ends_with("_secret")
        || normalized.ends_with("_token")
}

pub(crate) fn credential_value(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    if (lower.starts_with('{') || lower.starts_with('['))
        && serde_json::from_str::<Value>(text)
            .ok()
            .is_some_and(|value| json_contains_credentials(&value))
    {
        return true;
    }
    [
        "access_token=",
        "access_token\":",
        "app_secret=",
        "appsecret_proof=",
        "authorization:",
        "client_secret=",
        "password=",
        "api_key=",
        "secret=",
        "token=",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
        || lower.contains("bearer ")
        || Url::parse(text.trim()).ok().is_some_and(|url| {
            !url.username().is_empty()
                || url.password().is_some()
                || url
                    .query_pairs()
                    .any(|(key, _)| credential_key(key.as_ref()))
                || url.fragment().is_some_and(|fragment| {
                    let mut fragment_query = url.clone();
                    fragment_query.set_query(Some(fragment));
                    fragment_query
                        .query_pairs()
                        .any(|(key, _)| credential_key(key.as_ref()))
                })
        })
}

fn json_contains_credentials(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields
            .iter()
            .any(|(key, value)| credential_key(key) || json_contains_credentials(value)),
        Value::Array(items) => items.iter().any(json_contains_credentials),
        Value::String(text) => credential_value(text),
        _ => false,
    }
}

fn invalid_shape(field_name: &str) -> PublicError {
    PublicError::invalid_input(
        format!("{field_name} is too large or deeply nested"),
        "Keep the object under 128 keys, 1,024 values, and 8 nesting levels",
    )
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, json};

    use super::encode_nonempty_object;

    #[test]
    fn encodes_compact_objects_and_rejects_nested_credentials() {
        let object = serde_json::from_value::<Map<String, serde_json::Value>>(json!({
            "and": [{"event": "Purchase"}]
        }))
        .unwrap();
        assert_eq!(
            encode_nonempty_object(&object, "rule").unwrap(),
            "{\"and\":[{\"event\":\"Purchase\"}]}"
        );

        let unsafe_object = serde_json::from_value(json!({
            "nested": {"provider_access_token": "secret"}
        }))
        .unwrap();
        assert!(encode_nonempty_object(&unsafe_object, "rule").is_err());

        let unsafe_value = serde_json::from_value(json!({
            "payload": "Bearer abcdefghijklmnopqrstuvwxyz"
        }))
        .unwrap();
        assert!(encode_nonempty_object(&unsafe_value, "rule").is_err());
        let unsafe_url = serde_json::from_value(json!({
            "url": "https://example.test/?access_token=secret"
        }))
        .unwrap();
        assert!(encode_nonempty_object(&unsafe_url, "rule").is_err());

        for object in [
            json!({"nested": {"refresh_token": "synthetic-secret"}}),
            json!({"nested": {"provider_secret": "synthetic-secret"}}),
            json!({"url": "https://user:synthetic-secret@example.test/path"}),
            json!({"url": "https://example.test/?%61ccess_token=synthetic-secret"}),
            json!({"url": "https://example.test/?provider_token=synthetic-secret"}),
            json!({"url": "https://example.test/#%61ccess_token=synthetic-secret"}),
            json!({"secret_key": "synthetic-secret"}),
            json!({"private_key": "synthetic-secret"}),
            json!({"passwd": "synthetic-secret"}),
        ] {
            assert!(encode_nonempty_object(object.as_object().unwrap(), "rule").is_err());
        }

        let opaque_id = serde_json::from_value(json!({
            "provider_id": format!("EAA{}", "x".repeat(256))
        }))
        .unwrap();
        assert!(encode_nonempty_object(&opaque_id, "rule").is_ok());
    }
}
