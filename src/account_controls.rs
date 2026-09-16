use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    bounded_json::encode_nonempty_object,
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{ad_account as normalize_account_id, numeric_owned},
    mutation_result::ambiguous_mutation_result,
};

const CONTROL_FIELDS: &str =
    "audience_controls,placement_controls,is_age_restriction_enabled,status,campaigns_with_error";
const MAX_CONTROL_RECORDS: usize = 10;
const MAX_CAMPAIGN_ERRORS: usize = 100;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetAccountControlsInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    pub ad_account_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateAccountControlsInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    pub ad_account_id: String,
    /// Account-wide audience controls. Omit to leave this control family unchanged.
    pub audience_controls: Option<Map<String, Value>>,
    /// Account-wide placement controls. Omit to leave this control family unchanged.
    pub placement_controls: Option<Map<String, Value>>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AccountControlsList {
    pub controls: Vec<AccountControls>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AccountControls {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience_controls: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placement_controls: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_age_restriction_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub campaigns_with_error: Option<Vec<String>>,
    #[serde(skip_serializing_if = "is_false")]
    pub campaigns_with_error_truncated: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AccountControlsUpdate {
    pub accepted: bool,
}

#[derive(Debug)]
struct ValidatedUpdate {
    endpoint: String,
    form: Vec<(String, String)>,
}

#[derive(Debug, Deserialize)]
struct RawControlsList {
    #[serde(default)]
    data: Vec<RawControls>,
}

#[derive(Debug, Deserialize)]
struct RawControls {
    audience_controls: Option<Value>,
    placement_controls: Option<Value>,
    is_age_restriction_enabled: Option<bool>,
    status: Option<String>,
    campaigns_with_error: Option<Vec<String>>,
}

pub(crate) async fn get_account_controls(
    graph: &GraphClient,
    input: GetAccountControlsInput,
) -> ToolResponse<AccountControlsList> {
    let Some(account_id) = normalize_account_id(&input.ad_account_id) else {
        return ToolResponse::error(PublicError::invalid_input(
            "ad_account_id must be a numeric Meta account ID",
            "Use an ID such as `act_123456789` or `123456789`",
        ));
    };
    let query = vec![("fields".to_owned(), CONTROL_FIELDS.to_owned())];
    let payload = match graph
        .get_json(&format!("{account_id}/account_controls"), &query)
        .await
    {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    let raw = match serde_json::from_value::<RawControlsList>(payload) {
        Ok(raw) if raw.data.len() <= MAX_CONTROL_RECORDS => raw,
        _ => {
            return ToolResponse::error(PublicError::invalid_upstream(
                "Meta returned unexpected account controls",
            ));
        }
    };

    let controls = raw
        .data
        .into_iter()
        .map(|raw| {
            let (campaigns_with_error, campaigns_with_error_truncated) =
                clean_campaign_ids(raw.campaigns_with_error);
            AccountControls {
                audience_controls: object_only(raw.audience_controls),
                placement_controls: object_only(raw.placement_controls),
                is_age_restriction_enabled: raw.is_age_restriction_enabled,
                status: clean_text(raw.status),
                campaigns_with_error,
                campaigns_with_error_truncated,
            }
        })
        .collect();
    ToolResponse::success(AccountControlsList { controls })
}

pub(crate) async fn update_account_controls(
    graph: &GraphClient,
    input: UpdateAccountControlsInput,
) -> ToolResponse<AccountControlsUpdate> {
    let request = match build_update_request(input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    if !accepted_update(&payload) {
        return ToolResponse::error(ambiguous_mutation_result(
            "Meta did not acknowledge the account-controls update",
            "Read the account controls before submitting the same update again",
        ));
    }
    ToolResponse::success(AccountControlsUpdate { accepted: true })
}

fn accepted_update(payload: &Value) -> bool {
    match payload {
        Value::Bool(true) => true,
        Value::Object(object) => object.get("success").and_then(Value::as_bool) == Some(true),
        _ => false,
    }
}

fn build_update_request(input: UpdateAccountControlsInput) -> Result<ValidatedUpdate, PublicError> {
    let account_id = normalize_account_id(&input.ad_account_id).ok_or_else(|| {
        PublicError::invalid_input(
            "ad_account_id must be a numeric Meta account ID",
            "Use an ID such as `act_123456789` or `123456789`",
        )
    })?;
    let mut form = Vec::with_capacity(2);
    if let Some(controls) = input.audience_controls {
        form.push((
            "audience_controls".to_owned(),
            encode_nonempty_object(&controls, "audience_controls")?,
        ));
    }
    if let Some(controls) = input.placement_controls {
        form.push((
            "placement_controls".to_owned(),
            encode_nonempty_object(&controls, "placement_controls")?,
        ));
    }
    if form.is_empty() {
        return Err(PublicError::invalid_input(
            "at least one account-control object is required",
            "Provide audience_controls and/or placement_controls",
        ));
    }
    Ok(ValidatedUpdate {
        endpoint: format!("{account_id}/account_controls"),
        form,
    })
}

fn object_only(value: Option<Value>) -> Option<Value> {
    value.filter(Value::is_object)
}

fn clean_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn clean_campaign_ids(values: Option<Vec<String>>) -> (Option<Vec<String>>, bool) {
    let Some(values) = values else {
        return (None, false);
    };
    let mut cleaned = Vec::with_capacity(values.len().min(MAX_CAMPAIGN_ERRORS));
    let mut truncated = false;
    for value in values {
        let Some(value) = numeric_owned(&value) else {
            continue;
        };
        if cleaned.len() == MAX_CAMPAIGN_ERRORS {
            truncated = true;
        } else {
            cleaned.push(value);
        }
    }
    ((!cleaned.is_empty()).then_some(cleaned), truncated)
}

const fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        MAX_CAMPAIGN_ERRORS, UpdateAccountControlsInput, accepted_update, build_update_request,
        clean_campaign_ids, normalize_account_id, object_only,
    };

    #[test]
    fn validates_account_and_normalizes_nested_controls() {
        assert_eq!(normalize_account_id("123"), Some("act_123".to_owned()));
        assert_eq!(normalize_account_id("act_123"), Some("act_123".to_owned()));
        assert_eq!(normalize_account_id("../123"), None);
        assert!(object_only(Some(json!({"age_min": 21}))).is_some());
        assert!(object_only(Some(json!(["unexpected"]))).is_none());
        assert_eq!(
            clean_campaign_ids(Some(vec![" 42 ".to_owned(), "bad/id".to_owned()])),
            (Some(vec!["42".to_owned()]), false)
        );
        let (_, truncated) = clean_campaign_ids(Some(
            (0..=MAX_CAMPAIGN_ERRORS).map(|id| id.to_string()).collect(),
        ));
        assert!(truncated);
    }

    #[test]
    fn builds_one_bounded_idempotent_controls_update() {
        let request = build_update_request(UpdateAccountControlsInput {
            ad_account_id: "123".to_owned(),
            audience_controls: Some(
                json!({"age_min": 21, "countries": ["US"]})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
            placement_controls: None,
        })
        .unwrap();
        assert_eq!(request.endpoint, "act_123/account_controls");
        assert_eq!(request.form.len(), 1);
        assert_eq!(request.form[0].0, "audience_controls");
        assert!(request.form[0].1.contains("age_min"));
    }

    #[test]
    fn rejects_empty_oversized_or_secret_bearing_controls() {
        let empty = UpdateAccountControlsInput {
            ad_account_id: "123".to_owned(),
            audience_controls: None,
            placement_controls: None,
        };
        assert!(build_update_request(empty).is_err());

        for controls in [
            json!({}),
            json!({"access_token": "never"}),
            json!({"allowed": "x".repeat(crate::bounded_json::MAX_STRING_CHARS + 1)}),
        ] {
            let input = UpdateAccountControlsInput {
                ad_account_id: "123".to_owned(),
                audience_controls: controls.as_object().cloned(),
                placement_controls: None,
            };
            assert!(build_update_request(input).is_err());
        }
        assert!(accepted_update(&json!({"success": true})));
        assert!(!accepted_update(&json!({"id": "123"})));
        assert!(!accepted_update(&json!({"success": false})));
        assert!(!accepted_update(&json!({})));
    }
}
