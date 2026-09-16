use std::collections::{BTreeMap, BTreeSet};

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::numeric as normalize_digits,
    mutation_result::{
        ambiguous_mutation_result as shared_ambiguous_mutation_result,
        mutation_error_without_blind_retry,
    },
};

const DEFAULT_FIELDS: &str = "account_id,account_name,campaign_id,campaign_name,adset_id,adset_name,ad_id,ad_name,date_start,date_stop,impressions,clicks,spend,cpc,cpm,ctr,reach,frequency,actions,action_values,conversions,unique_clicks,cost_per_action_type";
const JOB_FIELDS: &str = "id,async_status,async_percent_completion,async_report_url,error_code,error_message,error_subcode,error_user_title,error_user_msg";
const DEFAULT_PAGE_SIZE: u16 = 25;
const DEFAULT_JOB_PAGE_SIZE: u16 = 50;
const MAX_PAGE_SIZE: u16 = 200;
const MAX_CURSOR_CHARS: usize = 2_048;
const MAX_FIELDS: usize = 50;
const MAX_FIELD_CHARS: usize = 128;
const MAX_FIELDS_CHARS: usize = 4_096;
const MAX_BREAKDOWNS: usize = 10;
const MAX_ACTION_STATS: usize = 512;
const MAX_ACTION_KEYS: usize = 64;
const MAX_STATUS_CHARS: usize = 64;
const MAX_REPORT_URL_CHARS: usize = 4_096;

macro_rules! graph_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
        pub enum $name {
            $(#[serde(rename = $value)] $variant),+
        }

        impl $name {
            const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }
        }
    };
}

graph_enum!(InsightLevel {
    Account => "account",
    Campaign => "campaign",
    AdSet => "adset",
    Ad => "ad",
});

graph_enum!(InsightDatePreset {
    DataMaximum => "data_maximum",
    Last14d => "last_14d",
    Last28d => "last_28d",
    Last30d => "last_30d",
    Last3d => "last_3d",
    Last7d => "last_7d",
    Last90d => "last_90d",
    LastMonth => "last_month",
    LastQuarter => "last_quarter",
    LastWeekMonSun => "last_week_mon_sun",
    LastWeekSunSat => "last_week_sun_sat",
    LastYear => "last_year",
    Maximum => "maximum",
    ThisMonth => "this_month",
    ThisQuarter => "this_quarter",
    ThisWeekMonToday => "this_week_mon_today",
    ThisWeekSunToday => "this_week_sun_today",
    ThisYear => "this_year",
    Today => "today",
    Yesterday => "yesterday",
});

// Exact ActionAttributionWindows enum from Meta's v26.0 generated SDK.
graph_enum!(ActionAttributionWindow {
    OneDayClick => "1d_click",
    OneDayEv => "1d_ev",
    OneDaySequenced => "1d_sequenced",
    OneDayView => "1d_view",
    TwentyEightDayClick => "28d_click",
    TwentyEightDaySequenced => "28d_sequenced",
    TwentyEightDayView => "28d_view",
    TwentyEightDayViewAllConversions => "28d_view_all_conversions",
    TwentyEightDayViewFirstConversion => "28d_view_first_conversion",
    SevenDayClick => "7d_click",
    SevenDaySequenced => "7d_sequenced",
    SevenDayView => "7d_view",
    SevenDayViewAllConversions => "7d_view_all_conversions",
    SevenDayViewFirstConversion => "7d_view_first_conversion",
    Custom => "custom",
    Dda => "dda",
    Default => "default",
    Incrementality => "incrementality",
    IncrementalityAllConversions => "incrementality_all_conversions",
    IncrementalityFirstConversion => "incrementality_first_conversion",
    SkanClick => "skan_click",
    SkanClickSecondPostback => "skan_click_second_postback",
    SkanClickThirdPostback => "skan_click_third_postback",
    SkanView => "skan_view",
    SkanViewSecondPostback => "skan_view_second_postback",
    SkanViewThirdPostback => "skan_view_third_postback",
});

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InsightTimeRange {
    /// Inclusive start date in `YYYY-MM-DD` format.
    pub since: String,
    /// Inclusive end date in `YYYY-MM-DD` format.
    pub until: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListInsightsInput {
    /// Numeric account, campaign, ad-set, or ad ID. Account IDs may use `act_`.
    pub object_id: String,
    /// Aggregation level. Defaults to `ad`.
    pub level: Option<InsightLevel>,
    /// Meta-defined relative date window. Defaults to `maximum`.
    pub date_preset: Option<InsightDatePreset>,
    /// Inclusive calendar dates; mutually exclusive with `date_preset`.
    pub time_range: Option<InsightTimeRange>,
    /// Time buckets: `1` through `90` days, `monthly`, or `all_days`.
    pub time_increment: Option<String>,
    /// Insight fields. Defaults to a compact performance bundle; maximum 50.
    pub fields: Option<Vec<String>>,
    /// Dimension breakdown names; maximum 10.
    pub breakdowns: Option<Vec<String>>,
    /// Action breakdown names; maximum 10.
    pub action_breakdowns: Option<Vec<String>>,
    /// Summary action breakdown names; maximum 10.
    pub summary_action_breakdowns: Option<Vec<String>>,
    /// Meta v26.0 attribution windows. Values remain separate in each row.
    pub action_attribution_windows: Option<Vec<ActionAttributionWindow>>,
    /// Optional filters, sorting, attribution settings, summaries and multiple date ranges.
    pub options: Option<InsightOptions>,
    /// Rows to return, from 1 through 200. Defaults to 25; prefer 25-50 at ad level to keep output model-friendly.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

/// Start one asynchronous Insights query. A repeated call can create another report run.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInsightsJobInput {
    /// Numeric account, campaign, ad-set, or ad ID. Account IDs may use `act_`.
    pub object_id: String,
    /// Aggregation level. Defaults to `ad`.
    pub level: Option<InsightLevel>,
    /// Meta-defined relative date window. Defaults to `maximum`.
    pub date_preset: Option<InsightDatePreset>,
    /// Inclusive calendar dates; mutually exclusive with `date_preset`.
    pub time_range: Option<InsightTimeRange>,
    /// Time buckets: `1` through `90` days, `monthly`, or `all_days`.
    pub time_increment: Option<String>,
    /// Insight fields. Defaults to a compact performance bundle; maximum 50.
    pub fields: Option<Vec<String>>,
    /// Dimension breakdown names; maximum 10.
    pub breakdowns: Option<Vec<String>>,
    /// Action breakdown names; maximum 10.
    pub action_breakdowns: Option<Vec<String>>,
    /// Summary action breakdown names; maximum 10.
    pub summary_action_breakdowns: Option<Vec<String>>,
    /// Meta v26.0 attribution windows. Values remain separate in each row.
    pub action_attribution_windows: Option<Vec<ActionAttributionWindow>>,
    pub options: Option<InsightOptions>,
    /// Optional asynchronous export format: CSV or XLS.
    pub export_format: Option<InsightsExportFormat>,
    #[schemars(length(min = 1, max = 50))]
    pub export_columns: Option<Vec<String>>,
    #[schemars(length(min = 1, max = 255))]
    pub export_name: Option<String>,
}

graph_enum!(ActionReportTime {
    Impression => "impression",
    Conversion => "conversion",
    Mixed => "mixed",
    Lifetime => "lifetime",
});

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InsightOptions {
    /// Meta filter objects: field (e.g. campaign.id), operator (e.g. IN), value.
    #[schemars(length(min = 1, max = 25))]
    pub filtering: Option<Vec<InsightFilter>>,
    /// One sort, e.g. spend_descending or actions:link_click_ascending.
    #[schemars(length(min = 1, max = 1), inner(length(min = 1, max = 128)))]
    pub sort: Option<Vec<String>>,
    pub action_report_time: Option<ActionReportTime>,
    pub use_account_attribution_setting: Option<bool>,
    pub use_unified_attribution_setting: Option<bool>,
    /// Overrides single-date selection; cannot be combined with time_increment.
    #[schemars(length(min = 1, max = 25))]
    pub time_ranges: Option<Vec<InsightTimeRange>>,
    pub default_summary: Option<bool>,
    #[schemars(length(min = 1, max = 50))]
    pub summary: Option<Vec<String>>,
    #[schemars(range(min = 1, max = 100))]
    pub product_id_limit: Option<u16>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InsightFilter {
    #[schemars(length(min = 1, max = 128))]
    pub field: String,
    pub operator: InsightFilterOperator,
    /// Scalar or bounded array of values, according to the chosen operator.
    pub value: Value,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InsightFilterOperator {
    Equal,
    NotEqual,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    InRange,
    NotInRange,
    Contain,
    NotContain,
    ContainsAny,
    ContainsAll,
    NotContainsAny,
    StemMatch,
    In,
    NotIn,
    StartsWith,
    EndsWith,
    Any,
    All,
    After,
    Before,
    OnOrAfter,
    OnOrBefore,
    None,
    Top,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum InsightsExportFormat {
    Csv,
    Xls,
}

impl InsightsExportFormat {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Xls => "xls",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadInsightsJobInput {
    /// Numeric async report-run ID.
    pub report_run_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadInsightsJobResultsInput {
    /// Numeric async report-run ID.
    pub report_run_id: String,
    /// Rows to return, from 1 through 200. Defaults to 50.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct InsightPage {
    pub rows: Vec<InsightRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<InsightRow>,
}

/// One Meta-reported row. No metric, including non-additive `reach`, is aggregated.
#[derive(Debug, Serialize, JsonSchema)]
pub struct InsightRow {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub campaign_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub campaign_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adset_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adset_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ad_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ad_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_stop: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impressions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clicks: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ctr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reach: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unique_clicks: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<ActionMetric>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_values: Option<Vec<ActionMetric>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversions: Option<Vec<ActionMetric>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_per_action_type: Option<Vec<ActionMetric>>,
    /// Requested custom metrics and breakdown dimensions.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ActionMetric {
    pub action_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Attribution buckets and action-breakdown values reported by Meta.
    #[serde(flatten)]
    pub detail: BTreeMap<String, Value>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct InsightsJob {
    pub report_run_id: String,
    /// Meta's current async status string, preserved for forward compatibility.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent_complete: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<InsightsJobError>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CreatedInsightsJob {
    pub report_run_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct InsightsJobError {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subcode: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_message: Option<String>,
}

#[derive(Debug)]
struct ValidatedPageRequest {
    endpoint: String,
    query: Vec<(String, String)>,
    page_size: u16,
}

#[derive(Debug, PartialEq, Eq)]
struct ValidatedMutationRequest {
    endpoint: String,
    form: Vec<(String, String)>,
}

struct InsightQuery<'a> {
    object_id: &'a str,
    level: Option<InsightLevel>,
    date_preset: Option<InsightDatePreset>,
    time_range: Option<&'a InsightTimeRange>,
    time_increment: Option<&'a str>,
    fields: Option<&'a [String]>,
    breakdowns: Option<&'a [String]>,
    action_breakdowns: Option<&'a [String]>,
    summary_action_breakdowns: Option<&'a [String]>,
    action_attribution_windows: Option<&'a [ActionAttributionWindow]>,
    options: Option<&'a InsightOptions>,
}

#[derive(Debug, Deserialize)]
struct RawInsightPage {
    data: Vec<Map<String, Value>>,
    paging: Option<RawPaging>,
    summary: Option<Map<String, Value>>,
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
struct RawInsightsJob {
    id: Option<Value>,
    async_status: Option<String>,
    async_percent_completion: Option<Value>,
    async_report_url: Option<String>,
    error_code: Option<Value>,
    error_message: Option<String>,
    error_subcode: Option<Value>,
    error_user_title: Option<String>,
    error_user_msg: Option<String>,
}

pub(crate) async fn list_insights(
    graph: &GraphClient,
    input: ListInsightsInput,
) -> ToolResponse<InsightPage> {
    let request = match build_list_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.get_json(&request.endpoint, &request.query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    match normalize_page(payload, usize::from(request.page_size)) {
        Ok(page) => ToolResponse::success(page),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn create_insights_job(
    graph: &GraphClient,
    input: CreateInsightsJobInput,
) -> ToolResponse<CreatedInsightsJob> {
    let request = match build_create_job_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(mutation_error_without_blind_retry(
                error,
                "Check Meta Ads Manager for the report run before trying again",
            ));
        }
    };
    match normalize_created_job(&payload) {
        Ok(job) => ToolResponse::success(job),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn read_insights_job(
    graph: &GraphClient,
    input: ReadInsightsJobInput,
) -> ToolResponse<InsightsJob> {
    let Some(report_run_id) = normalize_report_run_id(&input.report_run_id) else {
        return ToolResponse::error(PublicError::invalid_input(
            "report_run_id must be a numeric Meta report-run ID",
            "Use the report_run_id returned when the async job was created",
        ));
    };
    let query = vec![("fields".to_owned(), JOB_FIELDS.to_owned())];
    let payload = match graph.get_json(&report_run_id, &query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    let raw = match serde_json::from_value::<RawInsightsJob>(payload) {
        Ok(raw) => raw,
        Err(_) => {
            return ToolResponse::error(PublicError::invalid_upstream(
                "Meta returned unexpected async-insights job metadata",
            ));
        }
    };
    match normalize_job(raw, report_run_id) {
        Ok(job) => ToolResponse::success(job),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn read_insights_job_results(
    graph: &GraphClient,
    input: ReadInsightsJobResultsInput,
) -> ToolResponse<InsightPage> {
    let request = match build_job_results_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.get_json(&request.endpoint, &request.query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    match normalize_page(payload, usize::from(request.page_size)) {
        Ok(page) => ToolResponse::success(page),
        Err(error) => ToolResponse::error(error),
    }
}

fn build_list_request(input: &ListInsightsInput) -> Result<ValidatedPageRequest, PublicError> {
    let (endpoint, mut query) = build_insight_query(InsightQuery {
        object_id: &input.object_id,
        level: input.level,
        date_preset: input.date_preset,
        time_range: input.time_range.as_ref(),
        time_increment: input.time_increment.as_deref(),
        fields: input.fields.as_deref(),
        breakdowns: input.breakdowns.as_deref(),
        action_breakdowns: input.action_breakdowns.as_deref(),
        summary_action_breakdowns: input.summary_action_breakdowns.as_deref(),
        action_attribution_windows: input.action_attribution_windows.as_deref(),
        options: input.options.as_ref(),
    })?;
    let page_size = validate_page_size(input.page_size, DEFAULT_PAGE_SIZE)?;
    let cursor = validate_cursor(input.page_cursor.as_deref())?;
    query.push(("limit".to_owned(), page_size.to_string()));
    if let Some(cursor) = cursor {
        query.push(("after".to_owned(), cursor.to_owned()));
    }

    Ok(ValidatedPageRequest {
        endpoint,
        query,
        page_size,
    })
}

fn build_create_job_request(
    input: &CreateInsightsJobInput,
) -> Result<ValidatedMutationRequest, PublicError> {
    let (endpoint, mut form) = build_insight_query(InsightQuery {
        object_id: &input.object_id,
        level: input.level,
        date_preset: input.date_preset,
        time_range: input.time_range.as_ref(),
        time_increment: input.time_increment.as_deref(),
        fields: input.fields.as_deref(),
        breakdowns: input.breakdowns.as_deref(),
        action_breakdowns: input.action_breakdowns.as_deref(),
        summary_action_breakdowns: input.summary_action_breakdowns.as_deref(),
        action_attribution_windows: input.action_attribution_windows.as_deref(),
        options: input.options.as_ref(),
    })?;
    if let Some(export_format) = input.export_format {
        form.push((
            "export_format".to_owned(),
            export_format.as_str().to_owned(),
        ));
    }
    if let Some(columns) = &input.export_columns {
        normalize_fields(Some(columns))?;
        form.push((
            "export_columns".to_owned(),
            crate::graph_tools::json(&serde_json::json!(columns), "export_columns")?,
        ));
    }
    if let Some(name) = &input.export_name {
        form.push((
            "export_name".to_owned(),
            crate::graph_tools::text(name, "export_name", 255)?,
        ));
    }
    Ok(ValidatedMutationRequest { endpoint, form })
}

fn build_insight_query(
    input: InsightQuery<'_>,
) -> Result<(String, Vec<(String, String)>), PublicError> {
    let object_id = normalize_object_id(input.object_id).ok_or_else(|| {
        PublicError::invalid_input(
            "object_id must be a numeric Meta object ID",
            "Use a numeric account, campaign, ad-set, or ad ID; account IDs may use act_",
        )
    })?;
    let fields = normalize_fields(input.fields)?;
    let ranges = input
        .options
        .and_then(|options| options.time_ranges.as_ref());
    if ranges.is_some()
        && (input.time_range.is_some()
            || input.date_preset.is_some()
            || input.time_increment.is_some())
    {
        return Err(PublicError::invalid_input(
            "time_ranges cannot be combined with another date selector or time_increment",
            "Choose one date selection",
        ));
    }
    if input.date_preset.is_some() && input.time_range.is_some() {
        return Err(PublicError::invalid_input(
            "date_preset and time_range are mutually exclusive",
            "Choose one date selector",
        ));
    }
    if let Some(range) = input.time_range {
        validate_time_range(range)?;
    }

    let mut params = vec![
        ("fields".to_owned(), fields),
        (
            "level".to_owned(),
            input.level.unwrap_or(InsightLevel::Ad).as_str().to_owned(),
        ),
    ];
    if let Some(range) = input.time_range {
        params.push((
            "time_range".to_owned(),
            serde_json::to_string(range).map_err(|_| {
                PublicError::invalid_input(
                    "time_range could not be encoded",
                    "Use plain YYYY-MM-DD dates",
                )
            })?,
        ));
    } else if ranges.is_none() {
        params.push((
            "date_preset".to_owned(),
            input
                .date_preset
                .unwrap_or(InsightDatePreset::Maximum)
                .as_str()
                .to_owned(),
        ));
    }
    if let Some(time_increment) = normalize_time_increment(input.time_increment)? {
        params.push(("time_increment".to_owned(), time_increment));
    }
    append_string_list(&mut params, "breakdowns", input.breakdowns, MAX_BREAKDOWNS)?;
    append_string_list(
        &mut params,
        "action_breakdowns",
        input.action_breakdowns,
        MAX_BREAKDOWNS,
    )?;
    append_string_list(
        &mut params,
        "summary_action_breakdowns",
        input.summary_action_breakdowns,
        MAX_BREAKDOWNS,
    )?;
    append_attribution_windows(&mut params, input.action_attribution_windows)?;
    if let Some(options) = input.options {
        append_options(&mut params, options)?;
    }
    Ok((format!("{object_id}/insights"), params))
}

fn append_options(
    params: &mut Vec<(String, String)>,
    options: &InsightOptions,
) -> Result<(), PublicError> {
    use crate::graph_tools::{json, text};
    let invalid =
        |message| PublicError::invalid_input(message, "Use bounded documented Insights options");
    if let Some(filters) = &options.filtering {
        if filters.is_empty() || filters.len() > 25 {
            return Err(invalid("filtering must contain 1–25 filters"));
        }
        for filter in filters {
            if filter.field.is_empty()
                || filter.field.len() > 128
                || !filter
                    .field
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.'))
                || crate::bounded_json::credential_key(&filter.field)
                || filter.value.is_null()
                || filter.value.is_object()
                || filter.value.as_array().is_some_and(|items| {
                    items
                        .iter()
                        .any(|v| v.is_null() || v.is_array() || v.is_object())
                })
            {
                return Err(invalid(
                    "Each filter needs a simple field and scalar or scalar-array value",
                ));
            }
        }
        params.push((
            "filtering".into(),
            json(&serde_json::json!(filters), "filtering")?,
        ));
    }
    if let Some(sort) = &options.sort {
        if sort.len() != 1
            || sort[0].is_empty()
            || sort[0].len() > 128
            || !sort[0]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':'))
        {
            return Err(invalid(
                "sort accepts one metric or actions:<type>, optionally ending _ascending or _descending",
            ));
        }
        params.push(("sort".into(), json(&serde_json::json!(sort), "sort")?));
    }
    if let Some(report_time) = options.action_report_time {
        params.push(("action_report_time".into(), report_time.as_str().into()));
    }
    for (key, value) in [
        (
            "use_account_attribution_setting",
            options.use_account_attribution_setting,
        ),
        (
            "use_unified_attribution_setting",
            options.use_unified_attribution_setting,
        ),
        ("default_summary", options.default_summary),
    ] {
        if let Some(value) = value {
            params.push((key.into(), value.to_string()));
        }
    }
    if let Some(ranges) = &options.time_ranges {
        if ranges.is_empty() || ranges.len() > 25 {
            return Err(invalid("time_ranges must contain 1–25 date ranges"));
        }
        for range in ranges {
            validate_time_range(range)?;
        }
        params.push((
            "time_ranges".into(),
            json(&serde_json::json!(ranges), "time_ranges")?,
        ));
    }
    if let Some(summary) = &options.summary {
        normalize_fields(Some(summary))?;
        params.push((
            "summary".into(),
            json(&serde_json::json!(summary), "summary")?,
        ));
    }
    if let Some(limit) = options.product_id_limit {
        if !(1..=100).contains(&limit) {
            return Err(invalid("product_id_limit must be 1–100"));
        }
        params.push((
            "product_id_limit".into(),
            text(&limit.to_string(), "product_id_limit", 3)?,
        ));
    }
    Ok(())
}

fn build_job_results_request(
    input: &ReadInsightsJobResultsInput,
) -> Result<ValidatedPageRequest, PublicError> {
    let report_run_id = normalize_report_run_id(&input.report_run_id).ok_or_else(|| {
        PublicError::invalid_input(
            "report_run_id must be a numeric Meta report-run ID",
            "Use the report_run_id returned when the async job was created",
        )
    })?;
    let page_size = validate_page_size(input.page_size, DEFAULT_JOB_PAGE_SIZE)?;
    let cursor = validate_cursor(input.page_cursor.as_deref())?;
    let mut query = vec![("limit".to_owned(), page_size.to_string())];
    if let Some(cursor) = cursor {
        query.push(("after".to_owned(), cursor.to_owned()));
    }
    Ok(ValidatedPageRequest {
        endpoint: format!("{report_run_id}/insights"),
        query,
        page_size,
    })
}

fn validate_page_size(page_size: Option<u16>, default: u16) -> Result<u16, PublicError> {
    let page_size = page_size.unwrap_or(default);
    if !(1..=MAX_PAGE_SIZE).contains(&page_size) {
        return Err(PublicError::invalid_input(
            "page_size must be between 1 and 200",
            "Choose a bounded page_size and paginate with next_cursor",
        ));
    }
    Ok(page_size)
}

fn validate_cursor(cursor: Option<&str>) -> Result<Option<&str>, PublicError> {
    let cursor = cursor.filter(|value| !value.is_empty());
    if cursor.is_some_and(|value| value.chars().count() > MAX_CURSOR_CHARS) {
        return Err(PublicError::invalid_input(
            "page_cursor is too long",
            "Use next_cursor exactly as returned by the previous response",
        ));
    }
    Ok(cursor)
}

fn normalize_time_increment(value: Option<&str>) -> Result<Option<String>, PublicError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if matches!(value, "monthly" | "all_days") {
        return Ok(Some(value.to_owned()));
    }
    if value.bytes().all(|byte| byte.is_ascii_digit())
        && let Ok(days) = value.parse::<u8>()
        && (1..=90).contains(&days)
    {
        return Ok(Some(days.to_string()));
    }
    Err(PublicError::invalid_input(
        "time_increment must be 1 through 90, monthly, or all_days",
        "Use \"1\" for daily rows, another day count through \"90\", \"monthly\", or \"all_days\"",
    ))
}

fn normalize_fields(fields: Option<&[String]>) -> Result<String, PublicError> {
    let Some(fields) = fields else {
        return Ok(DEFAULT_FIELDS.to_owned());
    };
    if fields.is_empty() || fields.len() > MAX_FIELDS {
        return Err(PublicError::invalid_input(
            "fields must contain between 1 and 50 names",
            "Request only metrics needed for this analysis",
        ));
    }

    let mut seen = BTreeSet::new();
    let mut normalized = Vec::with_capacity(fields.len());
    let mut total_chars = 0_usize;
    for field in fields {
        let field = field.trim();
        let field_chars = field.chars().count();
        if field_chars == 0 || field_chars > MAX_FIELD_CHARS || !valid_field_name(field) {
            return Err(PublicError::invalid_input(
                "fields contain an invalid metric name",
                "Use plain Meta Insights field names with letters, digits, and underscores",
            ));
        }
        total_chars = total_chars.saturating_add(field_chars);
        if total_chars > MAX_FIELDS_CHARS {
            return Err(PublicError::invalid_input(
                "combined field names are too long",
                "Request fewer metrics",
            ));
        }
        if seen.insert(field.to_owned()) {
            normalized.push(field);
        }
    }
    Ok(normalized.join(","))
}

fn append_string_list(
    query: &mut Vec<(String, String)>,
    name: &str,
    values: Option<&[String]>,
    maximum: usize,
) -> Result<(), PublicError> {
    let Some(values) = values else {
        return Ok(());
    };
    if values.is_empty() || values.len() > maximum {
        return Err(PublicError::invalid_input(
            format!("{name} must contain between 1 and {maximum} names"),
            "Remove unused or duplicate breakdowns",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let value = value.trim();
        if value.is_empty() || value.chars().count() > MAX_FIELD_CHARS || !valid_field_name(value) {
            return Err(PublicError::invalid_input(
                format!("{name} contains an invalid breakdown name"),
                "Use plain Meta Insights breakdown names",
            ));
        }
        if seen.insert(value.to_owned()) {
            normalized.push(value);
        }
    }
    let encoded = serde_json::to_string(&normalized).map_err(|_| {
        PublicError::invalid_input(
            format!("{name} could not be encoded"),
            "Use plain Meta Insights breakdown names",
        )
    })?;
    query.push((name.to_owned(), encoded));
    Ok(())
}

fn append_attribution_windows(
    query: &mut Vec<(String, String)>,
    windows: Option<&[ActionAttributionWindow]>,
) -> Result<(), PublicError> {
    let Some(windows) = windows else {
        return Ok(());
    };
    if windows.is_empty() || windows.len() > 26 {
        return Err(PublicError::invalid_input(
            "action_attribution_windows must contain between 1 and 26 values",
            "Select only attribution windows needed for this analysis",
        ));
    }
    let values = windows
        .iter()
        .map(|window| window.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let encoded = serde_json::to_string(&values).map_err(|_| {
        PublicError::invalid_input(
            "action_attribution_windows could not be encoded",
            "Use values from the v26.0 attribution-window enum",
        )
    })?;
    query.push(("action_attribution_windows".to_owned(), encoded));
    Ok(())
}

fn normalize_page(payload: Value, max_rows: usize) -> Result<InsightPage, PublicError> {
    let raw = serde_json::from_value::<RawInsightPage>(payload)
        .map_err(|_| PublicError::invalid_upstream("Meta returned an unexpected Insights page"))?;
    if raw.data.len() > max_rows {
        return Err(PublicError::invalid_upstream(
            "Meta returned more Insights rows than requested",
        ));
    }
    let next_cursor = raw
        .paging
        .and_then(|paging| paging.cursors)
        .and_then(|cursors| cursors.after)
        .filter(|cursor| !cursor.is_empty());
    if next_cursor
        .as_ref()
        .is_some_and(|cursor| cursor.chars().count() > MAX_CURSOR_CHARS)
    {
        return Err(PublicError::invalid_upstream(
            "Meta returned an oversized Insights cursor",
        ));
    }
    let rows = raw
        .data
        .into_iter()
        .map(normalize_row)
        .collect::<Result<Vec<_>, _>>()?;
    let summary = raw.summary.map(normalize_row).transpose()?;
    Ok(InsightPage {
        rows,
        next_cursor,
        summary,
    })
}

fn normalize_row(mut raw: Map<String, Value>) -> Result<InsightRow, PublicError> {
    let row = InsightRow {
        account_id: take_scalar(&mut raw, "account_id")?,
        account_name: take_scalar(&mut raw, "account_name")?,
        campaign_id: take_scalar(&mut raw, "campaign_id")?,
        campaign_name: take_scalar(&mut raw, "campaign_name")?,
        adset_id: take_scalar(&mut raw, "adset_id")?,
        adset_name: take_scalar(&mut raw, "adset_name")?,
        ad_id: take_scalar(&mut raw, "ad_id")?,
        ad_name: take_scalar(&mut raw, "ad_name")?,
        date_start: take_scalar(&mut raw, "date_start")?,
        date_stop: take_scalar(&mut raw, "date_stop")?,
        impressions: take_scalar(&mut raw, "impressions")?,
        clicks: take_scalar(&mut raw, "clicks")?,
        spend: take_scalar(&mut raw, "spend")?,
        cpc: take_scalar(&mut raw, "cpc")?,
        cpm: take_scalar(&mut raw, "cpm")?,
        ctr: take_scalar(&mut raw, "ctr")?,
        reach: take_scalar(&mut raw, "reach")?,
        frequency: take_scalar(&mut raw, "frequency")?,
        unique_clicks: take_scalar(&mut raw, "unique_clicks")?,
        actions: take_action_metrics(&mut raw, "actions")?,
        action_values: take_action_metrics(&mut raw, "action_values")?,
        conversions: take_action_metrics(&mut raw, "conversions")?,
        cost_per_action_type: take_action_metrics(&mut raw, "cost_per_action_type")?,
        extra: raw
            .into_iter()
            .filter(|(_, value)| !value.is_null())
            .collect(),
    };
    Ok(row)
}

fn take_scalar(raw: &mut Map<String, Value>, name: &str) -> Result<Option<String>, PublicError> {
    let Some(value) = raw.remove(name) else {
        return Ok(None);
    };
    match value {
        Value::Null => Ok(None),
        Value::String(value) => Ok(nonempty(value)),
        Value::Number(value) => Ok(Some(value.to_string())),
        _ => Err(PublicError::invalid_upstream(format!(
            "Meta returned a non-scalar {name} value"
        ))),
    }
}

fn take_action_metrics(
    raw: &mut Map<String, Value>,
    name: &str,
) -> Result<Option<Vec<ActionMetric>>, PublicError> {
    let Some(value) = raw.remove(name) else {
        return Ok(None);
    };
    let values = match value {
        Value::Null => return Ok(None),
        Value::Array(values) => values,
        _ => {
            return Err(PublicError::invalid_upstream(format!(
                "Meta returned a non-list {name} value"
            )));
        }
    };
    if values.len() > MAX_ACTION_STATS {
        return Err(PublicError::invalid_upstream(format!(
            "Meta returned too many {name} entries"
        )));
    }
    let mut metrics = Vec::with_capacity(values.len());
    for value in values {
        let Value::Object(mut value) = value else {
            return Err(PublicError::invalid_upstream(format!(
                "Meta returned an invalid {name} entry"
            )));
        };
        if value.len() > MAX_ACTION_KEYS {
            return Err(PublicError::invalid_upstream(format!(
                "Meta returned an oversized {name} entry"
            )));
        }
        let action_type = value
            .remove("action_type")
            .and_then(value_to_string)
            .and_then(nonempty)
            .filter(|value| value.chars().count() <= MAX_FIELD_CHARS)
            .ok_or_else(|| {
                PublicError::invalid_upstream(format!("Meta omitted a valid action_type in {name}"))
            })?;
        let metric_value = value
            .remove("value")
            .and_then(value_to_string)
            .and_then(nonempty);
        metrics.push(ActionMetric {
            action_type,
            value: metric_value,
            detail: value
                .into_iter()
                .filter(|(_, value)| !value.is_null())
                .collect(),
        });
    }
    Ok((!metrics.is_empty()).then_some(metrics))
}

fn normalize_job(raw: RawInsightsJob, expected_id: String) -> Result<InsightsJob, PublicError> {
    let RawInsightsJob {
        id,
        async_status,
        async_percent_completion,
        async_report_url,
        error_code,
        error_message,
        error_subcode,
        error_user_title,
        error_user_msg,
    } = raw;
    if let Some(returned_id) = id.and_then(value_to_string)
        && normalize_report_run_id(&returned_id).as_deref() != Some(expected_id.as_str())
    {
        return Err(PublicError::invalid_upstream(
            "Meta returned a different async report-run ID",
        ));
    }
    let status = async_status
        .and_then(nonempty)
        .filter(|status| status.chars().count() <= MAX_STATUS_CHARS)
        .ok_or_else(|| PublicError::invalid_upstream("Meta omitted a valid async job status"))?;
    let percent_complete = match async_percent_completion {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value_to_string(value)
                .and_then(|value| value.parse::<u8>().ok())
                .filter(|value| *value <= 100)
                .ok_or_else(|| {
                    PublicError::invalid_upstream(
                        "Meta returned an invalid async completion percentage",
                    )
                })?,
        ),
    };
    let report_url = async_report_url.and_then(nonempty);
    if report_url
        .as_ref()
        .is_some_and(|url| url.chars().count() > MAX_REPORT_URL_CHARS)
    {
        return Err(PublicError::invalid_upstream(
            "Meta returned an oversized async report URL",
        ));
    }
    let error = if status == "Job Completed" {
        None
    } else {
        normalize_job_error(
            error_code,
            error_subcode,
            error_message,
            error_user_title,
            error_user_msg,
        )?
    };
    Ok(InsightsJob {
        report_run_id: expected_id,
        status,
        percent_complete,
        report_url,
        error,
    })
}

fn normalize_created_job(payload: &Value) -> Result<CreatedInsightsJob, PublicError> {
    let report_run_id = payload
        .get("report_run_id")
        .cloned()
        .and_then(value_to_string)
        .and_then(|value| normalize_report_run_id(&value))
        .ok_or_else(|| ambiguous_mutation_result("Meta did not confirm the async report-run ID"))?;
    Ok(CreatedInsightsJob { report_run_id })
}

fn ambiguous_mutation_result(message: impl Into<String>) -> PublicError {
    shared_ambiguous_mutation_result(
        message,
        "Verify the report run in Meta Ads Manager before retrying",
    )
}

fn normalize_job_error(
    error_code: Option<Value>,
    error_subcode: Option<Value>,
    error_message: Option<String>,
    error_user_title: Option<String>,
    error_user_msg: Option<String>,
) -> Result<Option<InsightsJobError>, PublicError> {
    let code = parse_i64(error_code, "error_code")?;
    let subcode = parse_u64(error_subcode, "error_subcode")?;
    let has_error = code.is_some()
        || subcode.is_some()
        || [error_message, error_user_title, error_user_msg]
            .into_iter()
            .flatten()
            .any(|text| !text.trim().is_empty());
    // Async status responses are HTTP 200 but their error prose can still
    // contain private input. Preserve provider codes without echoing that prose.
    Ok(has_error.then_some(InsightsJobError {
        code,
        subcode,
        message: Some(
            "Meta reported an insights job error; check the report in Ads Manager".to_owned(),
        ),
        user_title: None,
        user_message: None,
    }))
}

fn parse_i64(value: Option<Value>, field: &str) -> Result<Option<i64>, PublicError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value_to_string(value)
        .and_then(|value| value.parse::<i64>().ok())
        .map(Some)
        .ok_or_else(|| PublicError::invalid_upstream(format!("Meta returned an invalid {field}")))
}

fn parse_u64(value: Option<Value>, field: &str) -> Result<Option<u64>, PublicError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value_to_string(value)
        .and_then(|value| value.parse::<u64>().ok())
        .map(Some)
        .ok_or_else(|| PublicError::invalid_upstream(format!("Meta returned an invalid {field}")))
}

fn value_to_string(value: Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn nonempty(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn normalize_object_id(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let (prefix, digits) = match raw.strip_prefix("act_") {
        Some(digits) => ("act_", digits),
        None => ("", raw),
    };
    normalize_digits(digits).map(|digits| format!("{prefix}{digits}"))
}

fn normalize_report_run_id(raw: &str) -> Option<String> {
    normalize_digits(raw.trim()).map(str::to_owned)
}

fn valid_field_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn validate_time_range(range: &InsightTimeRange) -> Result<(), PublicError> {
    if !valid_date(&range.since) || !valid_date(&range.until) {
        return Err(PublicError::invalid_input(
            "time_range dates must be valid YYYY-MM-DD values",
            "Use calendar dates such as 2026-08-01",
        ));
    }
    if range.since > range.until {
        return Err(PublicError::invalid_input(
            "time_range.since must not be after time_range.until",
            "Reverse the dates or choose an earlier start date",
        ));
    }
    Ok(())
}

fn valid_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| !matches!(index, 4 | 7) && !byte.is_ascii_digit())
    {
        return false;
    }
    let year = u16::from(bytes[0] - b'0') * 1_000
        + u16::from(bytes[1] - b'0') * 100
        + u16::from(bytes[2] - b'0') * 10
        + u16::from(bytes[3] - b'0');
    let month = (bytes[5] - b'0') * 10 + (bytes[6] - b'0');
    let day = (bytes[8] - b'0') * 10 + (bytes[9] - b'0');
    let leap_year =
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap_year => 29,
        2 => 28,
        _ => return false,
    };
    (1..=max_day).contains(&day)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{
        ActionAttributionWindow, CreateInsightsJobInput, InsightDatePreset, InsightLevel,
        InsightTimeRange, InsightsExportFormat, ListInsightsInput, MAX_CURSOR_CHARS, MAX_FIELDS,
        ReadInsightsJobResultsInput, build_create_job_request, build_job_results_request,
        build_list_request, normalize_created_job, normalize_job, normalize_page,
    };

    fn base_input() -> ListInsightsInput {
        ListInsightsInput {
            object_id: "act_123".to_owned(),
            level: None,
            date_preset: None,
            time_range: None,
            time_increment: None,
            fields: None,
            breakdowns: None,
            action_breakdowns: None,
            summary_action_breakdowns: None,
            action_attribution_windows: None,
            options: None,
            page_size: None,
            page_cursor: None,
        }
    }

    #[test]
    fn extended_queries_keep_filters_dates_and_summary_without_changing_defaults() {
        let mut input: ListInsightsInput = serde_json::from_value(json!({
            "object_id":"act_123", "options": {
                "filtering":[{"field":"campaign.id","operator":"IN","value":["42"]}],
                "sort":["actions:link_click_ascending"], "action_report_time":"conversion",
                "time_ranges":[{"since":"2026-01-01","until":"2026-01-31"}],
                "use_unified_attribution_setting":true, "summary":["spend"], "product_id_limit":10
            }
        }))
        .unwrap();
        let request = build_list_request(&input).unwrap();
        let params: std::collections::HashMap<_, _> = request.query.into_iter().collect();
        assert!(!params.contains_key("date_preset"));
        assert_eq!(params["sort"], "[\"actions:link_click_ascending\"]");
        assert_eq!(params["action_report_time"], "conversion");
        assert_eq!(params["summary"], "[\"spend\"]");
        let result = normalize_page(
            json!({"data":[],"summary":{"spend":"12.50","reach":"21"}}),
            25,
        )
        .unwrap();
        assert_eq!(result.summary.unwrap().spend.as_deref(), Some("12.50"));
        input.time_increment = Some("monthly".into());
        assert!(build_list_request(&input).is_err());
        input.time_increment = None;
        input.options.as_mut().unwrap().filtering.as_mut().unwrap()[0].value =
            json!("access_token=private");
        assert!(build_list_request(&input).is_err());
    }

    #[test]
    fn accepts_exact_v26_attribution_window_enum() {
        let values = [
            "1d_click",
            "1d_ev",
            "1d_sequenced",
            "1d_view",
            "28d_click",
            "28d_sequenced",
            "28d_view",
            "28d_view_all_conversions",
            "28d_view_first_conversion",
            "7d_click",
            "7d_sequenced",
            "7d_view",
            "7d_view_all_conversions",
            "7d_view_first_conversion",
            "custom",
            "dda",
            "default",
            "incrementality",
            "incrementality_all_conversions",
            "incrementality_first_conversion",
            "skan_click",
            "skan_click_second_postback",
            "skan_click_third_postback",
            "skan_view",
            "skan_view_second_postback",
            "skan_view_third_postback",
        ];
        for value in values {
            let parsed: ActionAttributionWindow =
                serde_json::from_value(Value::String(value.to_owned())).unwrap();
            assert_eq!(serde_json::to_value(parsed).unwrap(), value);
        }
        assert!(serde_json::from_value::<ActionAttributionWindow>(json!("14d_click")).is_err());
    }

    #[test]
    fn builds_typed_bounded_list_request() {
        let input = ListInsightsInput {
            object_id: "act_123".to_owned(),
            level: Some(InsightLevel::Campaign),
            time_range: Some(InsightTimeRange {
                since: "2026-08-01".to_owned(),
                until: "2026-08-19".to_owned(),
            }),
            time_increment: Some("01".to_owned()),
            fields: Some(vec!["campaign_id".to_owned(), "reach".to_owned()]),
            breakdowns: Some(vec!["publisher_platform".to_owned()]),
            action_attribution_windows: Some(vec![
                ActionAttributionWindow::SevenDayView,
                ActionAttributionWindow::TwentyEightDayView,
            ]),
            page_size: Some(40),
            page_cursor: Some("opaque+/= cursor".to_owned()),
            ..base_input()
        };
        let request = build_list_request(&input).unwrap();
        assert_eq!(request.endpoint, "act_123/insights");
        let value = |name: &str| {
            request
                .query
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(value("fields"), Some("campaign_id,reach"));
        assert_eq!(value("level"), Some("campaign"));
        assert_eq!(
            value("time_range"),
            Some("{\"since\":\"2026-08-01\",\"until\":\"2026-08-19\"}")
        );
        assert_eq!(value("breakdowns"), Some("[\"publisher_platform\"]"));
        assert_eq!(value("time_increment"), Some("1"));
        assert_eq!(
            value("action_attribution_windows"),
            Some("[\"28d_view\",\"7d_view\"]")
        );
        assert_eq!(value("after"), Some("opaque+/= cursor"));
        assert!(value("date_preset").is_none());
    }

    #[test]
    fn rejects_unsafe_or_unbounded_list_inputs() {
        let mut input = base_input();
        input.object_id = "../123".to_owned();
        assert!(build_list_request(&input).is_err());

        input = base_input();
        input.page_size = Some(0);
        assert!(build_list_request(&input).is_err());

        input = base_input();
        input.page_cursor = Some("x".repeat(MAX_CURSOR_CHARS + 1));
        assert!(build_list_request(&input).is_err());

        input = base_input();
        input.fields = Some(vec!["reach,spend".to_owned()]);
        assert!(build_list_request(&input).is_err());

        input = base_input();
        input.fields = Some((0..=MAX_FIELDS).map(|index| format!("f{index}")).collect());
        assert!(build_list_request(&input).is_err());

        input = base_input();
        input.time_range = Some(InsightTimeRange {
            since: "2026-02-30".to_owned(),
            until: "2026-03-01".to_owned(),
        });
        assert!(build_list_request(&input).is_err());

        for value in ["", "0", "91", "weekly"] {
            input = base_input();
            input.time_increment = Some(value.to_owned());
            assert!(build_list_request(&input).is_err());
        }
    }

    #[test]
    fn builds_exact_async_job_form_and_requires_confirmed_numeric_id() {
        let request = build_create_job_request(&CreateInsightsJobInput {
            object_id: "act_123".to_owned(),
            level: Some(InsightLevel::Account),
            date_preset: Some(InsightDatePreset::Last30d),
            time_range: None,
            time_increment: Some("monthly".to_owned()),
            fields: Some(vec!["spend".to_owned(), "reach".to_owned()]),
            breakdowns: Some(vec!["mmm".to_owned()]),
            action_breakdowns: None,
            summary_action_breakdowns: None,
            action_attribution_windows: Some(vec![ActionAttributionWindow::SevenDayClick]),
            options: None,
            export_format: Some(InsightsExportFormat::Csv),
            export_columns: None,
            export_name: None,
        })
        .unwrap();
        assert_eq!(request.endpoint, "act_123/insights");
        assert_eq!(
            request.form,
            vec![
                ("fields".to_owned(), "spend,reach".to_owned()),
                ("level".to_owned(), "account".to_owned()),
                ("date_preset".to_owned(), "last_30d".to_owned()),
                ("time_increment".to_owned(), "monthly".to_owned()),
                ("breakdowns".to_owned(), "[\"mmm\"]".to_owned()),
                (
                    "action_attribution_windows".to_owned(),
                    "[\"7d_click\"]".to_owned(),
                ),
                ("export_format".to_owned(), "csv".to_owned()),
            ]
        );
        assert_eq!(
            normalize_created_job(&json!({"report_run_id": "9001"}))
                .unwrap()
                .report_run_id,
            "9001"
        );
        assert!(normalize_created_job(&json!({"success": true})).is_err());
    }

    #[test]
    fn preserves_each_reach_row_and_attribution_bucket() {
        let page = normalize_page(
            json!({
                "data": [
                    {
                        "campaign_id": "1",
                        "reach": "90",
                        "actions": [{
                            "action_type": "purchase",
                            "value": "4",
                            "1d_click": "3",
                            "7d_view": "1"
                        }]
                    },
                    {"campaign_id": "2", "reach": "80"}
                ],
                "paging": {
                    "cursors": {"after": "opaque-next"},
                    "next": "https://graph.facebook.com/next?access_token=secret"
                }
            }),
            2,
        )
        .unwrap();

        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[0].reach.as_deref(), Some("90"));
        assert_eq!(page.rows[1].reach.as_deref(), Some("80"));
        let action = page.rows[0].actions.as_ref().unwrap().first().unwrap();
        assert_eq!(action.detail.get("1d_click"), Some(&json!("3")));
        assert_eq!(action.detail.get("7d_view"), Some(&json!("1")));
        assert_eq!(page.next_cursor.as_deref(), Some("opaque-next"));
        let serialized = serde_json::to_string(&page).unwrap();
        assert!(!serialized.contains("access_token"));
        assert!(!serialized.contains("170"));
    }

    #[test]
    fn compacts_job_state_and_bounds_results_paging() {
        let raw = serde_json::from_value(json!({
            "id": "9001",
            "async_status": "Job Failed",
            "async_percent_completion": 42,
            "error_code": 8001,
            "error_subcode": "99",
            "error_message": "query failed for person@example.test",
            "error_user_title": "private customer record",
            "error_user_msg": "https://example.test/#access_token=private-token"
        }))
        .unwrap();
        let job = normalize_job(raw, "9001".to_owned()).unwrap();
        assert_eq!(job.status, "Job Failed");
        assert_eq!(job.percent_complete, Some(42));
        assert_eq!(job.error.as_ref().and_then(|error| error.code), Some(8001));
        assert_eq!(job.error.as_ref().and_then(|error| error.subcode), Some(99));
        let serialized = serde_json::to_string(&job).unwrap();
        assert!(!serialized.contains("person@example.test"));
        assert!(!serialized.contains("private"));
        assert!(serialized.contains("Ads Manager"));

        let input = ReadInsightsJobResultsInput {
            report_run_id: "9001".to_owned(),
            page_size: Some(100),
            page_cursor: Some("opaque-next".to_owned()),
        };
        let request = build_job_results_request(&input).unwrap();
        assert_eq!(request.endpoint, "9001/insights");
        assert_eq!(request.page_size, 100);
        assert_eq!(request.query[0], ("limit".to_owned(), "100".to_owned()));
        assert_eq!(
            request.query[1],
            ("after".to_owned(), "opaque-next".to_owned())
        );
    }
}
