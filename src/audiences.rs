use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize, de::IgnoredAny};
use serde_json::Value;

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{ad_account as normalize_ad_account_id, numeric_owned as normalize_numeric_id},
};

// v26.0 CustomAudience fields. Rules, data sources, descriptions, lookalike specs, and
// audience-member data are intentionally excluded from model-visible results.
const AUDIENCE_LIST_FIELDS: &str = "id,name,subtype,approximate_count_lower_bound,approximate_count_upper_bound,delivery_status,operation_status,is_value_based,time_updated";
const AUDIENCE_DETAIL_FIELDS: &str = "id,name,subtype,approximate_count_lower_bound,approximate_count_upper_bound,delivery_status,operation_status,customer_file_source,is_value_based,retention_days,is_eligible_for_sac_campaigns,fields_violating_integrity_policy,time_created,time_updated";
const DEFAULT_PAGE_SIZE: u16 = 25;
const MAX_PAGE_SIZE: u16 = 100;
const MAX_CURSOR_CHARS: usize = 2_048;
const MAX_NAME_CHARS: usize = 256;
const MAX_STATUS_CHARS: usize = 256;
const MAX_METADATA_CHARS: usize = 128;

type ListRequest = (String, Vec<(String, String)>, u16);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListCustomAudiencesInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    pub ad_account_id: String,
    /// Number of audiences to return, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadCustomAudienceInput {
    /// Numeric custom-audience ID.
    pub custom_audience_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CustomAudienceList {
    pub audiences: Vec<CustomAudience>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Compact custom-audience metadata. No audience members, rules, or source payloads are exposed.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CustomAudience {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtype: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approximate_count_lower_bound: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approximate_count_upper_bound: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_status: Option<AudienceStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_status: Option<AudienceStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer_file_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_value_based: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_eligible_for_sac_campaigns: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrity_policy_issue_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_created: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_updated: Option<i64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AudienceStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Deserialize)]
struct RawAudienceList {
    #[serde(default)]
    data: Vec<RawCustomAudience>,
    paging: Option<RawPaging>,
}

#[derive(Deserialize)]
struct RawPaging {
    cursors: Option<RawCursors>,
}

#[derive(Deserialize)]
struct RawCursors {
    after: Option<String>,
}

#[derive(Deserialize)]
struct RawCustomAudience {
    id: Option<String>,
    name: Option<String>,
    subtype: Option<String>,
    approximate_count_lower_bound: Option<i64>,
    approximate_count_upper_bound: Option<i64>,
    delivery_status: Option<RawAudienceStatus>,
    operation_status: Option<RawAudienceStatus>,
    customer_file_source: Option<String>,
    is_value_based: Option<bool>,
    retention_days: Option<i64>,
    is_eligible_for_sac_campaigns: Option<bool>,
    fields_violating_integrity_policy: Option<Vec<IgnoredAny>>,
    time_created: Option<i64>,
    time_updated: Option<i64>,
}

#[derive(Deserialize)]
struct RawAudienceStatus {
    code: Option<i64>,
    description: Option<String>,
}

pub(crate) async fn list_custom_audiences(
    graph: &GraphClient,
    input: ListCustomAudiencesInput,
) -> ToolResponse<CustomAudienceList> {
    let (endpoint, query, page_size) = match build_list_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.get_json(&endpoint, &query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };

    match parse_list_payload(payload, page_size) {
        Ok(audiences) => ToolResponse::success(audiences),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn read_custom_audience(
    graph: &GraphClient,
    input: ReadCustomAudienceInput,
) -> ToolResponse<CustomAudience> {
    let Some(audience_id) = normalize_numeric_id(&input.custom_audience_id) else {
        return ToolResponse::error(PublicError::invalid_input(
            "custom_audience_id must be a numeric Meta audience ID",
            "Use the ID returned by list_custom_audiences",
        ));
    };

    let query = vec![("fields".to_owned(), AUDIENCE_DETAIL_FIELDS.to_owned())];
    let payload = match graph.get_json(&audience_id, &query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    let raw = match serde_json::from_value::<RawCustomAudience>(payload) {
        Ok(raw) => raw,
        Err(_) => {
            return ToolResponse::error(PublicError::invalid_upstream(
                "Meta returned unexpected custom-audience metadata",
            ));
        }
    };

    match normalize_audience(raw) {
        Some(audience) => ToolResponse::success(audience),
        None => ToolResponse::error(PublicError::invalid_upstream(
            "Meta omitted a valid custom-audience ID",
        )),
    }
}

fn build_list_request(input: &ListCustomAudiencesInput) -> Result<ListRequest, PublicError> {
    let account_id = normalize_ad_account_id(&input.ad_account_id).ok_or_else(|| {
        PublicError::invalid_input(
            "ad_account_id must be a numeric Meta account ID",
            "Use an ID such as `act_123456789` or `123456789`",
        )
    })?;
    let page_size = input.page_size.unwrap_or(DEFAULT_PAGE_SIZE);
    if !(1..=MAX_PAGE_SIZE).contains(&page_size) {
        return Err(PublicError::invalid_input(
            "page_size must be between 1 and 100",
            "Choose a bounded page_size and paginate with next_cursor",
        ));
    }

    let page_cursor = input
        .page_cursor
        .as_deref()
        .filter(|cursor| !cursor.is_empty());
    if page_cursor.is_some_and(|cursor| cursor.chars().count() > MAX_CURSOR_CHARS) {
        return Err(PublicError::invalid_input(
            "page_cursor is too long",
            "Use next_cursor exactly as returned by the previous response",
        ));
    }

    let mut query = vec![
        ("fields".to_owned(), AUDIENCE_LIST_FIELDS.to_owned()),
        ("limit".to_owned(), page_size.to_string()),
    ];
    if let Some(cursor) = page_cursor {
        query.push(("after".to_owned(), cursor.to_owned()));
    }

    Ok((format!("{account_id}/customaudiences"), query, page_size))
}

fn parse_list_payload(
    payload: Value,
    requested_page_size: u16,
) -> Result<CustomAudienceList, PublicError> {
    let raw = serde_json::from_value::<RawAudienceList>(payload).map_err(|_| {
        PublicError::invalid_upstream("Meta returned an unexpected custom-audience list")
    })?;
    if raw.data.len() > usize::from(requested_page_size) {
        return Err(PublicError::invalid_upstream(
            "Meta returned more custom audiences than requested",
        ));
    }

    let next_cursor = raw
        .paging
        .and_then(|paging| paging.cursors)
        .and_then(|cursors| cursors.after);
    if next_cursor
        .as_ref()
        .is_some_and(|cursor| cursor.chars().count() > MAX_CURSOR_CHARS)
    {
        return Err(PublicError::invalid_upstream(
            "Meta returned an oversized custom-audience cursor",
        ));
    }

    let audiences = raw
        .data
        .into_iter()
        .filter_map(normalize_audience)
        .collect();
    Ok(CustomAudienceList {
        audiences,
        next_cursor,
    })
}

fn normalize_audience(raw: RawCustomAudience) -> Option<CustomAudience> {
    let id = normalize_numeric_id(raw.id.as_deref()?)?;
    let integrity_policy_issue_count = raw
        .fields_violating_integrity_policy
        .map(|fields| u32::try_from(fields.len()).unwrap_or(u32::MAX));

    Some(CustomAudience {
        id,
        name: clean_bounded_string(raw.name, MAX_NAME_CHARS),
        subtype: clean_bounded_string(raw.subtype, MAX_METADATA_CHARS),
        approximate_count_lower_bound: raw.approximate_count_lower_bound,
        approximate_count_upper_bound: raw.approximate_count_upper_bound,
        delivery_status: normalize_status(raw.delivery_status),
        operation_status: normalize_status(raw.operation_status),
        customer_file_source: clean_bounded_string(raw.customer_file_source, MAX_METADATA_CHARS),
        is_value_based: raw.is_value_based,
        retention_days: raw.retention_days,
        is_eligible_for_sac_campaigns: raw.is_eligible_for_sac_campaigns,
        integrity_policy_issue_count,
        time_created: raw.time_created,
        time_updated: raw.time_updated,
    })
}

fn normalize_status(raw: Option<RawAudienceStatus>) -> Option<AudienceStatus> {
    let raw = raw?;
    let status = AudienceStatus {
        code: raw.code,
        description: clean_bounded_string(raw.description, MAX_STATUS_CHARS),
    };
    (status.code.is_some() || status.description.is_some()).then_some(status)
}

fn clean_bounded_string(value: Option<String>, max_chars: usize) -> Option<String> {
    let value = value?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut chars = trimmed.chars();
    let mut output = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        output.pop();
        output.push('…');
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        AUDIENCE_DETAIL_FIELDS, AUDIENCE_LIST_FIELDS, ListCustomAudiencesInput, build_list_request,
        normalize_ad_account_id, normalize_numeric_id, parse_list_payload,
    };

    #[test]
    fn builds_a_bounded_v26_list_request_from_strict_ids() {
        let input = ListCustomAudiencesInput {
            ad_account_id: " act_123 ".to_owned(),
            page_size: Some(25),
            page_cursor: Some(" opaque-cursor ".to_owned()),
        };
        let (endpoint, query, page_size) = build_list_request(&input).unwrap();
        assert_eq!(endpoint, "act_123/customaudiences");
        assert_eq!(page_size, 25);
        assert!(query.contains(&("limit".to_owned(), "25".to_owned())));
        assert!(query.contains(&("after".to_owned(), " opaque-cursor ".to_owned())));
        assert_eq!(normalize_ad_account_id("123"), Some("act_123".to_owned()));
        assert_eq!(normalize_numeric_id("456"), Some("456".to_owned()));
        assert_eq!(normalize_ad_account_id("act_../123"), None);
        assert_eq!(normalize_numeric_id("https://example.test/1"), None);

        for fields in [AUDIENCE_LIST_FIELDS, AUDIENCE_DETAIL_FIELDS] {
            assert!(!fields.contains("rule"));
            assert!(!fields.contains("description"));
            assert!(!fields.contains("data_source"));
            assert!(!fields.contains("lookalike_spec"));
        }
    }

    #[test]
    fn rejects_unbounded_page_inputs_and_upstream_paging() {
        let invalid_page = ListCustomAudiencesInput {
            ad_account_id: "123".to_owned(),
            page_size: Some(0),
            page_cursor: None,
        };
        assert!(build_list_request(&invalid_page).is_err());

        let invalid_cursor = ListCustomAudiencesInput {
            page_size: Some(100),
            page_cursor: Some("x".repeat(2_049)),
            ..invalid_page
        };
        assert!(build_list_request(&invalid_cursor).is_err());

        let oversized_page = json!({
            "data": [{"id": "1"}, {"id": "2"}]
        });
        assert!(parse_list_payload(oversized_page, 1).is_err());
        let oversized_cursor = json!({
            "data": [],
            "paging": {"cursors": {"after": "x".repeat(2_049)}}
        });
        assert!(parse_list_payload(oversized_cursor, 1).is_err());
    }

    #[test]
    fn maps_only_compact_aggregate_audience_metadata() {
        let payload = json!({
            "data": [{
                "id": "42",
                "name": " Purchasers ",
                "subtype": "CUSTOM",
                "approximate_count_lower_bound": 1000,
                "approximate_count_upper_bound": 1200,
                "delivery_status": {"code": 200, "description": "Ready"},
                "operation_status": {"code": 200, "description": "Complete"},
                "fields_violating_integrity_policy": ["rule", "name"],
                "description": "do not expose this",
                "rule": "do not expose this",
                "lookalike_spec": {"country": "US"},
                "users": [{"email": "secret@example.test"}]
            }],
            "paging": {"cursors": {"after": "next"}}
        });

        let list = parse_list_payload(payload, 1).unwrap();
        assert_eq!(list.audiences.len(), 1);
        assert_eq!(list.audiences[0].name.as_deref(), Some("Purchasers"));
        assert_eq!(list.audiences[0].integrity_policy_issue_count, Some(2));
        assert_eq!(list.next_cursor.as_deref(), Some("next"));

        let output = serde_json::to_string(&list).unwrap();
        assert!(!output.contains("secret@example.test"));
        assert!(!output.contains("do not expose this"));
        assert!(!output.contains("lookalike_spec"));
        assert!(!output.contains("fields_violating_integrity_policy"));
    }
}
