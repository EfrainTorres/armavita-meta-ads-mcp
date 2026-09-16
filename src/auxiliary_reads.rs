use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{ad_account as normalize_ad_account_id, numeric_owned as normalize_numeric_id},
};

const DEFAULT_PAGE_SIZE: u16 = 25;
const MAX_PAGE_SIZE: u16 = 100;
const MAX_CURSOR_CHARS: usize = 2_048;
const MAX_NAME_CHARS: usize = 256;
const MAX_DESCRIPTION_CHARS: usize = 512;
const MAX_FORMULA_CHARS: usize = 2_048;
const MAX_METADATA_CHARS: usize = 128;
const MAX_MESSAGE_CHARS: usize = 1_000;
const MAX_USERNAME_CHARS: usize = 64;
const MAX_RECOMMENDATIONS: usize = 100;
const MAX_RECOMMENDATION_OBJECT_IDS: usize = 100;
const MAX_CURVE_POINTS: usize = 64;

// Meta v26.0 exposes this edge on Business, not AdAccount.
const DERIVED_METRIC_FIELDS: &str = "id,ad_account_id,creation_time,custom_derived_metric_type,description,format_type,formula,has_attribution_windows,has_inline_attribution_window,name,permission,saved_report_id,scope";
const RECOMMENDATION_FIELDS: &str = "recommendations";
const RF_LIST_FIELDS: &str = "id,name,status,prediction_progress,reservation_status,campaign_id,currency,objective_name,start_time,end_time,expiration_time,frequency_cap,budget,reach,impression,destination_id,instagram_destination_id";
const RF_DETAIL_FIELDS: &str = "id,name,status,prediction_progress,reservation_status,account_id,campaign_group_id,campaign_id,buying_type,currency,objective_name,start_time,end_time,expiration_time,time_created,time_updated,frequency_cap,prediction_mode,budget,reach,impression,external_budget,external_reach,external_impression,external_minimum_budget,external_maximum_budget,external_minimum_reach,external_maximum_reach,external_minimum_impression,external_maximum_impression,destination_id,instagram_destination_id,curve_budget_reach";

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DerivedMetricScope {
    Account,
    Business,
    BusinessAssetGroup,
}

impl DerivedMetricScope {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Account => "ACCOUNT",
            Self::Business => "BUSINESS",
            Self::BusinessAssetGroup => "BUSINESS_ASSET_GROUP",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListAdCustomDerivedMetricsInput {
    /// Numeric Meta Business ID. In v26.0 this edge is not on an ad account.
    pub business_id: String,
    /// Optional v26.0 metric scope.
    pub scope: Option<DerivedMetricScope>,
    /// Number of metrics to return, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DerivedMetricPage {
    pub business_id: String,
    pub metrics: Vec<DerivedMetric>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DerivedMetric {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ad_account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_report_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_attribution_windows: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_inline_attribution_window: Option<bool>,
    #[serde(skip_serializing_if = "is_false")]
    pub content_truncated: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListRecommendationsInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    pub ad_account_id: String,
    /// Number of recommendation envelopes to request, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RecommendationPage {
    pub recommendations: Vec<AdRecommendation>,
    #[serde(skip_serializing_if = "is_false")]
    pub recommendations_truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Advisory Meta recommendation. Provider action payloads, URLs, and signatures are omitted.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AdRecommendation {
    pub recommendation_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommendation_stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommendation_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lift_estimate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opportunity_score_lift: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub content_truncated: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListBrandedContentAdPermissionsInput {
    /// Numeric Instagram professional/business account ID (the v26.0 IG User node).
    pub instagram_business_account_id: String,
    /// Optional creator username filter, with or without a leading `@`.
    pub creator_username: Option<String>,
    /// Number of permissions to return, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BrandedContentPermissionPage {
    pub permissions: Vec<BrandedContentPermission>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Account-level partnership-ad permission. Requires `instagram_branded_content_ads_brand`,
/// `instagram_basic`, and `business_management`, plus ADVERTISER access to the IG account.
#[derive(Debug, Serialize, JsonSchema)]
pub struct BrandedContentPermission {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator_username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator_facebook_page_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand_instagram_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_type: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadReachFrequencyPredictionInput {
    /// Numeric reach-and-frequency prediction ID.
    pub rf_prediction_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListReachFrequencyPredictionsInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    pub ad_account_id: String,
    /// Number of predictions to return, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ReachFrequencyPredictionPage {
    pub predictions: Vec<ReachFrequencyPredictionSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ReachFrequencyPredictionSummary {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prediction_progress: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reservation_status: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub campaign_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiration_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_cap: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reach: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impression: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instagram_destination_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ReachFrequencyPrediction {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prediction_progress: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reservation_status: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub campaign_group_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub campaign_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buying_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiration_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_updated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_cap: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prediction_mode: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reach: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impression: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_budget: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_reach: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_impression: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_minimum_budget: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_maximum_budget: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_minimum_reach: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_maximum_reach: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_minimum_impression: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_maximum_impression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instagram_destination_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub curve_budget_reach: Option<ReachFrequencyCurve>,
}

/// Bounded reach/budget curve. Arrays retain Meta's index alignment and are never aggregated.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ReachFrequencyCurve {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budgets: Option<Vec<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reaches: Option<Vec<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impressions: Option<Vec<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversions: Option<Vec<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_reaches: Option<Vec<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_impressions: Option<Vec<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported_point_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interpolated_reach: Option<f64>,
    #[serde(skip_serializing_if = "is_false")]
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThreadsAccountMode {
    Associated,
    InstagramBacked,
    PageBacked,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetThreadsAccountInput {
    /// Required for `associated` and `instagram_backed` modes.
    pub instagram_business_account_id: Option<String>,
    /// Required for `page_backed` mode.
    pub facebook_page_id: Option<String>,
    /// Threads account relationship to discover. Defaults to `associated`.
    pub mode: Option<ThreadsAccountMode>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ThreadsAccount {
    pub mode: ThreadsAccountMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threads_user_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawPaging {
    cursors: Option<RawCursors>,
}

#[derive(Debug, Deserialize)]
struct RawCursors {
    after: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawDerivedMetricPage {
    #[serde(default)]
    data: Vec<RawDerivedMetric>,
    paging: Option<RawPaging>,
}

#[derive(Debug, Deserialize)]
struct RawDerivedMetric {
    id: Option<String>,
    ad_account_id: Option<String>,
    name: Option<String>,
    description: Option<String>,
    formula: Option<String>,
    format_type: Option<String>,
    custom_derived_metric_type: Option<String>,
    scope: Option<String>,
    permission: Option<String>,
    saved_report_id: Option<String>,
    creation_time: Option<String>,
    has_attribution_windows: Option<bool>,
    has_inline_attribution_window: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct RawRecommendationPage {
    #[serde(default)]
    data: Vec<RawRecommendationEnvelope>,
    paging: Option<RawPaging>,
}

#[derive(Debug, Deserialize)]
struct RawRecommendationEnvelope {
    #[serde(default)]
    recommendations: Vec<RawAdRecommendation>,
}

#[derive(Debug, Deserialize)]
struct RawAdRecommendation {
    #[serde(rename = "type")]
    recommendation_type: String,
    #[serde(default)]
    object_ids: Vec<Value>,
    recommendation_content: Option<RawRecommendationContent>,
    recommendation_stage: Option<String>,
    recommendation_time: Option<Value>,
    // Provider URLs and recommendation signatures are deliberately not deserialized.
}

#[derive(Debug, Default, Deserialize)]
struct RawRecommendationContent {
    body: Option<String>,
    lift_estimate: Option<Value>,
    opportunity_score_lift: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct RawBrandedContentPermissionPage {
    #[serde(default)]
    data: Vec<RawBrandedContentPermission>,
    paging: Option<RawPaging>,
}

#[derive(Debug, Deserialize)]
struct RawBrandedContentPermission {
    id: Option<String>,
    creator_username: Option<String>,
    creator_id: Option<String>,
    creator_fb_page: Option<String>,
    brand_ig_user: Option<RawId>,
    permission_status: Option<String>,
    // These two names remain in Meta's v26.0 generated IGBCAdsPermission type.
    status: Option<String>,
    permission_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawId {
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawReachFrequencyPage {
    #[serde(default)]
    data: Vec<RawReachFrequencyPrediction>,
    paging: Option<RawPaging>,
}

#[derive(Debug, Deserialize)]
struct RawReachFrequencyPrediction {
    id: Option<String>,
    name: Option<String>,
    status: Option<Value>,
    prediction_progress: Option<Value>,
    reservation_status: Option<Value>,
    account_id: Option<Value>,
    campaign_group_id: Option<Value>,
    campaign_id: Option<Value>,
    buying_type: Option<String>,
    currency: Option<String>,
    objective_name: Option<String>,
    start_time: Option<String>,
    end_time: Option<String>,
    expiration_time: Option<String>,
    time_created: Option<String>,
    time_updated: Option<String>,
    frequency_cap: Option<Value>,
    prediction_mode: Option<Value>,
    budget: Option<Value>,
    reach: Option<Value>,
    impression: Option<Value>,
    external_budget: Option<Value>,
    external_reach: Option<Value>,
    external_impression: Option<Value>,
    external_minimum_budget: Option<Value>,
    external_maximum_budget: Option<Value>,
    external_minimum_reach: Option<Value>,
    external_maximum_reach: Option<Value>,
    external_minimum_impression: Option<Value>,
    external_maximum_impression: Option<Value>,
    destination_id: Option<Value>,
    instagram_destination_id: Option<Value>,
    curve_budget_reach: Option<RawReachFrequencyCurve>,
}

#[derive(Debug, Deserialize)]
struct RawReachFrequencyCurve {
    budget: Option<Vec<i64>>,
    reach: Option<Vec<i64>>,
    impression: Option<Vec<i64>>,
    conversion: Option<Vec<i64>>,
    raw_reach: Option<Vec<i64>>,
    raw_impression: Option<Vec<i64>>,
    num_points: Option<u64>,
    interpolated_reach: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct RawThreadsEdge {
    #[serde(default)]
    data: Vec<RawThreadsUser>,
}

#[derive(Debug, Deserialize)]
struct RawThreadsUser {
    threads_user_id: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct RawPageThreadsAccount {
    page_backed_threads_account_id: Option<Value>,
}

pub(crate) async fn list_ad_custom_derived_metrics(
    graph: &GraphClient,
    input: ListAdCustomDerivedMetricsInput,
) -> ToolResponse<DerivedMetricPage> {
    let business_id = match normalize_numeric_id(&input.business_id) {
        Some(id) => id,
        None => {
            return ToolResponse::error(PublicError::invalid_input(
                "business_id must be a numeric Meta Business ID",
                "Use the Business ID that owns the custom derived metrics",
            ));
        }
    };
    let (page_size, cursor) = match validate_page(input.page_size, input.page_cursor.as_deref()) {
        Ok(page) => page,
        Err(error) => return ToolResponse::error(error),
    };
    let mut query = page_query(DERIVED_METRIC_FIELDS, page_size, cursor);
    if let Some(scope) = input.scope {
        query.push(("scope".to_owned(), scope.as_str().to_owned()));
    }

    let payload = match graph
        .get_json(&format!("{business_id}/ad_custom_derived_metrics"), &query)
        .await
    {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    match parse_derived_metric_page(payload, business_id, page_size) {
        Ok(page) => ToolResponse::success(page),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn list_recommendations(
    graph: &GraphClient,
    input: ListRecommendationsInput,
) -> ToolResponse<RecommendationPage> {
    let account_id = match normalize_ad_account_id(&input.ad_account_id) {
        Some(id) => id,
        None => {
            return ToolResponse::error(PublicError::invalid_input(
                "ad_account_id must be a numeric Meta account ID",
                "Use an ID such as `act_123456789` or `123456789`",
            ));
        }
    };
    let (page_size, cursor) = match validate_page(input.page_size, input.page_cursor.as_deref()) {
        Ok(page) => page,
        Err(error) => return ToolResponse::error(error),
    };
    let query = page_query(RECOMMENDATION_FIELDS, page_size, cursor);
    let payload = match graph
        .get_json(&format!("{account_id}/recommendations"), &query)
        .await
    {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    match parse_recommendation_page(payload, page_size) {
        Ok(page) => ToolResponse::success(page),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn list_branded_content_ad_permissions(
    graph: &GraphClient,
    input: ListBrandedContentAdPermissionsInput,
) -> ToolResponse<BrandedContentPermissionPage> {
    let instagram_id = match normalize_numeric_id(&input.instagram_business_account_id) {
        Some(id) => id,
        None => {
            return ToolResponse::error(PublicError::invalid_input(
                "instagram_business_account_id must be a numeric Instagram professional-account ID",
                "Use the IG User ID for the advertiser's Instagram business account",
            ));
        }
    };
    let creator_username = match input.creator_username.as_deref() {
        Some(username) => match normalize_username(username) {
            Some(username) => Some(username),
            None => {
                return ToolResponse::error(PublicError::invalid_input(
                    "creator_username contains unsupported characters or is too long",
                    "Use an Instagram username containing letters, digits, periods, or underscores",
                ));
            }
        },
        None => None,
    };
    let (page_size, cursor) = match validate_page(input.page_size, input.page_cursor.as_deref()) {
        Ok(page) => page,
        Err(error) => return ToolResponse::error(error),
    };
    // The current Meta guide documents default response fields for this edge. Avoid guessing a
    // `fields` projection while the generated v26.0 SDK still exposes the older field names.
    let mut query = page_query_without_fields(page_size, cursor);
    if let Some(username) = creator_username {
        query.push(("creator_username".to_owned(), username));
    }
    let payload = match graph
        .get_json(
            &format!("{instagram_id}/branded_content_ad_permissions"),
            &query,
        )
        .await
    {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    match parse_branded_content_permission_page(payload, page_size) {
        Ok(page) => ToolResponse::success(page),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn read_reach_frequency_prediction(
    graph: &GraphClient,
    input: ReadReachFrequencyPredictionInput,
) -> ToolResponse<ReachFrequencyPrediction> {
    let prediction_id = match normalize_numeric_id(&input.rf_prediction_id) {
        Some(id) => id,
        None => {
            return ToolResponse::error(PublicError::invalid_input(
                "rf_prediction_id must be a numeric Meta prediction ID",
                "Use the ID returned by list_reach_frequency_predictions",
            ));
        }
    };
    let query = vec![("fields".to_owned(), RF_DETAIL_FIELDS.to_owned())];
    let payload = match graph.get_json(&prediction_id, &query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    let raw = match serde_json::from_value::<RawReachFrequencyPrediction>(payload) {
        Ok(raw) => raw,
        Err(_) => {
            return ToolResponse::error(PublicError::invalid_upstream(
                "Meta returned unexpected reach-and-frequency prediction metadata",
            ));
        }
    };
    match normalize_prediction(raw) {
        Some(prediction) => ToolResponse::success(prediction),
        None => ToolResponse::error(PublicError::invalid_upstream(
            "Meta omitted a valid reach-and-frequency prediction ID",
        )),
    }
}

pub(crate) async fn list_reach_frequency_predictions(
    graph: &GraphClient,
    input: ListReachFrequencyPredictionsInput,
) -> ToolResponse<ReachFrequencyPredictionPage> {
    let account_id = match normalize_ad_account_id(&input.ad_account_id) {
        Some(id) => id,
        None => {
            return ToolResponse::error(PublicError::invalid_input(
                "ad_account_id must be a numeric Meta account ID",
                "Use an ID such as `act_123456789` or `123456789`",
            ));
        }
    };
    let (page_size, cursor) = match validate_page(input.page_size, input.page_cursor.as_deref()) {
        Ok(page) => page,
        Err(error) => return ToolResponse::error(error),
    };
    let query = page_query(RF_LIST_FIELDS, page_size, cursor);
    let payload = match graph
        .get_json(&format!("{account_id}/reachfrequencypredictions"), &query)
        .await
    {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    match parse_reach_frequency_page(payload, page_size) {
        Ok(page) => ToolResponse::success(page),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn get_threads_account(
    graph: &GraphClient,
    input: GetThreadsAccountInput,
) -> ToolResponse<ThreadsAccount> {
    let mode = input.mode.unwrap_or(ThreadsAccountMode::Associated);
    let request = match build_threads_request(&input, mode) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.get_json(&request.0, &request.1).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };

    let result = match mode {
        ThreadsAccountMode::Associated | ThreadsAccountMode::InstagramBacked => {
            parse_threads_edge(payload, mode)
        }
        ThreadsAccountMode::PageBacked => parse_page_threads_account(payload, mode),
    };
    match result {
        Ok(account) => ToolResponse::success(account),
        Err(error) => ToolResponse::error(error),
    }
}

fn parse_derived_metric_page(
    payload: Value,
    business_id: String,
    requested_page_size: u16,
) -> Result<DerivedMetricPage, PublicError> {
    let raw = serde_json::from_value::<RawDerivedMetricPage>(payload).map_err(|_| {
        PublicError::invalid_upstream("Meta returned an unexpected custom-derived-metric list")
    })?;
    check_page_len(
        raw.data.len(),
        requested_page_size,
        "custom derived metrics",
    )?;
    let next_cursor = next_cursor(raw.paging)?;
    let metrics = raw
        .data
        .into_iter()
        .filter_map(normalize_derived_metric)
        .collect();
    Ok(DerivedMetricPage {
        business_id,
        metrics,
        next_cursor,
    })
}

fn normalize_derived_metric(raw: RawDerivedMetric) -> Option<DerivedMetric> {
    let id = normalize_numeric_id(raw.id.as_deref()?)?;
    let (description, description_truncated) = bounded_text(raw.description, MAX_DESCRIPTION_CHARS);
    let (formula, formula_truncated) = bounded_text(raw.formula, MAX_FORMULA_CHARS);
    Some(DerivedMetric {
        id,
        ad_account_id: raw
            .ad_account_id
            .as_deref()
            .and_then(normalize_ad_account_id),
        name: bounded_text(raw.name, MAX_NAME_CHARS).0,
        description,
        formula,
        format_type: bounded_text(raw.format_type, MAX_METADATA_CHARS).0,
        metric_type: bounded_text(raw.custom_derived_metric_type, MAX_METADATA_CHARS).0,
        scope: bounded_text(raw.scope, MAX_METADATA_CHARS).0,
        permission: bounded_text(raw.permission, MAX_METADATA_CHARS).0,
        saved_report_id: raw
            .saved_report_id
            .as_deref()
            .and_then(normalize_numeric_id),
        creation_time: bounded_text(raw.creation_time, MAX_METADATA_CHARS).0,
        has_attribution_windows: raw.has_attribution_windows,
        has_inline_attribution_window: raw.has_inline_attribution_window,
        content_truncated: description_truncated || formula_truncated,
    })
}

fn parse_recommendation_page(
    payload: Value,
    requested_page_size: u16,
) -> Result<RecommendationPage, PublicError> {
    let raw = serde_json::from_value::<RawRecommendationPage>(payload).map_err(|_| {
        PublicError::invalid_upstream("Meta returned an unexpected recommendation list")
    })?;
    check_page_len(
        raw.data.len(),
        requested_page_size,
        "recommendation envelopes",
    )?;
    let next_cursor = next_cursor(raw.paging)?;
    let mut recommendations = Vec::new();
    let mut recommendations_truncated = false;
    for recommendation in raw
        .data
        .into_iter()
        .flat_map(|envelope| envelope.recommendations)
    {
        if recommendations.len() == MAX_RECOMMENDATIONS {
            recommendations_truncated = true;
            break;
        }
        recommendations.push(normalize_recommendation(recommendation)?);
    }
    Ok(RecommendationPage {
        recommendations,
        recommendations_truncated,
        next_cursor,
    })
}

fn normalize_recommendation(raw: RawAdRecommendation) -> Result<AdRecommendation, PublicError> {
    let (recommendation_type, type_truncated) =
        bounded_text(Some(raw.recommendation_type), MAX_METADATA_CHARS);
    let recommendation_type = recommendation_type.ok_or_else(|| {
        PublicError::invalid_upstream("Meta returned a recommendation without a usable type")
    })?;
    let object_ids_truncated = raw.object_ids.len() > MAX_RECOMMENDATION_OBJECT_IDS;
    let object_ids = raw
        .object_ids
        .into_iter()
        .take(MAX_RECOMMENDATION_OBJECT_IDS)
        .map(|id| {
            numeric_value_string(Some(id)).ok_or_else(|| {
                PublicError::invalid_upstream("Meta returned an invalid recommendation object ID")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let content = raw.recommendation_content.unwrap_or_default();
    let (stage, stage_truncated) = bounded_text(raw.recommendation_stage, MAX_METADATA_CHARS);
    let (time, time_truncated) =
        bounded_scalar_value_string(raw.recommendation_time, MAX_METADATA_CHARS);
    let (body, body_truncated) = bounded_text(content.body, MAX_MESSAGE_CHARS);
    let (lift_estimate, lift_truncated) =
        bounded_scalar_value_string(content.lift_estimate, MAX_METADATA_CHARS);
    let (opportunity_score_lift, score_truncated) =
        bounded_scalar_value_string(content.opportunity_score_lift, MAX_METADATA_CHARS);
    Ok(AdRecommendation {
        recommendation_type,
        object_ids: (!object_ids.is_empty()).then_some(object_ids),
        recommendation_stage: stage,
        recommendation_time: time,
        body,
        lift_estimate,
        opportunity_score_lift,
        content_truncated: type_truncated
            || object_ids_truncated
            || stage_truncated
            || time_truncated
            || body_truncated
            || lift_truncated
            || score_truncated,
    })
}

fn parse_branded_content_permission_page(
    payload: Value,
    requested_page_size: u16,
) -> Result<BrandedContentPermissionPage, PublicError> {
    let raw = serde_json::from_value::<RawBrandedContentPermissionPage>(payload).map_err(|_| {
        PublicError::invalid_upstream("Meta returned an unexpected partnership-permission list")
    })?;
    check_page_len(
        raw.data.len(),
        requested_page_size,
        "partnership permissions",
    )?;
    let next_cursor = next_cursor(raw.paging)?;
    let permissions = raw
        .data
        .into_iter()
        .filter_map(normalize_branded_content_permission)
        .collect();
    Ok(BrandedContentPermissionPage {
        permissions,
        next_cursor,
    })
}

fn normalize_branded_content_permission(
    raw: RawBrandedContentPermission,
) -> Option<BrandedContentPermission> {
    let id = normalize_numeric_id(raw.id.as_deref()?)?;
    Some(BrandedContentPermission {
        id,
        creator_username: raw
            .creator_username
            .and_then(|value| normalize_username(&value)),
        creator_id: raw.creator_id.as_deref().and_then(normalize_numeric_id),
        creator_facebook_page_id: raw
            .creator_fb_page
            .as_deref()
            .and_then(normalize_numeric_id),
        brand_instagram_user_id: raw
            .brand_ig_user
            .and_then(|user| user.id)
            .as_deref()
            .and_then(normalize_numeric_id),
        status: bounded_text(raw.permission_status.or(raw.status), MAX_METADATA_CHARS).0,
        permission_type: bounded_text(raw.permission_type, MAX_METADATA_CHARS).0,
    })
}

fn parse_reach_frequency_page(
    payload: Value,
    requested_page_size: u16,
) -> Result<ReachFrequencyPredictionPage, PublicError> {
    let raw = serde_json::from_value::<RawReachFrequencyPage>(payload).map_err(|_| {
        PublicError::invalid_upstream("Meta returned an unexpected reach-and-frequency list")
    })?;
    check_page_len(
        raw.data.len(),
        requested_page_size,
        "reach-and-frequency predictions",
    )?;
    let next_cursor = next_cursor(raw.paging)?;
    let predictions = raw
        .data
        .into_iter()
        .filter_map(normalize_prediction_summary)
        .collect();
    Ok(ReachFrequencyPredictionPage {
        predictions,
        next_cursor,
    })
}

fn normalize_prediction_summary(
    raw: RawReachFrequencyPrediction,
) -> Option<ReachFrequencyPredictionSummary> {
    let id = normalize_numeric_id(raw.id.as_deref()?)?;
    Some(ReachFrequencyPredictionSummary {
        id,
        name: bounded_text(raw.name, MAX_NAME_CHARS).0,
        status: value_u64(raw.status),
        prediction_progress: value_u64(raw.prediction_progress),
        reservation_status: value_u64(raw.reservation_status),
        campaign_id: numeric_value_string(raw.campaign_id),
        currency: bounded_text(raw.currency, 3).0,
        objective_name: bounded_text(raw.objective_name, MAX_METADATA_CHARS).0,
        start_time: bounded_text(raw.start_time, MAX_METADATA_CHARS).0,
        end_time: bounded_text(raw.end_time, MAX_METADATA_CHARS).0,
        expiration_time: bounded_text(raw.expiration_time, MAX_METADATA_CHARS).0,
        frequency_cap: value_u64(raw.frequency_cap),
        budget: value_u64(raw.budget),
        reach: value_u64(raw.reach),
        impression: value_u64(raw.impression),
        destination_id: numeric_value_string(raw.destination_id),
        instagram_destination_id: numeric_value_string(raw.instagram_destination_id),
    })
}

fn normalize_prediction(raw: RawReachFrequencyPrediction) -> Option<ReachFrequencyPrediction> {
    let id = normalize_numeric_id(raw.id.as_deref()?)?;
    Some(ReachFrequencyPrediction {
        id,
        name: bounded_text(raw.name, MAX_NAME_CHARS).0,
        status: value_u64(raw.status),
        prediction_progress: value_u64(raw.prediction_progress),
        reservation_status: value_u64(raw.reservation_status),
        account_id: numeric_value_string(raw.account_id),
        campaign_group_id: numeric_value_string(raw.campaign_group_id),
        campaign_id: numeric_value_string(raw.campaign_id),
        buying_type: bounded_text(raw.buying_type, MAX_METADATA_CHARS).0,
        currency: bounded_text(raw.currency, 3).0,
        objective_name: bounded_text(raw.objective_name, MAX_METADATA_CHARS).0,
        start_time: bounded_text(raw.start_time, MAX_METADATA_CHARS).0,
        end_time: bounded_text(raw.end_time, MAX_METADATA_CHARS).0,
        expiration_time: bounded_text(raw.expiration_time, MAX_METADATA_CHARS).0,
        time_created: bounded_text(raw.time_created, MAX_METADATA_CHARS).0,
        time_updated: bounded_text(raw.time_updated, MAX_METADATA_CHARS).0,
        frequency_cap: value_u64(raw.frequency_cap),
        prediction_mode: value_u64(raw.prediction_mode),
        budget: value_u64(raw.budget),
        reach: value_u64(raw.reach),
        impression: value_u64(raw.impression),
        external_budget: value_i64(raw.external_budget),
        external_reach: value_u64(raw.external_reach),
        external_impression: value_u64(raw.external_impression),
        external_minimum_budget: value_i64(raw.external_minimum_budget),
        external_maximum_budget: value_i64(raw.external_maximum_budget),
        external_minimum_reach: value_u64(raw.external_minimum_reach),
        external_maximum_reach: value_u64(raw.external_maximum_reach),
        external_minimum_impression: value_u64(raw.external_minimum_impression),
        external_maximum_impression: scalar_value_string(raw.external_maximum_impression),
        destination_id: numeric_value_string(raw.destination_id),
        instagram_destination_id: numeric_value_string(raw.instagram_destination_id),
        curve_budget_reach: raw.curve_budget_reach.map(normalize_curve),
    })
}

fn normalize_curve(raw: RawReachFrequencyCurve) -> ReachFrequencyCurve {
    let (budgets, budgets_truncated) = bounded_numbers(raw.budget);
    let (reaches, reaches_truncated) = bounded_numbers(raw.reach);
    let (impressions, impressions_truncated) = bounded_numbers(raw.impression);
    let (conversions, conversions_truncated) = bounded_numbers(raw.conversion);
    let (raw_reaches, raw_reaches_truncated) = bounded_numbers(raw.raw_reach);
    let (raw_impressions, raw_impressions_truncated) = bounded_numbers(raw.raw_impression);
    ReachFrequencyCurve {
        budgets,
        reaches,
        impressions,
        conversions,
        raw_reaches,
        raw_impressions,
        reported_point_count: raw.num_points,
        interpolated_reach: raw.interpolated_reach.filter(|value| value.is_finite()),
        truncated: budgets_truncated
            || reaches_truncated
            || impressions_truncated
            || conversions_truncated
            || raw_reaches_truncated
            || raw_impressions_truncated,
    }
}

fn build_threads_request(
    input: &GetThreadsAccountInput,
    mode: ThreadsAccountMode,
) -> Result<(String, Vec<(String, String)>), PublicError> {
    match mode {
        ThreadsAccountMode::Associated | ThreadsAccountMode::InstagramBacked => {
            let instagram_id = input
                .instagram_business_account_id
                .as_deref()
                .and_then(normalize_numeric_id)
                .ok_or_else(|| {
                    PublicError::invalid_input(
                        "instagram_business_account_id must be a numeric ID for this mode",
                        "Provide the advertiser's Instagram professional-account ID",
                    )
                })?;
            let edge = match mode {
                ThreadsAccountMode::Associated => "connected_threads_user",
                ThreadsAccountMode::InstagramBacked => "instagram_backed_threads_user",
                ThreadsAccountMode::PageBacked => unreachable!(),
            };
            Ok((
                format!("{instagram_id}/{edge}"),
                vec![
                    ("fields".to_owned(), "threads_user_id".to_owned()),
                    ("limit".to_owned(), "1".to_owned()),
                ],
            ))
        }
        ThreadsAccountMode::PageBacked => {
            let page_id = input
                .facebook_page_id
                .as_deref()
                .and_then(normalize_numeric_id)
                .ok_or_else(|| {
                    PublicError::invalid_input(
                        "facebook_page_id must be a numeric Page ID for page_backed mode",
                        "Provide the Facebook Page backing the Threads ad account",
                    )
                })?;
            Ok((
                page_id,
                vec![(
                    "fields".to_owned(),
                    "page_backed_threads_account_id".to_owned(),
                )],
            ))
        }
    }
}

fn parse_threads_edge(
    payload: Value,
    mode: ThreadsAccountMode,
) -> Result<ThreadsAccount, PublicError> {
    let raw = serde_json::from_value::<RawThreadsEdge>(payload).map_err(|_| {
        PublicError::invalid_upstream("Meta returned an unexpected Threads account result")
    })?;
    if raw.data.len() > 1 {
        return Err(PublicError::invalid_upstream(
            "Meta returned multiple Threads accounts for a singular relationship",
        ));
    }
    let threads_user_id = raw
        .data
        .into_iter()
        .next()
        .and_then(|user| numeric_value_string(user.threads_user_id));
    Ok(ThreadsAccount {
        mode,
        threads_user_id,
    })
}

fn parse_page_threads_account(
    payload: Value,
    mode: ThreadsAccountMode,
) -> Result<ThreadsAccount, PublicError> {
    let raw = serde_json::from_value::<RawPageThreadsAccount>(payload).map_err(|_| {
        PublicError::invalid_upstream(
            "Meta returned an unexpected page-backed Threads account result",
        )
    })?;
    Ok(ThreadsAccount {
        mode,
        threads_user_id: numeric_value_string(raw.page_backed_threads_account_id),
    })
}

fn validate_page(
    page_size: Option<u16>,
    page_cursor: Option<&str>,
) -> Result<(u16, Option<&str>), PublicError> {
    let page_size = page_size.unwrap_or(DEFAULT_PAGE_SIZE);
    if !(1..=MAX_PAGE_SIZE).contains(&page_size) {
        return Err(PublicError::invalid_input(
            "page_size must be between 1 and 100",
            "Choose a bounded page_size and paginate with next_cursor",
        ));
    }
    let cursor = page_cursor.filter(|cursor| !cursor.is_empty());
    if cursor.is_some_and(|cursor| cursor.chars().count() > MAX_CURSOR_CHARS) {
        return Err(PublicError::invalid_input(
            "page_cursor is too long",
            "Use next_cursor exactly as returned by the previous response",
        ));
    }
    Ok((page_size, cursor))
}

fn page_query(fields: &str, page_size: u16, cursor: Option<&str>) -> Vec<(String, String)> {
    let mut query = vec![
        ("fields".to_owned(), fields.to_owned()),
        ("limit".to_owned(), page_size.to_string()),
    ];
    if let Some(cursor) = cursor {
        query.push(("after".to_owned(), cursor.to_owned()));
    }
    query
}

fn page_query_without_fields(page_size: u16, cursor: Option<&str>) -> Vec<(String, String)> {
    let mut query = vec![("limit".to_owned(), page_size.to_string())];
    if let Some(cursor) = cursor {
        query.push(("after".to_owned(), cursor.to_owned()));
    }
    query
}

fn check_page_len(actual: usize, requested: u16, item: &str) -> Result<(), PublicError> {
    if actual > usize::from(requested) {
        return Err(PublicError::invalid_upstream(format!(
            "Meta returned more {item} than requested"
        )));
    }
    Ok(())
}

fn next_cursor(paging: Option<RawPaging>) -> Result<Option<String>, PublicError> {
    let cursor = paging
        .and_then(|paging| paging.cursors)
        .and_then(|cursors| cursors.after)
        .filter(|cursor| !cursor.is_empty());
    if cursor
        .as_ref()
        .is_some_and(|cursor| cursor.chars().count() > MAX_CURSOR_CHARS)
    {
        return Err(PublicError::invalid_upstream(
            "Meta returned an oversized pagination cursor",
        ));
    }
    Ok(cursor)
}

fn normalize_username(raw: &str) -> Option<String> {
    let value = raw.trim().strip_prefix('@').unwrap_or(raw.trim());
    if value.is_empty()
        || value.chars().count() > MAX_USERNAME_CHARS
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_'))
    {
        return None;
    }
    Some(value.to_owned())
}

fn bounded_text(value: Option<String>, max_chars: usize) -> (Option<String>, bool) {
    let Some(value) = value else {
        return (None, false);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return (None, false);
    }
    let mut chars = trimmed.chars();
    let mut output = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        output.pop();
        output.push('…');
        (Some(output), true)
    } else {
        (Some(output), false)
    }
}

fn bounded_numbers(values: Option<Vec<i64>>) -> (Option<Vec<i64>>, bool) {
    let Some(mut values) = values else {
        return (None, false);
    };
    let truncated = values.len() > MAX_CURVE_POINTS;
    values.truncate(MAX_CURVE_POINTS);
    ((!values.is_empty()).then_some(values), truncated)
}

fn numeric_value_string(value: Option<Value>) -> Option<String> {
    let value = scalar_value_string(value)?;
    normalize_numeric_id(&value)
}

fn scalar_value_string(value: Option<Value>) -> Option<String> {
    bounded_scalar_value_string(value, MAX_METADATA_CHARS).0
}

fn bounded_scalar_value_string(value: Option<Value>, max_chars: usize) -> (Option<String>, bool) {
    let value = match value {
        Some(Value::String(value)) => value,
        Some(Value::Number(value)) => value.to_string(),
        _ => return (None, false),
    };
    bounded_text(Some(value), max_chars)
}

fn value_u64(value: Option<Value>) -> Option<u64> {
    match value? {
        Value::Number(value) => value.as_u64(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

fn value_i64(value: Option<Value>) -> Option<i64> {
    match value? {
        Value::Number(value) => value.as_i64(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

const fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        DerivedMetricScope, GetThreadsAccountInput, ListAdCustomDerivedMetricsInput,
        MAX_CURVE_POINTS, ThreadsAccountMode, build_threads_request, normalize_ad_account_id,
        normalize_prediction, normalize_username, parse_branded_content_permission_page,
        parse_derived_metric_page, parse_reach_frequency_page, parse_recommendation_page,
        parse_threads_edge, validate_page,
    };

    #[test]
    fn validates_ids_pages_usernames_and_v26_business_scope() {
        assert_eq!(normalize_ad_account_id("123"), Some("act_123".to_owned()));
        assert_eq!(
            normalize_ad_account_id("act_123"),
            Some("act_123".to_owned())
        );
        assert_eq!(normalize_ad_account_id("../123"), None);
        assert_eq!(
            normalize_username(" @creator.name "),
            Some("creator.name".to_owned())
        );
        assert_eq!(normalize_username("bad/name"), None);
        assert!(validate_page(Some(100), Some("cursor")).is_ok());
        assert!(validate_page(Some(0), None).is_err());
        let oversized_cursor = "x".repeat(2_049);
        assert!(validate_page(Some(25), Some(&oversized_cursor)).is_err());

        let input = ListAdCustomDerivedMetricsInput {
            business_id: "42".to_owned(),
            scope: Some(DerivedMetricScope::Business),
            page_size: Some(25),
            page_cursor: None,
        };
        assert_eq!(input.business_id, "42");
        assert_eq!(input.scope.unwrap().as_str(), "BUSINESS");
    }

    #[test]
    fn parses_compact_derived_metrics_and_marks_truncation() {
        let payload = json!({
            "data": [{
                "id": "11",
                "ad_account_id": "123",
                "name": "qualified_roas",
                "formula": "x".repeat(2_100),
                "scope": "ACCOUNT",
                "has_attribution_windows": true
            }],
            "paging": {"cursors": {"after": "next"}}
        });
        let page = parse_derived_metric_page(payload, "9".to_owned(), 25).unwrap();
        assert_eq!(page.business_id, "9");
        assert_eq!(page.next_cursor.as_deref(), Some("next"));
        assert_eq!(page.metrics[0].ad_account_id.as_deref(), Some("act_123"));
        assert!(page.metrics[0].content_truncated);
    }

    #[test]
    fn recommendations_follow_account_edge_shape_and_reject_drift() {
        let payload = json!({
            "data": [{"recommendations": [{
                "type": "EXAMPLE_RECOMMENDATION",
                "object_ids": ["101", "202"],
                "recommendation_content": {
                    "body": "Review this account setting",
                    "lift_estimate": "1.5",
                    "opportunity_score_lift": "2"
                },
                "recommendation_stage": "ACTIVE",
                "recommendation_time": "2026-08-28T00:00:00+0000",
                "recommendation_signature": "redacted",
                "url": "https://example.invalid/private"
            }]}]
        });
        let page = parse_recommendation_page(payload, 25).unwrap();
        assert_eq!(
            serde_json::to_value(page).unwrap(),
            json!({"recommendations": [{
                "recommendation_type": "EXAMPLE_RECOMMENDATION",
                "object_ids": ["101", "202"],
                "recommendation_stage": "ACTIVE",
                "recommendation_time": "2026-08-28T00:00:00+0000",
                "body": "Review this account setting",
                "lift_estimate": "1.5",
                "opportunity_score_lift": "2"
            }]})
        );

        let drifted = json!({"data": [{"recommendations": [{"legacy": "shape"}]}]});
        assert!(parse_recommendation_page(drifted, 25).is_err());
    }

    #[test]
    fn parses_current_partnership_fields_with_v26_sdk_fallbacks() {
        let payload = json!({
            "data": [{
                "id": "5",
                "creator_username": "creator",
                "creator_id": "6",
                "creator_fb_page": "7",
                "brand_ig_user": {"id": "8"},
                "permission_status": "APPROVED"
            }, {
                "id": "9",
                "status": "PENDING",
                "permission_type": "ACCOUNT_LEVEL"
            }]
        });
        let page = parse_branded_content_permission_page(payload, 25).unwrap();
        assert_eq!(page.permissions[0].status.as_deref(), Some("APPROVED"));
        assert_eq!(
            page.permissions[0].brand_instagram_user_id.as_deref(),
            Some("8")
        );
        assert_eq!(page.permissions[1].status.as_deref(), Some("PENDING"));
    }

    #[test]
    fn list_prediction_is_compact_and_detail_curve_is_bounded() {
        let payload = json!({
            "data": [{
                "id": "101",
                "status": 2,
                "prediction_progress": "75",
                "account_id": 42,
                "target_spec": {"geo_locations": {"countries": ["US"]}},
                "curve_budget_reach": {
                    "budget": (0..=MAX_CURVE_POINTS).map(|value| value as i64).collect::<Vec<_>>(),
                    "reach": [100, 200],
                    "num_points": 65
                }
            }]
        });
        let detail =
            normalize_prediction(serde_json::from_value(payload["data"][0].clone()).unwrap())
                .unwrap();
        let curve = detail.curve_budget_reach.as_ref().unwrap();
        assert_eq!(curve.budgets.as_ref().unwrap().len(), MAX_CURVE_POINTS);
        assert!(curve.truncated);

        let page = parse_reach_frequency_page(payload, 25).unwrap();
        let prediction = &page.predictions[0];
        assert_eq!(prediction.prediction_progress, Some(75));
        let serialized = serde_json::to_string(prediction).unwrap();
        assert!(!serialized.contains("target_spec"));
        assert!(!serialized.contains("curve_budget_reach"));
        assert!(!serialized.contains("account_id"));
    }

    #[test]
    fn builds_each_documented_threads_read_without_guessing_edges() {
        let instagram = GetThreadsAccountInput {
            instagram_business_account_id: Some("123".to_owned()),
            facebook_page_id: None,
            mode: Some(ThreadsAccountMode::Associated),
        };
        let associated = build_threads_request(&instagram, ThreadsAccountMode::Associated).unwrap();
        assert_eq!(associated.0, "123/connected_threads_user");
        let backed =
            build_threads_request(&instagram, ThreadsAccountMode::InstagramBacked).unwrap();
        assert_eq!(backed.0, "123/instagram_backed_threads_user");

        let page = GetThreadsAccountInput {
            instagram_business_account_id: None,
            facebook_page_id: Some("456".to_owned()),
            mode: Some(ThreadsAccountMode::PageBacked),
        };
        let page_backed = build_threads_request(&page, ThreadsAccountMode::PageBacked).unwrap();
        assert_eq!(page_backed.0, "456");
        assert_eq!(page_backed.1[0].1, "page_backed_threads_account_id");

        let parsed = parse_threads_edge(
            json!({"data": [{"threads_user_id": "789"}]}),
            ThreadsAccountMode::Associated,
        )
        .unwrap();
        assert_eq!(parsed.threads_user_id.as_deref(), Some("789"));
    }
}
