use std::collections::BTreeMap;

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{ad_account as normalize_account_id, numeric_owned as normalize_object_id},
};

const AD_SET_SUMMARY_FIELDS: &str = concat!(
    "id,name,campaign_id,status,effective_status,daily_budget,lifetime_budget,",
    "budget_remaining,bid_strategy,optimization_goal,billing_event,start_time,end_time,",
    "is_dynamic_creative,created_time,updated_time"
);
const AD_SET_DETAIL_FIELDS: &str = concat!(
    "targeting,promoted_object,bid_amount,bid_constraints,attribution_spec,destination_type,",
    "pacing_type,dsa_beneficiary,dsa_payor,placement_soft_opt_out,",
    "frequency_control_specs{event,interval_days,max_frequency}"
);
const DEFAULT_PAGE_SIZE: u16 = 10;
const MAX_PAGE_SIZE: u16 = 100;
const MAX_CURSOR_CHARS: usize = 2_048;
const MAX_TEXT_CHARS: usize = 256;
const MAX_COLLECTION_ITEMS: usize = 64;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListAdSetsInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    pub ad_account_id: String,
    /// Numeric campaign ID. When set, list only that campaign's ad sets.
    pub campaign_id: Option<String>,
    /// Number of ad sets to return, from 1 through 100. Defaults to 10.
    pub page_size: Option<u16>,
    /// `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadAdSetInput {
    /// Numeric Meta ad-set ID.
    pub ad_set_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AdSetList {
    pub ad_sets: Vec<AdSet>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AdSet {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub campaign_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily_budget: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifetime_budget: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_remaining: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bid_amount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bid_strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bid_constraints: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimization_goal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_dynamic_creative: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_control_specs: Option<Vec<FrequencyControlSpec>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribution_spec: Option<Vec<AttributionSpec>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pacing_type: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dsa_beneficiary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dsa_payor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placement_soft_opt_out: Option<BTreeMap<String, Vec<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub targeting: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub promoted_object: Option<Value>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FrequencyControlSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_days: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_frequency: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AttributionSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_days: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RawAdSetList {
    #[serde(default)]
    data: Vec<RawAdSet>,
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

#[derive(Debug, Deserialize)]
struct RawAdSet {
    id: Option<String>,
    name: Option<String>,
    campaign_id: Option<String>,
    status: Option<String>,
    effective_status: Option<String>,
    daily_budget: Option<Value>,
    lifetime_budget: Option<Value>,
    budget_remaining: Option<Value>,
    bid_amount: Option<Value>,
    bid_strategy: Option<String>,
    bid_constraints: Option<BTreeMap<String, Value>>,
    optimization_goal: Option<String>,
    billing_event: Option<String>,
    start_time: Option<String>,
    end_time: Option<String>,
    created_time: Option<String>,
    updated_time: Option<String>,
    is_dynamic_creative: Option<bool>,
    frequency_control_specs: Option<Vec<RawFrequencyControlSpec>>,
    attribution_spec: Option<Vec<RawAttributionSpec>>,
    destination_type: Option<String>,
    pacing_type: Option<Vec<String>>,
    dsa_beneficiary: Option<String>,
    dsa_payor: Option<String>,
    placement_soft_opt_out: Option<BTreeMap<String, Vec<String>>>,
    targeting: Option<Value>,
    promoted_object: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct RawFrequencyControlSpec {
    event: Option<String>,
    interval_days: Option<Value>,
    max_frequency: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct RawAttributionSpec {
    event_type: Option<String>,
    window_days: Option<Value>,
}

pub(crate) async fn list_ad_sets(
    graph: &GraphClient,
    input: ListAdSetsInput,
) -> ToolResponse<AdSetList> {
    let Some(account_id) = normalize_account_id(&input.ad_account_id) else {
        return ToolResponse::error(PublicError::invalid_input(
            "ad_account_id must be a numeric Meta account ID",
            "Use an ID such as `act_123456789` or `123456789`",
        ));
    };
    let campaign_id = match normalize_optional_object_id(input.campaign_id.as_deref()) {
        Ok(campaign_id) => campaign_id,
        Err(()) => {
            return ToolResponse::error(PublicError::invalid_input(
                "campaign_id must be a numeric Meta campaign ID",
                "Remove URL, path, and non-numeric characters from campaign_id",
            ));
        }
    };
    let page_size = match normalize_page_size(input.page_size) {
        Ok(page_size) => page_size,
        Err(()) => {
            return ToolResponse::error(PublicError::invalid_input(
                "page_size must be between 1 and 100",
                "Choose a bounded page_size and paginate with next_cursor",
            ));
        }
    };
    let cursor = match normalize_cursor(input.page_cursor.as_deref()) {
        Ok(cursor) => cursor,
        Err(()) => {
            return ToolResponse::error(PublicError::invalid_input(
                "page_cursor is too long",
                "Use next_cursor exactly as returned by the previous response",
            ));
        }
    };

    let mut query = vec![
        ("fields".to_owned(), AD_SET_SUMMARY_FIELDS.to_owned()),
        ("limit".to_owned(), page_size.to_string()),
    ];
    if let Some(cursor) = cursor {
        query.push(("after".to_owned(), cursor));
    }

    let target = campaign_id.as_deref().unwrap_or(&account_id);
    let endpoint = format!("{target}/adsets");
    let payload = match graph.get_json(&endpoint, &query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    let raw = match serde_json::from_value::<RawAdSetList>(payload) {
        Ok(raw) => raw,
        Err(_) => {
            return ToolResponse::error(PublicError::invalid_upstream(
                "Meta returned an unexpected ad-set list",
            ));
        }
    };
    if raw.data.len() > usize::from(page_size) {
        return ToolResponse::error(PublicError::invalid_upstream(
            "Meta returned more ad sets than requested",
        ));
    }

    let next_cursor = match normalize_cursor(
        raw.paging
            .as_ref()
            .and_then(|paging| paging.cursors.as_ref())
            .and_then(|cursors| cursors.after.as_deref()),
    ) {
        Ok(cursor) => cursor,
        Err(()) => {
            return ToolResponse::error(PublicError::invalid_upstream(
                "Meta returned an oversized ad-set pagination cursor",
            ));
        }
    };
    let ad_sets = raw.data.into_iter().filter_map(normalize_ad_set).collect();
    ToolResponse::success(AdSetList {
        ad_sets,
        next_cursor,
    })
}

pub(crate) async fn read_ad_set(graph: &GraphClient, input: ReadAdSetInput) -> ToolResponse<AdSet> {
    let Some(ad_set_id) = normalize_object_id(&input.ad_set_id) else {
        return ToolResponse::error(PublicError::invalid_input(
            "ad_set_id must be a numeric Meta ad-set ID",
            "Use the numeric ID returned by list_ad_sets",
        ));
    };
    let fields = format!("{AD_SET_SUMMARY_FIELDS},{AD_SET_DETAIL_FIELDS}");
    let query = vec![("fields".to_owned(), fields)];
    let payload = match graph.get_json(&ad_set_id, &query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    let raw = match serde_json::from_value::<RawAdSet>(payload) {
        Ok(raw) => raw,
        Err(_) => {
            return ToolResponse::error(PublicError::invalid_upstream(
                "Meta returned unexpected ad-set metadata",
            ));
        }
    };
    match normalize_ad_set(raw) {
        Some(ad_set) => ToolResponse::success(ad_set),
        None => ToolResponse::error(PublicError::invalid_upstream(
            "Meta omitted or malformed the ad-set ID",
        )),
    }
}

fn normalize_ad_set(raw: RawAdSet) -> Option<AdSet> {
    let id = normalize_object_id(raw.id.as_deref()?)?;
    Some(AdSet {
        id,
        name: normalize_text(raw.name),
        campaign_id: raw.campaign_id.as_deref().and_then(normalize_object_id),
        status: normalize_text(raw.status),
        effective_status: normalize_text(raw.effective_status),
        daily_budget: normalize_scalar(raw.daily_budget),
        lifetime_budget: normalize_scalar(raw.lifetime_budget),
        budget_remaining: normalize_scalar(raw.budget_remaining),
        bid_amount: normalize_scalar(raw.bid_amount),
        bid_strategy: normalize_text(raw.bid_strategy),
        bid_constraints: normalize_scalar_map(raw.bid_constraints),
        optimization_goal: normalize_text(raw.optimization_goal),
        billing_event: normalize_text(raw.billing_event),
        start_time: normalize_text(raw.start_time),
        end_time: normalize_text(raw.end_time),
        created_time: normalize_text(raw.created_time),
        updated_time: normalize_text(raw.updated_time),
        is_dynamic_creative: raw.is_dynamic_creative,
        frequency_control_specs: normalize_frequency_specs(raw.frequency_control_specs),
        attribution_spec: normalize_attribution_specs(raw.attribution_spec),
        destination_type: normalize_text(raw.destination_type),
        pacing_type: normalize_string_list(raw.pacing_type),
        dsa_beneficiary: normalize_text(raw.dsa_beneficiary),
        dsa_payor: normalize_text(raw.dsa_payor),
        placement_soft_opt_out: normalize_string_map(raw.placement_soft_opt_out),
        targeting: normalize_nested_object(raw.targeting),
        promoted_object: normalize_nested_object(raw.promoted_object),
    })
}

fn normalize_optional_object_id(raw: Option<&str>) -> Result<Option<String>, ()> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    normalize_object_id(raw).map(Some).ok_or(())
}

fn normalize_page_size(raw: Option<u16>) -> Result<u16, ()> {
    let page_size = raw.unwrap_or(DEFAULT_PAGE_SIZE);
    (1..=MAX_PAGE_SIZE)
        .contains(&page_size)
        .then_some(page_size)
        .ok_or(())
}

fn normalize_cursor(raw: Option<&str>) -> Result<Option<String>, ()> {
    let Some(cursor) = raw.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if cursor.chars().count() > MAX_CURSOR_CHARS {
        return Err(());
    }
    Ok(Some(cursor.to_owned()))
}

fn normalize_text(raw: Option<String>) -> Option<String> {
    let value = raw?;
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

fn normalize_scalar(raw: Option<Value>) -> Option<String> {
    match raw? {
        Value::String(value) => normalize_text(Some(value)),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn normalize_unsigned(raw: Option<Value>) -> Option<u64> {
    match raw? {
        Value::String(value) => value.trim().parse().ok(),
        Value::Number(value) => value.as_u64(),
        _ => None,
    }
}

fn normalize_scalar_map(raw: Option<BTreeMap<String, Value>>) -> Option<BTreeMap<String, String>> {
    let normalized = raw?
        .into_iter()
        .take(MAX_COLLECTION_ITEMS)
        .filter_map(|(key, value)| {
            let key = normalize_text(Some(key))?;
            normalize_scalar(Some(value)).map(|value| (key, value))
        })
        .collect::<BTreeMap<_, _>>();
    (!normalized.is_empty()).then_some(normalized)
}

fn normalize_string_list(raw: Option<Vec<String>>) -> Option<Vec<String>> {
    let normalized = raw?
        .into_iter()
        .take(MAX_COLLECTION_ITEMS)
        .filter_map(|value| normalize_text(Some(value)))
        .collect::<Vec<_>>();
    (!normalized.is_empty()).then_some(normalized)
}

fn normalize_string_map(
    raw: Option<BTreeMap<String, Vec<String>>>,
) -> Option<BTreeMap<String, Vec<String>>> {
    let normalized = raw?
        .into_iter()
        .take(MAX_COLLECTION_ITEMS)
        .filter_map(|(key, values)| {
            let key = normalize_text(Some(key))?;
            normalize_string_list(Some(values)).map(|values| (key, values))
        })
        .collect::<BTreeMap<_, _>>();
    (!normalized.is_empty()).then_some(normalized)
}

fn normalize_frequency_specs(
    raw: Option<Vec<RawFrequencyControlSpec>>,
) -> Option<Vec<FrequencyControlSpec>> {
    let normalized = raw?
        .into_iter()
        .take(MAX_COLLECTION_ITEMS)
        .filter_map(|spec| {
            let spec = FrequencyControlSpec {
                event: normalize_text(spec.event),
                interval_days: normalize_unsigned(spec.interval_days),
                max_frequency: normalize_unsigned(spec.max_frequency),
            };
            (spec.event.is_some() || spec.interval_days.is_some() || spec.max_frequency.is_some())
                .then_some(spec)
        })
        .collect::<Vec<_>>();
    (!normalized.is_empty()).then_some(normalized)
}

fn normalize_attribution_specs(
    raw: Option<Vec<RawAttributionSpec>>,
) -> Option<Vec<AttributionSpec>> {
    let normalized = raw?
        .into_iter()
        .take(MAX_COLLECTION_ITEMS)
        .filter_map(|spec| {
            let spec = AttributionSpec {
                event_type: normalize_text(spec.event_type),
                window_days: normalize_unsigned(spec.window_days),
            };
            (spec.event_type.is_some() || spec.window_days.is_some()).then_some(spec)
        })
        .collect::<Vec<_>>();
    (!normalized.is_empty()).then_some(normalized)
}

fn normalize_nested_object(raw: Option<Value>) -> Option<Value> {
    raw.filter(Value::is_object)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        RawAdSet, normalize_account_id, normalize_ad_set, normalize_cursor, normalize_object_id,
        normalize_page_size,
    };

    #[test]
    fn validates_numeric_ids_and_page_bounds() {
        assert_eq!(normalize_account_id(" 123 "), Some("act_123".to_owned()));
        assert_eq!(normalize_account_id("act_123"), Some("act_123".to_owned()));
        assert_eq!(normalize_object_id("987"), Some("987".to_owned()));
        assert_eq!(normalize_account_id("../123"), None);
        assert_eq!(normalize_object_id("123/adsets"), None);
        assert!(normalize_page_size(None).is_ok_and(|size| size == 10));
        assert!(normalize_page_size(Some(0)).is_err());
        assert!(normalize_page_size(Some(101)).is_err());
    }

    #[test]
    fn preserves_an_opaque_bounded_cursor() {
        assert_eq!(
            normalize_cursor(Some(" Ab+/=._- ")),
            Ok(Some(" Ab+/=._- ".to_owned()))
        );
        assert!(normalize_cursor(Some(&"x".repeat(2_049))).is_err());
    }

    #[test]
    fn normalizes_typed_ad_set_fields() {
        let raw = serde_json::from_value::<RawAdSet>(json!({
            "id": "123",
            "name": " Prospecting ",
            "campaign_id": "456",
            "daily_budget": 2500,
            "lifetime_budget": " 10000 ",
            "bid_constraints": {"roas_average_floor": 20000},
            "frequency_control_specs": [{
                "event": "IMPRESSIONS",
                "interval_days": "7",
                "max_frequency": 2
            }],
            "attribution_spec": [{"event_type": "CLICK_THROUGH", "window_days": 7}],
            "pacing_type": ["standard"],
            "targeting": {"geo_locations": {"countries": ["US"]}},
            "promoted_object": "not-an-object"
        }))
        .unwrap();

        let ad_set = normalize_ad_set(raw).unwrap();
        assert_eq!(ad_set.name.as_deref(), Some("Prospecting"));
        assert_eq!(ad_set.daily_budget.as_deref(), Some("2500"));
        assert_eq!(ad_set.lifetime_budget.as_deref(), Some("10000"));
        assert_eq!(
            ad_set
                .bid_constraints
                .as_ref()
                .and_then(|values| values.get("roas_average_floor"))
                .map(String::as_str),
            Some("20000")
        );
        assert_eq!(
            ad_set.frequency_control_specs.as_ref().unwrap()[0].interval_days,
            Some(7)
        );
        assert!(ad_set.targeting.is_some());
        assert!(ad_set.promoted_object.is_none());
    }
}
