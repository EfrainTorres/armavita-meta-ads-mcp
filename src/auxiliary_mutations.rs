// Copyright (C) 2025 ArmaVita LLC
// SPDX-License-Identifier: AGPL-3.0-only

use std::time::{SystemTime, UNIX_EPOCH};

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{ad_account as normalize_ad_account_id, numeric_owned as numeric_id, numeric_value},
    mutation_result::{ambiguous_mutation_result, mutation_error_without_blind_retry},
    safety::validate_removal_acknowledgement,
};

const MAX_USERNAME_CHARS: usize = 30;
const MAX_BUDGET_MINOR_UNITS: u64 = i64::MAX as u64;
const MAX_UNIX_TIME: u64 = 4_102_444_800; // 2100-01-01T00:00:00Z
const MAX_PREDICTION_HORIZON_SECONDS: u64 = 8 * 7 * 24 * 60 * 60;

/// Grant one creator partnership-ad permission on an Instagram professional account.
/// Requires `instagram_branded_content_ads_brand`, `instagram_basic`, and
/// `business_management`, plus ADVERTISER access to the brand Instagram account.
/// This tool always sends `revoke=false`; it cannot revoke a permission.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GrantBrandedContentAdPermissionInput {
    /// Numeric brand Instagram professional-account ID.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub instagram_business_account_id: String,
    /// Numeric creator Instagram account ID (`creator_instagram_account` in Meta v26).
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub creator_instagram_account: String,
    /// Creator username without `@` (`creator_instagram_username` in Meta v26).
    #[schemars(length(min = 1, max = 30), regex(pattern = "^[A-Za-z0-9._]{1,30}$"))]
    pub creator_instagram_username: String,
}

/// Revoke one creator partnership-ad permission on an Instagram professional account.
/// This removal requires the exact operator acknowledgement before any Meta request.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevokeBrandedContentAdPermissionInput {
    /// Numeric brand Instagram professional-account ID.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub instagram_business_account_id: String,
    /// Numeric creator Instagram account ID (`creator_instagram_account` in Meta v26).
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub creator_instagram_account: String,
    /// Creator username without `@` (`creator_instagram_username` in Meta v26).
    #[schemars(length(min = 1, max = 30), regex(pattern = "^[A-Za-z0-9._]{1,30}$"))]
    pub creator_instagram_username: String,
    /// Exact phrase proving the operator approved this removal.
    #[schemars(
        length(min = 25, max = 25),
        regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
    )]
    pub removal_acknowledgement: String,
}

/// Create a bounded budget-to-reach quote for reservation planning. This does
/// not reserve inventory and exposes only the common Page/optional-Instagram
/// subset of Meta v26.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateReachFrequencyPredictionInput {
    /// Numeric Meta ad-account ID, with or without `act_`.
    #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
    pub ad_account_id: String,
    /// Numeric advertiser Page ID.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub facebook_page_id: String,
    /// Numeric Instagram professional-account ID when Instagram may deliver.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub instagram_business_account_id: Option<String>,
    /// Expected lifetime budget in account-currency minor units.
    #[schemars(range(min = 1, max = 9_223_372_036_854_775_807_u64))]
    pub budget: u64,
    /// Future Unix start timestamp.
    #[schemars(range(min = 1, max = 4_102_444_800_u64))]
    pub start_time: u64,
    /// Unix stop timestamp after start and no more than eight weeks ahead.
    #[schemars(range(min = 1, max = 4_102_444_800_u64))]
    pub stop_time: u64,
    /// One two-letter country code. Meta reservation predictions allow one country.
    #[schemars(length(min = 2, max = 2), regex(pattern = "^[A-Za-z]{2}$"))]
    pub country: String,
    /// Optional minimum age, 13 through 65.
    #[schemars(range(min = 13, max = 65))]
    pub age_min: Option<u8>,
    /// Optional maximum age, 13 through 65.
    #[schemars(range(min = 13, max = 65))]
    pub age_max: Option<u8>,
    /// Omit for all genders.
    pub gender: Option<ReservationGender>,
    /// Defaults to `REACH`; mobile-app installs are intentionally not exposed.
    pub objective: Option<ReservationObjective>,
    /// Optional positive lifetime frequency cap.
    #[schemars(range(min = 1))]
    pub frequency_cap: Option<u16>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReservationGender {
    Male,
    Female,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReservationObjective {
    BrandAwareness,
    LinkClicks,
    PostEngagement,
    Reach,
    VideoViews,
    WebsiteConversions,
}

impl ReservationObjective {
    const fn as_str(self) -> &'static str {
        match self {
            Self::BrandAwareness => "BRAND_AWARENESS",
            Self::LinkClicks => "LINK_CLICKS",
            Self::PostEngagement => "POST_ENGAGEMENT",
            Self::Reach => "REACH",
            Self::VideoViews => "VIDEO_VIEWS",
            Self::WebsiteConversions => "WEBSITE_CONVERSIONS",
        }
    }
}

/// Create one API-backed Threads account through a zero-parameter v26 POST.
/// Meta v26 creatives do not accept the returned ID, and repeat behavior is unspecified.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateThreadsAccountInput {
    pub mode: ThreadsAccountMode,
    /// Required only for `instagram_backed`; omit for `page_backed`.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub instagram_business_account_id: Option<String>,
    /// Required only for `page_backed`; omit for `instagram_backed`.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub facebook_page_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThreadsAccountMode {
    InstagramBacked,
    PageBacked,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GrantedBrandedContentAdPermission {
    pub permission_id: String,
    pub granted: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RevokedBrandedContentAdPermission {
    pub revoked: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CreatedReachFrequencyPrediction {
    pub rf_prediction_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CreatedThreadsAccount {
    pub threads_user_id: String,
}

#[derive(Debug, PartialEq, Eq)]
struct MutationRequest {
    endpoint: String,
    form: Vec<(String, String)>,
}

pub(crate) async fn grant_branded_content_ad_permission(
    graph: &GraphClient,
    input: GrantBrandedContentAdPermissionInput,
) -> ToolResponse<GrantedBrandedContentAdPermission> {
    let request = match build_permission_request(
        &input.instagram_business_account_id,
        &input.creator_instagram_account,
        &input.creator_instagram_username,
        false,
    ) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(mutation_error_without_blind_retry(
                error,
                "List partnership permissions before trying the grant again",
            ));
        }
    };
    match created_numeric_id(&payload, &["id"]) {
        Some(permission_id) => ToolResponse::success(GrantedBrandedContentAdPermission {
            permission_id,
            granted: true,
        }),
        None => ToolResponse::error(ambiguous_result(
            "Meta did not confirm the partnership permission ID",
            "List partnership permissions before trying the grant again",
        )),
    }
}

pub(crate) async fn revoke_branded_content_ad_permission(
    graph: &GraphClient,
    input: RevokeBrandedContentAdPermissionInput,
) -> ToolResponse<RevokedBrandedContentAdPermission> {
    if let Err(error) =
        validate_removal_acknowledgement(true, Some(input.removal_acknowledgement.as_str()))
    {
        return ToolResponse::error(error);
    }
    let request = match build_permission_request(
        &input.instagram_business_account_id,
        &input.creator_instagram_account,
        &input.creator_instagram_username,
        true,
    ) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(mutation_error_without_blind_retry(
                error,
                "List partnership permissions before trying the revocation again",
            ));
        }
    };
    if confirmed_permission_revocation(&payload) {
        ToolResponse::success(RevokedBrandedContentAdPermission { revoked: true })
    } else {
        ToolResponse::error(ambiguous_result(
            "Meta did not confirm the partnership permission revocation",
            "List partnership permissions before trying the revocation again",
        ))
    }
}

pub(crate) async fn create_reach_frequency_prediction(
    graph: &GraphClient,
    input: CreateReachFrequencyPredictionInput,
) -> ToolResponse<CreatedReachFrequencyPrediction> {
    let request = match build_prediction_request(&input, unix_now()) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(mutation_error_without_blind_retry(
                error,
                "List reach/frequency predictions before creating another one",
            ));
        }
    };
    match created_numeric_id(&payload, &["id"]) {
        Some(rf_prediction_id) => {
            ToolResponse::success(CreatedReachFrequencyPrediction { rf_prediction_id })
        }
        None => ToolResponse::error(ambiguous_result(
            "Meta did not confirm the reach/frequency prediction ID",
            "List reach/frequency predictions before creating another one",
        )),
    }
}

pub(crate) async fn create_threads_account(
    graph: &GraphClient,
    input: CreateThreadsAccountInput,
) -> ToolResponse<CreatedThreadsAccount> {
    let (request, response_keys) = match build_threads_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(mutation_error_without_blind_retry(
                error,
                "Call get_threads_account for this IG account or Page before retrying",
            ));
        }
    };
    match created_numeric_id(&payload, response_keys) {
        Some(threads_user_id) => ToolResponse::success(CreatedThreadsAccount { threads_user_id }),
        None => ToolResponse::error(ambiguous_result(
            "Meta did not confirm the Threads account ID",
            "Call get_threads_account for this IG account or Page before retrying",
        )),
    }
}

fn build_permission_request(
    instagram_business_account_id: &str,
    creator_instagram_account: &str,
    creator_instagram_username: &str,
    revoke: bool,
) -> Result<MutationRequest, PublicError> {
    let instagram_id = numeric_id(instagram_business_account_id).ok_or_else(|| {
        PublicError::invalid_input(
            "instagram_business_account_id must be a numeric Meta ID",
            "Use the brand Instagram professional-account ID",
        )
    })?;
    let creator_id = numeric_id(creator_instagram_account).ok_or_else(|| {
        PublicError::invalid_input(
            "creator_instagram_account must be a numeric Meta ID",
            "Use the creator Instagram professional-account ID",
        )
    })?;
    let username = normalize_username(creator_instagram_username)?;

    Ok(MutationRequest {
        endpoint: format!("{instagram_id}/branded_content_ad_permissions"),
        form: vec![
            ("creator_instagram_account".to_owned(), creator_id),
            ("creator_instagram_username".to_owned(), username),
            ("revoke".to_owned(), revoke.to_string()),
        ],
    })
}

fn build_prediction_request(
    input: &CreateReachFrequencyPredictionInput,
    now: u64,
) -> Result<MutationRequest, PublicError> {
    let account_id = normalize_ad_account_id(&input.ad_account_id).ok_or_else(|| {
        PublicError::invalid_input(
            "ad_account_id must be a numeric Meta account ID",
            "Use an ID such as `act_123456789` or `123456789`",
        )
    })?;
    let page_id = numeric_id(&input.facebook_page_id).ok_or_else(|| {
        PublicError::invalid_input(
            "facebook_page_id must be a numeric Meta Page ID",
            "Use the advertiser Page ID",
        )
    })?;
    let instagram_id = input
        .instagram_business_account_id
        .as_deref()
        .map(|value| {
            numeric_id(value).ok_or_else(|| {
                PublicError::invalid_input(
                    "instagram_business_account_id must be a numeric Meta ID",
                    "Use the advertiser Instagram professional-account ID",
                )
            })
        })
        .transpose()?;
    validate_prediction_budget(input.budget)?;
    validate_prediction_times(input.start_time, input.stop_time, now)?;
    let country = normalize_country(&input.country)?;
    validate_ages(input.age_min, input.age_max)?;
    if input.frequency_cap == Some(0) {
        return Err(PublicError::invalid_input(
            "frequency_cap must be greater than zero",
            "Omit frequency_cap or provide a positive lifetime cap",
        ));
    }

    let mut destinations = vec![page_id.as_str()];
    if let Some(instagram_id) = instagram_id.as_deref() {
        destinations.push(instagram_id);
    }
    let destination_ids = serde_json::to_string(&destinations).map_err(|_| encoding_error())?;
    let genders = input.gender.map(|gender| match gender {
        ReservationGender::Male => [1_u8],
        ReservationGender::Female => [2_u8],
    });
    let target_spec = serde_json::to_string(&TargetSpec {
        geo_locations: TargetGeoLocations {
            countries: [&country],
        },
        age_min: input.age_min,
        age_max: input.age_max,
        genders,
    })
    .map_err(|_| encoding_error())?;

    let mut form = vec![
        ("budget".to_owned(), input.budget.to_string()),
        ("destination_ids".to_owned(), destination_ids),
        (
            "objective".to_owned(),
            input
                .objective
                .unwrap_or(ReservationObjective::Reach)
                .as_str()
                .to_owned(),
        ),
        ("prediction_mode".to_owned(), "1".to_owned()),
        ("start_time".to_owned(), input.start_time.to_string()),
        ("stop_time".to_owned(), input.stop_time.to_string()),
        ("target_spec".to_owned(), target_spec),
    ];
    if let Some(frequency_cap) = input.frequency_cap {
        form.push(("frequency_cap".to_owned(), frequency_cap.to_string()));
    }

    Ok(MutationRequest {
        endpoint: format!("{account_id}/reachfrequencypredictions"),
        form,
    })
}

fn build_threads_request(
    input: &CreateThreadsAccountInput,
) -> Result<(MutationRequest, &'static [&'static str]), PublicError> {
    let (endpoint, response_keys) = match input.mode {
        ThreadsAccountMode::InstagramBacked => {
            if input.facebook_page_id.is_some() {
                return Err(PublicError::invalid_input(
                    "facebook_page_id does not apply to instagram_backed mode",
                    "Provide only instagram_business_account_id",
                ));
            }
            let instagram_business_account_id = input
                .instagram_business_account_id
                .as_deref()
                .ok_or_else(|| {
                    PublicError::invalid_input(
                        "instagram_business_account_id is required for instagram_backed mode",
                        "Provide one numeric Instagram professional-account ID",
                    )
                })?;
            let instagram_id = numeric_id(instagram_business_account_id).ok_or_else(|| {
                PublicError::invalid_input(
                    "instagram_business_account_id must be a numeric Meta ID",
                    "Use the Instagram professional-account ID",
                )
            })?;
            (
                format!("{instagram_id}/instagram_backed_threads_user"),
                &["threads_user_id", "id"][..],
            )
        }
        ThreadsAccountMode::PageBacked => {
            if input.instagram_business_account_id.is_some() {
                return Err(PublicError::invalid_input(
                    "instagram_business_account_id does not apply to page_backed mode",
                    "Provide only facebook_page_id",
                ));
            }
            let facebook_page_id = input.facebook_page_id.as_deref().ok_or_else(|| {
                PublicError::invalid_input(
                    "facebook_page_id is required for page_backed mode",
                    "Provide one numeric Facebook Page ID",
                )
            })?;
            let page_id = numeric_id(facebook_page_id).ok_or_else(|| {
                PublicError::invalid_input(
                    "facebook_page_id must be a numeric Meta Page ID",
                    "Use the Page that owns the requested Threads relationship",
                )
            })?;
            (
                format!("{page_id}/page_backed_threads_accounts"),
                &["id", "page_backed_threads_account_id", "threads_user_id"][..],
            )
        }
    };
    Ok((
        MutationRequest {
            endpoint,
            form: Vec::new(),
        },
        response_keys,
    ))
}

#[derive(Serialize)]
struct TargetSpec<'a> {
    geo_locations: TargetGeoLocations<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    age_min: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    age_max: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    genders: Option<[u8; 1]>,
}

#[derive(Serialize)]
struct TargetGeoLocations<'a> {
    countries: [&'a str; 1],
}

fn normalize_username(raw: &str) -> Result<String, PublicError> {
    let value = raw.trim().strip_prefix('@').unwrap_or(raw.trim());
    if value.is_empty()
        || value.len() > MAX_USERNAME_CHARS
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_'))
    {
        return Err(PublicError::invalid_input(
            "creator_instagram_username must be a valid bounded Instagram username",
            "Use 1 through 30 letters, digits, periods, or underscores without `@`",
        ));
    }
    Ok(value.to_ascii_lowercase())
}

fn normalize_country(raw: &str) -> Result<String, PublicError> {
    let value = raw.trim().to_ascii_uppercase();
    if value.len() != 2 || !value.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(PublicError::invalid_input(
            "country must be one two-letter code",
            "Use a code such as `US`, `GB`, or `CA`",
        ));
    }
    Ok(value)
}

fn validate_prediction_budget(budget: u64) -> Result<(), PublicError> {
    if budget == 0 || budget > MAX_BUDGET_MINOR_UNITS {
        return Err(PublicError::invalid_input(
            "budget must be a positive signed-64-bit minor-unit amount",
            "Use the expected lifetime budget in the ad account currency",
        ));
    }
    Ok(())
}

fn validate_prediction_times(start: u64, stop: u64, now: u64) -> Result<(), PublicError> {
    if start <= now
        || stop <= start
        || stop > MAX_UNIX_TIME
        || stop > now.saturating_add(MAX_PREDICTION_HORIZON_SECONDS)
    {
        return Err(PublicError::invalid_input(
            "prediction times must be future, ordered, and within eight weeks",
            "Use future Unix seconds; Meta also validates the account-timezone end hour",
        ));
    }
    Ok(())
}

fn validate_ages(minimum: Option<u8>, maximum: Option<u8>) -> Result<(), PublicError> {
    if minimum.is_some_and(|age| !(13..=65).contains(&age))
        || maximum.is_some_and(|age| !(13..=65).contains(&age))
        || minimum
            .zip(maximum)
            .is_some_and(|(minimum, maximum)| minimum > maximum)
    {
        return Err(PublicError::invalid_input(
            "ages must be ordered values from 13 through 65",
            "Omit ages for Meta defaults or provide a supported range",
        ));
    }
    Ok(())
}

fn encoding_error() -> PublicError {
    PublicError::invalid_input(
        "reach/frequency values could not be encoded",
        "Use the supported typed destination and targeting fields",
    )
}

fn created_numeric_id(payload: &Value, keys: &[&str]) -> Option<String> {
    if payload.get("success").and_then(Value::as_bool) == Some(false)
        || payload.get("error").is_some()
    {
        return None;
    }
    keys.iter()
        .find_map(|key| payload.get(*key).and_then(numeric_value))
        .or_else(|| {
            payload
                .get("data")
                .and_then(Value::as_array)
                .and_then(|data| (data.len() == 1).then_some(&data[0]))
                .and_then(|item| {
                    keys.iter()
                        .find_map(|key| item.get(*key).and_then(numeric_value))
                })
        })
}

fn confirmed_permission_revocation(payload: &Value) -> bool {
    payload.get("error").is_none()
        && (payload.get("success").and_then(Value::as_bool) == Some(true)
            || created_numeric_id(payload, &["id"]).is_some())
}

fn ambiguous_result(message: &str, action: &str) -> PublicError {
    ambiguous_mutation_result(message, action)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use rmcp::schemars::schema_for;
    use serde_json::json;

    use crate::{
        config::MetaConfig, error::ToolResponse, graph::GraphClient,
        safety::REMOVAL_ACKNOWLEDGEMENT,
    };

    use super::{
        CreateReachFrequencyPredictionInput, CreateThreadsAccountInput, ReservationGender,
        ReservationObjective, RevokeBrandedContentAdPermissionInput, ThreadsAccountMode,
        build_permission_request, build_prediction_request, build_threads_request,
        confirmed_permission_revocation, created_numeric_id, revoke_branded_content_ad_permission,
    };

    const NOW: u64 = 2_000_000_000;

    #[test]
    fn builds_exact_partnership_grant_and_revoke_forms() {
        let grant = build_permission_request(" 123 ", "456", "@Creator.Name", false).unwrap();

        assert_eq!(grant.endpoint, "123/branded_content_ad_permissions");
        assert_eq!(
            grant.form,
            [
                ("creator_instagram_account".to_owned(), "456".to_owned()),
                (
                    "creator_instagram_username".to_owned(),
                    "creator.name".to_owned()
                ),
                ("revoke".to_owned(), "false".to_owned()),
            ]
        );

        let revoke = build_permission_request("123", "456", "Creator.Name", true).unwrap();
        assert_eq!(revoke.endpoint, grant.endpoint);
        assert_eq!(revoke.form[..2], grant.form[..2]);
        assert_eq!(revoke.form[2], ("revoke".to_owned(), "true".to_owned()));
    }

    #[test]
    fn rejects_invalid_partnership_identifiers_and_unknown_fields() {
        assert!(build_permission_request("123/path", "456", "creator", false).is_err());
        assert!(
            serde_json::from_value::<RevokeBrandedContentAdPermissionInput>(json!({
                "instagram_business_account_id": "123",
                "creator_instagram_account": "456",
                "creator_instagram_username": "creator",
                "removal_acknowledgement": REMOVAL_ACKNOWLEDGEMENT,
                "unexpected": true
            }))
            .is_err()
        );
    }

    #[test]
    fn revoke_schema_is_closed_and_publishes_exact_acknowledgement() {
        let schema = serde_json::to_value(schema_for!(RevokeBrandedContentAdPermissionInput))
            .expect("JSON schema");
        assert_eq!(schema["additionalProperties"], false);
        let acknowledgement = &schema["properties"]["removal_acknowledgement"];
        assert_eq!(acknowledgement["minLength"], 25);
        assert_eq!(acknowledgement["maxLength"], 25);
        assert_eq!(acknowledgement["pattern"], "^CONFIRM_META_ADS_REMOVALS$");
    }

    #[test]
    fn recognizes_only_explicit_revocation_confirmations() {
        assert!(confirmed_permission_revocation(&json!({"success": true})));
        assert!(confirmed_permission_revocation(&json!({"id": "123"})));
        assert!(!confirmed_permission_revocation(&json!({"success": false})));
        assert!(!confirmed_permission_revocation(
            &json!({"error": {"code": 1}})
        ));
        assert!(!confirmed_permission_revocation(&json!({})));
    }

    #[tokio::test]
    async fn revoke_validates_acknowledgement_before_auth_or_network() {
        let graph = GraphClient::new(&MetaConfig::for_test("http://127.0.0.1:9/v26.0", None))
            .expect("test client");
        let input = |acknowledgement: &str| RevokeBrandedContentAdPermissionInput {
            instagram_business_account_id: "123".to_owned(),
            creator_instagram_account: "456".to_owned(),
            creator_instagram_username: "creator".to_owned(),
            removal_acknowledgement: acknowledgement.to_owned(),
        };

        let ToolResponse::Error { error } =
            revoke_branded_content_ad_permission(&graph, input("yes")).await
        else {
            panic!("invalid acknowledgement must fail");
        };
        assert_eq!(error.code, "INVALID_INPUT");

        let ToolResponse::Error { error } =
            revoke_branded_content_ad_permission(&graph, input(REMOVAL_ACKNOWLEDGEMENT)).await
        else {
            panic!("unauthenticated request must fail");
        };
        assert_eq!(error.code, "AUTH_REQUIRED");
    }

    #[test]
    fn builds_exact_common_reservation_prediction() {
        let request = build_prediction_request(
            &CreateReachFrequencyPredictionInput {
                ad_account_id: "123".to_owned(),
                facebook_page_id: "456".to_owned(),
                instagram_business_account_id: Some("789".to_owned()),
                budget: 50_000,
                start_time: NOW + 3_600,
                stop_time: NOW + 604_800,
                country: "us".to_owned(),
                age_min: Some(18),
                age_max: Some(65),
                gender: Some(ReservationGender::Female),
                objective: Some(ReservationObjective::VideoViews),
                frequency_cap: Some(3),
            },
            NOW,
        )
        .unwrap();

        assert_eq!(request.endpoint, "act_123/reachfrequencypredictions");
        assert_eq!(
            request.form,
            [
                ("budget".to_owned(), "50000".to_owned()),
                ("destination_ids".to_owned(), "[\"456\",\"789\"]".to_owned()),
                ("objective".to_owned(), "VIDEO_VIEWS".to_owned()),
                ("prediction_mode".to_owned(), "1".to_owned()),
                ("start_time".to_owned(), (NOW + 3_600).to_string()),
                ("stop_time".to_owned(), (NOW + 604_800).to_string()),
                (
                    "target_spec".to_owned(),
                    "{\"geo_locations\":{\"countries\":[\"US\"]},\"age_min\":18,\"age_max\":65,\"genders\":[2]}".to_owned()
                ),
                ("frequency_cap".to_owned(), "3".to_owned()),
            ]
        );
    }

    #[test]
    fn prediction_defaults_are_compact_and_bounds_are_enforced() {
        let valid = CreateReachFrequencyPredictionInput {
            ad_account_id: "act_123".to_owned(),
            facebook_page_id: "456".to_owned(),
            instagram_business_account_id: None,
            budget: 1,
            start_time: NOW + 1,
            stop_time: NOW + 2,
            country: "GB".to_owned(),
            age_min: None,
            age_max: None,
            gender: None,
            objective: None,
            frequency_cap: None,
        };
        let request = build_prediction_request(&valid, NOW).unwrap();
        assert_eq!(request.form[1].1, "[\"456\"]");
        assert_eq!(request.form[2].1, "REACH");
        assert_eq!(
            request.form[6].1,
            "{\"geo_locations\":{\"countries\":[\"GB\"]}}"
        );

        let mut invalid = valid;
        invalid.stop_time = NOW + 8 * 7 * 24 * 60 * 60 + 1;
        assert!(build_prediction_request(&invalid, NOW).is_err());
        invalid.stop_time = NOW + 2;
        invalid.age_min = Some(66);
        assert!(build_prediction_request(&invalid, NOW).is_err());
        invalid.age_min = None;
        invalid.budget = 0;
        assert!(build_prediction_request(&invalid, NOW).is_err());

        assert!(
            serde_json::from_value::<CreateReachFrequencyPredictionInput>(json!({
                "ad_account_id": "123",
                "facebook_page_id": "456",
                "budget": 1,
                "start_time": NOW + 1,
                "stop_time": NOW + 2,
                "country": "US",
                "prediction_mode": 0
            }))
            .is_err()
        );
    }

    #[test]
    fn builds_both_zero_parameter_threads_edges() {
        let (instagram, instagram_keys) = build_threads_request(&CreateThreadsAccountInput {
            mode: ThreadsAccountMode::InstagramBacked,
            instagram_business_account_id: Some("123".to_owned()),
            facebook_page_id: None,
        })
        .unwrap();
        assert_eq!(instagram.endpoint, "123/instagram_backed_threads_user");
        assert!(instagram.form.is_empty());
        assert_eq!(instagram_keys, ["threads_user_id", "id"]);

        let (page, page_keys) = build_threads_request(&CreateThreadsAccountInput {
            mode: ThreadsAccountMode::PageBacked,
            instagram_business_account_id: None,
            facebook_page_id: Some("456".to_owned()),
        })
        .unwrap();
        assert_eq!(page.endpoint, "456/page_backed_threads_accounts");
        assert!(page.form.is_empty());
        assert_eq!(
            page_keys,
            ["id", "page_backed_threads_account_id", "threads_user_id"]
        );
    }

    #[test]
    fn threads_mode_rejects_unknown_fields_and_ids() {
        assert!(
            serde_json::from_value::<CreateThreadsAccountInput>(json!({
                "mode": "instagram_backed",
                "instagram_business_account_id": "123",
                "unexpected": true
            }))
            .is_err()
        );
        assert!(
            build_threads_request(&CreateThreadsAccountInput {
                mode: ThreadsAccountMode::PageBacked,
                instagram_business_account_id: None,
                facebook_page_id: Some("page-456".to_owned()),
            })
            .is_err()
        );
        assert!(
            build_threads_request(&CreateThreadsAccountInput {
                mode: ThreadsAccountMode::InstagramBacked,
                instagram_business_account_id: Some("123".to_owned()),
                facebook_page_id: Some("456".to_owned()),
            })
            .is_err()
        );
    }

    #[test]
    fn accepts_only_one_bounded_numeric_mutation_id() {
        assert_eq!(
            created_numeric_id(&json!({"id": "123"}), &["id"]),
            Some("123".to_owned())
        );
        assert_eq!(
            created_numeric_id(
                &json!({"data": [{"threads_user_id": 456}]}),
                &["threads_user_id"]
            ),
            Some("456".to_owned())
        );
        assert_eq!(
            created_numeric_id(
                &json!({"data": [{"threads_user_id": "1"}, {"threads_user_id": "2"}]}),
                &["threads_user_id"]
            ),
            None
        );
    }
}
