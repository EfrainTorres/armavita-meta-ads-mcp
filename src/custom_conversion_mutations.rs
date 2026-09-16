// Copyright (C) 2025 ArmaVita LLC
// SPDX-License-Identifier: AGPL-3.0-only

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    bounded_json::encode_nonempty_object,
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{ad_account, numeric, numeric_value as normalize_numeric_value},
    mutation_result::{ambiguous_mutation_result, mutation_error_without_blind_retry},
    node_identity::{MetaNodeKind, verify_meta_node},
    safety::validate_removal_acknowledgement,
};

const MAX_NAME_CHARS: usize = 100;
const MAX_DESCRIPTION_CHARS: usize = 1_024;
const MAX_CONVERSION_VALUE: f64 = 1_000_000_000_000.0;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCustomConversionInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    pub ad_account_id: String,
    /// Conversion name, from 1 through 100 characters.
    pub name: String,
    /// Numeric Pixel or dataset ID that emits the source events.
    pub event_source_id: String,
    /// Nonempty Meta custom-conversion rule object, encoded under a 16 KiB safety cap.
    pub rule: Map<String, Value>,
    /// Standard event used for reporting and optimization.
    pub custom_event_type: CustomConversionEventType,
    /// Optional description, up to 1,024 characters.
    pub description: Option<String>,
    /// Optional current Meta advanced-rule object, encoded under a 16 KiB safety cap.
    pub advanced_rule: Option<Map<String, Value>>,
    /// Optional source channel for the event.
    pub action_source_type: Option<CustomConversionActionSource>,
    /// Fixed value for events without a value. Must be finite and nonnegative.
    pub default_conversion_value: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateCustomConversionInput {
    /// Numeric Meta ad-account ID that owns the conversion, with or without `act_`.
    #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
    pub ad_account_id: String,
    /// Numeric Meta custom-conversion ID.
    pub custom_conversion_id: String,
    /// New nonempty name, up to 100 characters.
    pub name: Option<String>,
    /// New description, up to 1,024 characters. An empty string clears it.
    pub description: Option<String>,
    /// New fixed value for events without a value. Must be finite and nonnegative.
    pub default_conversion_value: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteCustomConversionInput {
    /// Numeric Meta ad-account ID that owns the conversion, with or without `act_`.
    #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
    pub ad_account_id: String,
    /// Numeric Meta custom-conversion ID to archive/delete.
    pub custom_conversion_id: String,
    /// Exact destructive-action acknowledgement required after operator approval.
    #[schemars(
        length(min = 25, max = 25),
        regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
    )]
    pub removal_acknowledgement: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
pub enum CustomConversionEventType {
    #[serde(rename = "ADD_PAYMENT_INFO")]
    AddPaymentInfo,
    #[serde(rename = "ADD_TO_CART")]
    AddToCart,
    #[serde(rename = "ADD_TO_WISHLIST")]
    AddToWishlist,
    #[serde(rename = "COMPLETE_REGISTRATION")]
    CompleteRegistration,
    #[serde(rename = "CONTACT")]
    Contact,
    #[serde(rename = "CONTENT_VIEW")]
    ContentView,
    #[serde(rename = "CUSTOMIZE_PRODUCT")]
    CustomizeProduct,
    #[serde(rename = "DONATE")]
    Donate,
    #[serde(rename = "FACEBOOK_SELECTED")]
    FacebookSelected,
    #[serde(rename = "FIND_LOCATION")]
    FindLocation,
    #[serde(rename = "INITIATED_CHECKOUT")]
    InitiatedCheckout,
    #[serde(rename = "LEAD")]
    Lead,
    #[serde(rename = "LISTING_INTERACTION")]
    ListingInteraction,
    #[serde(rename = "OTHER")]
    Other,
    #[serde(rename = "PURCHASE")]
    Purchase,
    #[serde(rename = "SCHEDULE")]
    Schedule,
    #[serde(rename = "SEARCH")]
    Search,
    #[serde(rename = "START_TRIAL")]
    StartTrial,
    #[serde(rename = "SUBMIT_APPLICATION")]
    SubmitApplication,
    #[serde(rename = "SUBSCRIBE")]
    Subscribe,
}

impl CustomConversionEventType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::AddPaymentInfo => "ADD_PAYMENT_INFO",
            Self::AddToCart => "ADD_TO_CART",
            Self::AddToWishlist => "ADD_TO_WISHLIST",
            Self::CompleteRegistration => "COMPLETE_REGISTRATION",
            Self::Contact => "CONTACT",
            Self::ContentView => "CONTENT_VIEW",
            Self::CustomizeProduct => "CUSTOMIZE_PRODUCT",
            Self::Donate => "DONATE",
            Self::FacebookSelected => "FACEBOOK_SELECTED",
            Self::FindLocation => "FIND_LOCATION",
            Self::InitiatedCheckout => "INITIATED_CHECKOUT",
            Self::Lead => "LEAD",
            Self::ListingInteraction => "LISTING_INTERACTION",
            Self::Other => "OTHER",
            Self::Purchase => "PURCHASE",
            Self::Schedule => "SCHEDULE",
            Self::Search => "SEARCH",
            Self::StartTrial => "START_TRIAL",
            Self::SubmitApplication => "SUBMIT_APPLICATION",
            Self::Subscribe => "SUBSCRIBE",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CustomConversionActionSource {
    App,
    BusinessMessaging,
    Chat,
    Email,
    Other,
    PhoneCall,
    PhysicalStore,
    SystemGenerated,
    Website,
}

impl CustomConversionActionSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::BusinessMessaging => "business_messaging",
            Self::Chat => "chat",
            Self::Email => "email",
            Self::Other => "other",
            Self::PhoneCall => "phone_call",
            Self::PhysicalStore => "physical_store",
            Self::SystemGenerated => "system_generated",
            Self::Website => "website",
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CreatedCustomConversion {
    pub custom_conversion_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UpdatedCustomConversion {
    pub custom_conversion_id: String,
    pub updated: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeletedCustomConversion {
    pub custom_conversion_id: String,
    pub deleted: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct MutationRequest {
    endpoint: String,
    params: Vec<(String, String)>,
}

pub(crate) async fn create_custom_conversion(
    graph: &GraphClient,
    input: CreateCustomConversionInput,
) -> ToolResponse<CreatedCustomConversion> {
    let request = match build_create_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph
        .post_form_json(&request.endpoint, &request.params)
        .await
    {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(mutation_error_without_blind_retry(
                error,
                "Check Events Manager for the custom conversion before trying again",
            ));
        }
    };
    match created_id(&payload) {
        Ok(custom_conversion_id) => ToolResponse::success(CreatedCustomConversion {
            custom_conversion_id,
        }),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn update_custom_conversion(
    graph: &GraphClient,
    input: UpdateCustomConversionInput,
) -> ToolResponse<UpdatedCustomConversion> {
    let request = match build_update_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let custom_conversion_id = request.endpoint.clone();
    if let Err(error) = verify_meta_node(
        graph,
        &custom_conversion_id,
        MetaNodeKind::CustomConversion,
        Some(&input.ad_account_id),
    )
    .await
    {
        return ToolResponse::error(error);
    }
    let payload = match graph
        .post_form_json(&request.endpoint, &request.params)
        .await
    {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    if confirmed(&payload, &custom_conversion_id) {
        ToolResponse::success(UpdatedCustomConversion {
            custom_conversion_id,
            updated: true,
        })
    } else {
        ToolResponse::error(ambiguous_result(
            "Meta did not confirm the custom-conversion update",
            false,
        ))
    }
}

pub(crate) async fn delete_custom_conversion(
    graph: &GraphClient,
    input: DeleteCustomConversionInput,
) -> ToolResponse<DeletedCustomConversion> {
    let request = match build_delete_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let custom_conversion_id = request.endpoint.clone();
    if let Err(error) = verify_meta_node(
        graph,
        &custom_conversion_id,
        MetaNodeKind::CustomConversion,
        Some(&input.ad_account_id),
    )
    .await
    {
        return ToolResponse::error(error);
    }
    let payload = match graph.delete_json(&request.endpoint, &request.params).await {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(mutation_error_without_blind_retry(
                error,
                "Check Events Manager before attempting another delete",
            ));
        }
    };
    if confirmed(&payload, &custom_conversion_id) {
        ToolResponse::success(DeletedCustomConversion {
            custom_conversion_id,
            deleted: true,
        })
    } else {
        ToolResponse::error(ambiguous_result(
            "Meta did not confirm the custom-conversion deletion",
            true,
        ))
    }
}

fn build_create_request(
    input: &CreateCustomConversionInput,
) -> Result<MutationRequest, PublicError> {
    let account_id = normalize_account_id(&input.ad_account_id)?;
    let name = required_text(&input.name, MAX_NAME_CHARS, "name")?;
    let event_source_id = numeric_id(&input.event_source_id, "event_source_id")?;
    let rule = encode_nonempty_object(&input.rule, "rule")?;
    let mut params = vec![
        ("name".to_owned(), name),
        ("event_source_id".to_owned(), event_source_id),
        ("rule".to_owned(), rule),
        (
            "custom_event_type".to_owned(),
            input.custom_event_type.as_str().to_owned(),
        ),
    ];
    if let Some(description) = &input.description {
        params.push((
            "description".to_owned(),
            required_text(description, MAX_DESCRIPTION_CHARS, "description")?,
        ));
    }
    if let Some(advanced_rule) = &input.advanced_rule {
        params.push((
            "advanced_rule".to_owned(),
            encode_nonempty_object(advanced_rule, "advanced_rule")?,
        ));
    }
    if let Some(action_source_type) = input.action_source_type {
        params.push((
            "action_source_type".to_owned(),
            action_source_type.as_str().to_owned(),
        ));
    }
    if let Some(value) = input.default_conversion_value {
        params.push((
            "default_conversion_value".to_owned(),
            conversion_value(value)?,
        ));
    }
    Ok(MutationRequest {
        endpoint: format!("{account_id}/customconversions"),
        params,
    })
}

fn build_update_request(
    input: &UpdateCustomConversionInput,
) -> Result<MutationRequest, PublicError> {
    let custom_conversion_id = numeric_id(&input.custom_conversion_id, "custom_conversion_id")?;
    let mut params = Vec::with_capacity(3);
    if let Some(name) = &input.name {
        params.push((
            "name".to_owned(),
            required_text(name, MAX_NAME_CHARS, "name")?,
        ));
    }
    if let Some(description) = &input.description {
        params.push((
            "description".to_owned(),
            bounded_text(description, MAX_DESCRIPTION_CHARS, "description")?,
        ));
    }
    if let Some(value) = input.default_conversion_value {
        params.push((
            "default_conversion_value".to_owned(),
            conversion_value(value)?,
        ));
    }
    if params.is_empty() {
        return Err(PublicError::invalid_input(
            "at least one writable custom-conversion field is required",
            "Provide name, description, and/or default_conversion_value",
        ));
    }
    Ok(MutationRequest {
        endpoint: custom_conversion_id,
        params,
    })
}

fn build_delete_request(
    input: &DeleteCustomConversionInput,
) -> Result<MutationRequest, PublicError> {
    validate_removal_acknowledgement(true, Some(&input.removal_acknowledgement))?;
    Ok(MutationRequest {
        endpoint: numeric_id(&input.custom_conversion_id, "custom_conversion_id")?,
        params: Vec::new(),
    })
}

fn normalize_account_id(raw: &str) -> Result<String, PublicError> {
    ad_account(raw).ok_or_else(|| {
        PublicError::invalid_input(
            "ad_account_id must be a numeric Meta account ID",
            "Use an ID such as `act_123456789` or `123456789`",
        )
    })
}

fn numeric_id(raw: &str, field: &str) -> Result<String, PublicError> {
    numeric(raw).map(str::to_owned).ok_or_else(|| {
        PublicError::invalid_input(
            format!("{field} must be a numeric Meta ID"),
            "Use the numeric ID returned by Meta",
        )
    })
}

fn required_text(raw: &str, maximum: usize, field: &str) -> Result<String, PublicError> {
    let value = bounded_text(raw, maximum, field)?;
    if value.is_empty() {
        return Err(PublicError::invalid_input(
            format!("{field} cannot be empty"),
            format!("Provide {field} using no more than {maximum} characters"),
        ));
    }
    Ok(value)
}

fn bounded_text(raw: &str, maximum: usize, field: &str) -> Result<String, PublicError> {
    let value = raw.trim();
    if value.chars().count() > maximum || value.chars().any(char::is_control) {
        return Err(PublicError::invalid_input(
            format!("{field} is invalid or too long"),
            format!("Use no more than {maximum} printable characters"),
        ));
    }
    Ok(value.to_owned())
}

fn conversion_value(value: f64) -> Result<String, PublicError> {
    if !value.is_finite() || !(0.0..=MAX_CONVERSION_VALUE).contains(&value) {
        return Err(PublicError::invalid_input(
            "default_conversion_value must be finite and nonnegative",
            "Use a value from 0 through 1,000,000,000,000",
        ));
    }
    Ok(value.to_string())
}

fn created_id(payload: &Value) -> Result<String, PublicError> {
    payload
        .get("id")
        .and_then(normalize_numeric_value)
        .ok_or_else(|| ambiguous_result("Meta did not confirm the custom-conversion ID", false))
}

fn confirmed(payload: &Value, expected_id: &str) -> bool {
    payload.get("success").and_then(Value::as_bool) == Some(true)
        || payload
            .get("id")
            .and_then(normalize_numeric_value)
            .is_some_and(|id| id == expected_id)
}

fn ambiguous_result(message: impl Into<String>, destructive: bool) -> PublicError {
    let action = if destructive {
        "Verify the conversion in Events Manager before attempting another delete"
    } else {
        "Verify the conversion in Events Manager before retrying"
    };
    ambiguous_mutation_result(message, action)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rmcp::schemars::schema_for;
    use serde_json::{Map, json};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    use crate::{
        config::MetaConfig, error::ToolResponse, graph::GraphClient,
        safety::REMOVAL_ACKNOWLEDGEMENT,
    };

    use super::{
        CreateCustomConversionInput, CustomConversionActionSource, CustomConversionEventType,
        DeleteCustomConversionInput, UpdateCustomConversionInput, build_create_request,
        build_delete_request, build_update_request, confirmed, created_id,
        delete_custom_conversion,
    };

    fn object(value: serde_json::Value) -> Map<String, serde_json::Value> {
        value.as_object().unwrap().clone()
    }

    async fn scripted_graph(
        responses: Vec<&'static str>,
    ) -> (GraphClient, tokio::task::JoinHandle<Vec<Vec<u8>>>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut requests = Vec::with_capacity(responses.len());
            for body in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                requests.push(read_http_request(&mut socket).await);
                write_json(&mut socket, body).await;
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(200), listener.accept())
                    .await
                    .is_err(),
                "custom-conversion delete emitted an unexpected request"
            );
            requests
        });
        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        ))
        .unwrap();
        (graph, server)
    }

    async fn read_http_request(socket: &mut TcpStream) -> Vec<u8> {
        let mut request = Vec::with_capacity(2 * 1024);
        let mut buffer = [0_u8; 1024];
        loop {
            let bytes_read = socket.read(&mut buffer).await.unwrap();
            if bytes_read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..bytes_read]);
            let Some(headers_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let body_start = headers_end + 4;
            let headers = String::from_utf8_lossy(&request[..headers_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or_default();
            if request.len() >= body_start + content_length {
                break;
            }
        }
        request
    }

    async fn write_json(socket: &mut TcpStream, body: &str) {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    }

    #[test]
    fn direct_conversion_schemas_require_a_bounded_account_id() {
        for schema in [
            serde_json::to_value(schema_for!(UpdateCustomConversionInput)).unwrap(),
            serde_json::to_value(schema_for!(DeleteCustomConversionInput)).unwrap(),
        ] {
            assert!(
                schema["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("ad_account_id"))
            );
            let account = &schema["properties"]["ad_account_id"];
            assert_eq!(account["minLength"], 1);
            assert_eq!(account["maxLength"], 68);
            assert_eq!(account["pattern"], "^(act_)?[0-9]{1,64}$");
        }
    }

    #[tokio::test]
    async fn wrong_type_or_account_blocks_custom_conversion_delete() {
        for identity in [
            r#"{"id":"789","account_id":"123","objective":"OUTCOME_SALES"}"#,
            r#"{"id":"789","account_id":"999","custom_event_type":"PURCHASE"}"#,
        ] {
            let (graph, server) = scripted_graph(vec![identity]).await;
            assert!(matches!(
                delete_custom_conversion(
                    &graph,
                    DeleteCustomConversionInput {
                        ad_account_id: "123".to_owned(),
                        custom_conversion_id: "789".to_owned(),
                        removal_acknowledgement: REMOVAL_ACKNOWLEDGEMENT.to_owned(),
                    },
                )
                .await,
                ToolResponse::Error { .. }
            ));

            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 1, "identity mismatch reached DELETE");
            assert!(String::from_utf8(requests[0].clone()).unwrap().starts_with(
                "GET /789?fields=id%2Cname%2Caccount_id%2Ccustom_event_type HTTP/1.1\r\n"
            ));
        }
    }

    #[tokio::test]
    async fn valid_identity_precedes_exactly_one_custom_conversion_delete() {
        let (graph, server) = scripted_graph(vec![
            r#"{"id":"789","name":"Purchase","account_id":"123","custom_event_type":"PURCHASE"}"#,
            r#"{"success":true}"#,
        ])
        .await;
        assert!(matches!(
            delete_custom_conversion(
                &graph,
                DeleteCustomConversionInput {
                    ad_account_id: "act_123".to_owned(),
                    custom_conversion_id: "789".to_owned(),
                    removal_acknowledgement: REMOVAL_ACKNOWLEDGEMENT.to_owned(),
                },
            )
            .await,
            ToolResponse::Success { .. }
        ));

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(String::from_utf8(requests[0].clone()).unwrap().starts_with(
            "GET /789?fields=id%2Cname%2Caccount_id%2Ccustom_event_type HTTP/1.1\r\n"
        ));
        assert_eq!(
            String::from_utf8(requests[1].clone())
                .unwrap()
                .lines()
                .next(),
            Some("DELETE /789 HTTP/1.1")
        );
    }

    #[test]
    fn builds_exact_bounded_create_form() {
        let request = build_create_request(&CreateCustomConversionInput {
            ad_account_id: "123".to_owned(),
            name: "Qualified purchase".to_owned(),
            event_source_id: "456".to_owned(),
            rule: object(json!({"event": {"eq": "Purchase"}})),
            custom_event_type: CustomConversionEventType::Purchase,
            description: Some("High-value orders".to_owned()),
            advanced_rule: None,
            action_source_type: Some(CustomConversionActionSource::Website),
            default_conversion_value: Some(25.5),
        })
        .unwrap();
        assert_eq!(request.endpoint, "act_123/customconversions");
        assert_eq!(
            request.params,
            vec![
                ("name".to_owned(), "Qualified purchase".to_owned()),
                ("event_source_id".to_owned(), "456".to_owned()),
                (
                    "rule".to_owned(),
                    "{\"event\":{\"eq\":\"Purchase\"}}".to_owned(),
                ),
                ("custom_event_type".to_owned(), "PURCHASE".to_owned()),
                ("description".to_owned(), "High-value orders".to_owned()),
                ("action_source_type".to_owned(), "website".to_owned()),
                ("default_conversion_value".to_owned(), "25.5".to_owned()),
            ]
        );
    }

    #[test]
    fn update_exposes_only_current_writable_fields() {
        let request = build_update_request(&UpdateCustomConversionInput {
            ad_account_id: "123".to_owned(),
            custom_conversion_id: "789".to_owned(),
            name: Some("Renamed".to_owned()),
            description: Some(String::new()),
            default_conversion_value: Some(0.0),
        })
        .unwrap();
        assert_eq!(request.endpoint, "789");
        assert_eq!(request.params.len(), 3);
        assert_eq!(request.params[1], ("description".to_owned(), String::new()));

        assert!(
            build_update_request(&UpdateCustomConversionInput {
                ad_account_id: "123".to_owned(),
                custom_conversion_id: "789".to_owned(),
                name: None,
                description: None,
                default_conversion_value: None,
            })
            .is_err()
        );
    }

    #[test]
    fn rejects_unsafe_rules_values_and_ids() {
        let mut input = CreateCustomConversionInput {
            ad_account_id: "act_123".to_owned(),
            name: "Purchase".to_owned(),
            event_source_id: "456".to_owned(),
            rule: object(json!({"access_token": "secret"})),
            custom_event_type: CustomConversionEventType::Purchase,
            description: None,
            advanced_rule: None,
            action_source_type: None,
            default_conversion_value: None,
        };
        assert!(build_create_request(&input).is_err());
        input.rule = object(json!({"event": "Purchase"}));
        input.default_conversion_value = Some(-1.0);
        assert!(build_create_request(&input).is_err());
        input.default_conversion_value = None;
        input.event_source_id = "../456".to_owned();
        assert!(build_create_request(&input).is_err());
    }

    #[test]
    fn validates_delete_and_compact_confirmations() {
        let request = build_delete_request(&DeleteCustomConversionInput {
            ad_account_id: "123".to_owned(),
            custom_conversion_id: "789".to_owned(),
            removal_acknowledgement: REMOVAL_ACKNOWLEDGEMENT.to_owned(),
        })
        .unwrap();
        assert_eq!(request.endpoint, "789");
        assert!(request.params.is_empty());
        assert_eq!(created_id(&json!({"id": "789"})).unwrap(), "789");
        assert!(confirmed(&json!({"success": true}), "789"));
        assert!(confirmed(&json!({"id": 789}), "789"));
        assert!(!confirmed(&json!({"success": false}), "789"));
        assert!(
            build_delete_request(&DeleteCustomConversionInput {
                ad_account_id: "123".to_owned(),
                custom_conversion_id: "789".to_owned(),
                removal_acknowledgement: "yes".to_owned(),
            })
            .is_err()
        );
    }
}
