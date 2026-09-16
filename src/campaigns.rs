use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{ad_account as normalize_ad_account_id, numeric_owned as normalize_numeric_id},
};

const CAMPAIGN_LIST_FIELDS: &str = "id,name,objective,status,effective_status,daily_budget,lifetime_budget,buying_type,start_time,stop_time,created_time,updated_time,bid_strategy,advantage_state_info,special_ad_categories,special_ad_category_country,is_adset_budget_sharing_enabled,spend_cap";
const CAMPAIGN_DETAIL_FIELDS: &str = "id,name,objective,status,effective_status,daily_budget,lifetime_budget,budget_remaining,buying_type,start_time,stop_time,created_time,updated_time,bid_strategy,advantage_state_info,special_ad_categories,special_ad_category_country,is_adset_budget_sharing_enabled,spend_cap,configured_status,smart_promotion_type";
const DEFAULT_PAGE_SIZE: u16 = 10;
const MAX_PAGE_SIZE: u16 = 100;
const MAX_CURSOR_CHARS: usize = 2_048;
const MAX_TEXT_CHARS: usize = 256;
const MAX_STRING_LIST_ITEMS: usize = 32;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListCampaignsInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    pub ad_account_id: String,
    /// Number of campaigns to return, from 1 through 100. Defaults to 10.
    pub page_size: Option<u16>,
    /// Return only campaigns with this effective delivery status.
    pub effective_status: Option<CampaignEffectiveStatus>,
    /// Return campaigns in a Meta-defined relative date window.
    pub date_preset: Option<CampaignDatePreset>,
    /// Return campaigns in this inclusive calendar-date window.
    pub time_range: Option<CampaignTimeRange>,
    /// Filter by Meta's campaign-completion state.
    pub is_completed: Option<bool>,
    /// `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadCampaignInput {
    /// Numeric Meta campaign ID.
    pub campaign_id: String,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CampaignEffectiveStatus {
    Active,
    Archived,
    Deleted,
    InProcess,
    Paused,
    WithIssues,
}

impl CampaignEffectiveStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Archived => "ARCHIVED",
            Self::Deleted => "DELETED",
            Self::InProcess => "IN_PROCESS",
            Self::Paused => "PAUSED",
            Self::WithIssues => "WITH_ISSUES",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CampaignDatePreset {
    DataMaximum,
    #[serde(rename = "last_14d")]
    Last14d,
    #[serde(rename = "last_28d")]
    Last28d,
    #[serde(rename = "last_30d")]
    Last30d,
    #[serde(rename = "last_3d")]
    Last3d,
    #[serde(rename = "last_7d")]
    Last7d,
    #[serde(rename = "last_90d")]
    Last90d,
    LastMonth,
    LastQuarter,
    LastWeekMonSun,
    LastWeekSunSat,
    LastYear,
    Maximum,
    ThisMonth,
    ThisQuarter,
    ThisWeekMonToday,
    ThisWeekSunToday,
    ThisYear,
    Today,
    Yesterday,
}

impl CampaignDatePreset {
    const fn as_str(self) -> &'static str {
        match self {
            Self::DataMaximum => "data_maximum",
            Self::Last14d => "last_14d",
            Self::Last28d => "last_28d",
            Self::Last30d => "last_30d",
            Self::Last3d => "last_3d",
            Self::Last7d => "last_7d",
            Self::Last90d => "last_90d",
            Self::LastMonth => "last_month",
            Self::LastQuarter => "last_quarter",
            Self::LastWeekMonSun => "last_week_mon_sun",
            Self::LastWeekSunSat => "last_week_sun_sat",
            Self::LastYear => "last_year",
            Self::Maximum => "maximum",
            Self::ThisMonth => "this_month",
            Self::ThisQuarter => "this_quarter",
            Self::ThisWeekMonToday => "this_week_mon_today",
            Self::ThisWeekSunToday => "this_week_sun_today",
            Self::ThisYear => "this_year",
            Self::Today => "today",
            Self::Yesterday => "yesterday",
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CampaignTimeRange {
    /// Start date in `YYYY-MM-DD` format.
    pub since: String,
    /// End date in `YYYY-MM-DD` format.
    pub until: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CampaignList {
    pub campaigns: Vec<Campaign>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Campaign metadata. Monetary fields are unconverted account-currency minor units.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Campaign {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configured_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily_budget: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifetime_budget: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_remaining: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spend_cap: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buying_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bid_strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advantage_state_info: Option<AdvantageStateInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub special_ad_categories: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub special_ad_category_country: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_adset_budget_sharing_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smart_promotion_type: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AdvantageStateInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advantage_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advantage_audience_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advantage_budget_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advantage_placement_state: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawCampaignList {
    #[serde(default)]
    data: Vec<RawCampaign>,
    paging: Option<RawPaging>,
}

#[derive(Debug, Deserialize)]
struct RawPaging {
    cursors: Option<RawCursors>,
}

#[derive(Debug, Deserialize)]
struct RawCursors {
    after: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawCampaign {
    id: Option<String>,
    name: Option<String>,
    objective: Option<String>,
    status: Option<String>,
    configured_status: Option<String>,
    effective_status: Option<String>,
    daily_budget: Option<Value>,
    lifetime_budget: Option<Value>,
    budget_remaining: Option<Value>,
    spend_cap: Option<Value>,
    buying_type: Option<String>,
    bid_strategy: Option<String>,
    advantage_state_info: Option<RawAdvantageStateInfo>,
    start_time: Option<String>,
    stop_time: Option<String>,
    created_time: Option<String>,
    updated_time: Option<String>,
    special_ad_categories: Option<Vec<String>>,
    special_ad_category_country: Option<Vec<String>>,
    is_adset_budget_sharing_enabled: Option<bool>,
    smart_promotion_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawAdvantageStateInfo {
    advantage_state: Option<String>,
    advantage_audience_state: Option<String>,
    advantage_budget_state: Option<String>,
    advantage_placement_state: Option<String>,
}

pub(crate) async fn list_campaigns(
    graph: &GraphClient,
    input: ListCampaignsInput,
) -> ToolResponse<CampaignList> {
    let (endpoint, query) = match build_list_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.get_json(&endpoint, &query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    let raw = match serde_json::from_value::<RawCampaignList>(payload) {
        Ok(raw) => raw,
        Err(_) => {
            return ToolResponse::error(PublicError::invalid_upstream(
                "Meta returned an unexpected campaign list",
            ));
        }
    };
    let page_size = input.page_size.unwrap_or(DEFAULT_PAGE_SIZE);
    if raw.data.len() > usize::from(page_size) {
        return ToolResponse::error(PublicError::invalid_upstream(
            "Meta returned more campaigns than requested",
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
        return ToolResponse::error(PublicError::invalid_upstream(
            "Meta returned an oversized campaign cursor",
        ));
    }

    let campaigns = raw
        .data
        .into_iter()
        .filter_map(normalize_campaign)
        .collect();
    ToolResponse::success(CampaignList {
        campaigns,
        next_cursor,
    })
}

pub(crate) async fn read_campaign(
    graph: &GraphClient,
    input: ReadCampaignInput,
) -> ToolResponse<Campaign> {
    let Some(campaign_id) = normalize_numeric_id(&input.campaign_id) else {
        return ToolResponse::error(PublicError::invalid_input(
            "campaign_id must be a numeric Meta campaign ID",
            "Use the ID returned by list_campaigns",
        ));
    };
    let query = vec![("fields".to_owned(), CAMPAIGN_DETAIL_FIELDS.to_owned())];
    let payload = match graph.get_json(&campaign_id, &query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    let raw = match serde_json::from_value::<RawCampaign>(payload) {
        Ok(raw) => raw,
        Err(_) => {
            return ToolResponse::error(PublicError::invalid_upstream(
                "Meta returned unexpected campaign metadata",
            ));
        }
    };
    match normalize_campaign(raw) {
        Some(campaign) => ToolResponse::success(campaign),
        None => ToolResponse::error(PublicError::invalid_upstream(
            "Meta omitted a valid campaign ID",
        )),
    }
}

fn build_list_request(
    input: &ListCampaignsInput,
) -> Result<(String, Vec<(String, String)>), PublicError> {
    let Some(account_id) = normalize_ad_account_id(&input.ad_account_id) else {
        return Err(PublicError::invalid_input(
            "ad_account_id must be a numeric Meta account ID",
            "Use an ID such as `act_123456789` or `123456789`",
        ));
    };
    let page_size = input.page_size.unwrap_or(DEFAULT_PAGE_SIZE);
    if !(1..=MAX_PAGE_SIZE).contains(&page_size) {
        return Err(PublicError::invalid_input(
            "page_size must be between 1 and 100",
            "Choose a bounded page_size and paginate with next_cursor",
        ));
    }

    let cursor = input
        .page_cursor
        .as_deref()
        .filter(|cursor| !cursor.is_empty());
    if cursor.is_some_and(|cursor| cursor.chars().count() > MAX_CURSOR_CHARS) {
        return Err(PublicError::invalid_input(
            "page_cursor is too long",
            "Use next_cursor exactly as returned by the previous response",
        ));
    }
    if input.date_preset.is_some() && input.time_range.is_some() {
        return Err(PublicError::invalid_input(
            "date_preset and time_range are mutually exclusive",
            "Choose one date selector",
        ));
    }
    if let Some(range) = &input.time_range {
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
    }

    let mut query = vec![
        ("fields".to_owned(), CAMPAIGN_LIST_FIELDS.to_owned()),
        ("limit".to_owned(), page_size.to_string()),
    ];
    if let Some(status) = input.effective_status {
        query.push((
            "effective_status".to_owned(),
            format!("[\"{}\"]", status.as_str()),
        ));
    }
    if let Some(preset) = input.date_preset {
        query.push(("date_preset".to_owned(), preset.as_str().to_owned()));
    }
    if let Some(range) = &input.time_range {
        let encoded = serde_json::to_string(range).map_err(|_| {
            PublicError::invalid_input(
                "time_range could not be encoded",
                "Use plain YYYY-MM-DD dates",
            )
        })?;
        query.push(("time_range".to_owned(), encoded));
    }
    if let Some(is_completed) = input.is_completed {
        query.push(("is_completed".to_owned(), is_completed.to_string()));
    }
    if let Some(cursor) = cursor {
        query.push(("after".to_owned(), cursor.to_owned()));
    }

    Ok((format!("{account_id}/campaigns"), query))
}

fn normalize_campaign(raw: RawCampaign) -> Option<Campaign> {
    let id = normalize_numeric_id(raw.id.as_deref()?)?;
    Some(Campaign {
        id,
        name: clean_string(raw.name),
        objective: clean_string(raw.objective),
        status: clean_string(raw.status),
        configured_status: clean_string(raw.configured_status),
        effective_status: clean_string(raw.effective_status),
        daily_budget: normalize_scalar(raw.daily_budget),
        lifetime_budget: normalize_scalar(raw.lifetime_budget),
        budget_remaining: normalize_scalar(raw.budget_remaining),
        spend_cap: normalize_scalar(raw.spend_cap),
        buying_type: clean_string(raw.buying_type),
        bid_strategy: clean_string(raw.bid_strategy),
        advantage_state_info: normalize_advantage_state(raw.advantage_state_info),
        start_time: clean_string(raw.start_time),
        stop_time: clean_string(raw.stop_time),
        created_time: clean_string(raw.created_time),
        updated_time: clean_string(raw.updated_time),
        special_ad_categories: clean_strings(raw.special_ad_categories),
        special_ad_category_country: clean_strings(raw.special_ad_category_country),
        is_adset_budget_sharing_enabled: raw.is_adset_budget_sharing_enabled,
        smart_promotion_type: clean_string(raw.smart_promotion_type),
    })
}

fn normalize_advantage_state(raw: Option<RawAdvantageStateInfo>) -> Option<AdvantageStateInfo> {
    let raw = raw?;
    let value = AdvantageStateInfo {
        advantage_state: clean_string(raw.advantage_state),
        advantage_audience_state: clean_string(raw.advantage_audience_state),
        advantage_budget_state: clean_string(raw.advantage_budget_state),
        advantage_placement_state: clean_string(raw.advantage_placement_state),
    };
    (value.advantage_state.is_some()
        || value.advantage_audience_state.is_some()
        || value.advantage_budget_state.is_some()
        || value.advantage_placement_state.is_some())
    .then_some(value)
}

fn clean_string(value: Option<String>) -> Option<String> {
    let value = value?;
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let mut chars = value.chars();
    let mut output = chars.by_ref().take(MAX_TEXT_CHARS).collect::<String>();
    if chars.next().is_some() {
        output.pop();
        output.push('…');
    }
    Some(output)
}

fn clean_strings(values: Option<Vec<String>>) -> Option<Vec<String>> {
    let values = values?
        .into_iter()
        .take(MAX_STRING_LIST_ITEMS)
        .filter_map(|value| clean_string(Some(value)))
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

fn normalize_scalar(value: Option<Value>) -> Option<String> {
    match value? {
        Value::String(value) => clean_string(Some(value)),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
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
    use serde_json::json;

    use super::{
        CampaignDatePreset, CampaignEffectiveStatus, CampaignTimeRange, ListCampaignsInput,
        RawCampaign, build_list_request, normalize_ad_account_id, normalize_campaign,
        normalize_numeric_id, valid_date,
    };

    #[test]
    fn accepts_only_safe_numeric_graph_ids() {
        assert_eq!(normalize_ad_account_id(" 123 "), Some("act_123".to_owned()));
        assert_eq!(
            normalize_ad_account_id("act_123"),
            Some("act_123".to_owned())
        );
        assert_eq!(normalize_numeric_id("456"), Some("456".to_owned()));
        assert_eq!(normalize_ad_account_id("act_../123"), None);
        assert_eq!(normalize_numeric_id("https://example.test/1"), None);
    }

    #[test]
    fn builds_only_documented_v26_campaign_edge_filters() {
        assert!(matches!(
            serde_json::from_value::<CampaignDatePreset>(json!("last_7d")).unwrap(),
            CampaignDatePreset::Last7d
        ));
        assert!(matches!(
            serde_json::from_value::<CampaignEffectiveStatus>(json!("WITH_ISSUES")).unwrap(),
            CampaignEffectiveStatus::WithIssues
        ));

        let input = ListCampaignsInput {
            ad_account_id: "123".to_owned(),
            page_size: Some(25),
            effective_status: Some(CampaignEffectiveStatus::Active),
            date_preset: Some(CampaignDatePreset::Last7d),
            time_range: None,
            is_completed: Some(false),
            page_cursor: Some("opaque-cursor".to_owned()),
        };

        let (endpoint, query) = build_list_request(&input).unwrap();
        assert_eq!(endpoint, "act_123/campaigns");
        let value = |name: &str| {
            query
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(value("limit"), Some("25"));
        assert_eq!(value("effective_status"), Some("[\"ACTIVE\"]"));
        assert_eq!(value("date_preset"), Some("last_7d"));
        assert_eq!(value("time_range"), None);
        assert_eq!(value("is_completed"), Some("false"));
        assert_eq!(value("after"), Some("opaque-cursor"));
        assert!(value("filtering").is_none());
    }

    #[test]
    fn rejects_invalid_dates_page_sizes_and_cursor_lengths() {
        assert!(valid_date("2024-02-29"));
        assert!(!valid_date("2025-02-29"));
        assert!(!valid_date("2026-13-01"));

        let invalid_page = ListCampaignsInput {
            ad_account_id: "123".to_owned(),
            page_size: Some(0),
            effective_status: None,
            date_preset: None,
            time_range: None,
            is_completed: None,
            page_cursor: None,
        };
        assert!(build_list_request(&invalid_page).is_err());

        let invalid_range = ListCampaignsInput {
            page_size: None,
            time_range: Some(CampaignTimeRange {
                since: "2026-08-20".to_owned(),
                until: "2026-08-19".to_owned(),
            }),
            ..invalid_page
        };
        assert!(build_list_request(&invalid_range).is_err());

        let conflicting_dates = ListCampaignsInput {
            ad_account_id: "123".to_owned(),
            page_size: None,
            effective_status: None,
            date_preset: Some(CampaignDatePreset::Last7d),
            time_range: Some(CampaignTimeRange {
                since: "2026-08-01".to_owned(),
                until: "2026-08-19".to_owned(),
            }),
            is_completed: None,
            page_cursor: None,
        };
        assert!(build_list_request(&conflicting_dates).is_err());

        let oversized_cursor = ListCampaignsInput {
            time_range: None,
            page_cursor: Some("x".repeat(2_049)),
            ..invalid_range
        };
        assert!(build_list_request(&oversized_cursor).is_err());
    }

    #[test]
    fn normalizes_campaign_scalars_without_losing_minor_units() {
        let campaign = normalize_campaign(RawCampaign {
            id: Some("123".to_owned()),
            name: Some(" Launch ".to_owned()),
            daily_budget: Some(json!(500)),
            lifetime_budget: Some(json!("1200")),
            special_ad_categories: Some(vec!["HOUSING".to_owned(), " ".to_owned()]),
            ..RawCampaign::default()
        })
        .unwrap();

        assert_eq!(campaign.name.as_deref(), Some("Launch"));
        assert_eq!(campaign.daily_budget.as_deref(), Some("500"));
        assert_eq!(campaign.lifetime_budget.as_deref(), Some("1200"));
        assert_eq!(
            campaign.special_ad_categories,
            Some(vec!["HOUSING".to_owned()])
        );
    }
}
