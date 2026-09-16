use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{ad_account as normalize_account_id, numeric_owned as normalize_object_id},
};

// Keep list results small. Creative metadata has dedicated tools and can be very large.
const AD_LIST_FIELDS: &str = "id,name,account_id,campaign_id,adset_id,status,configured_status,effective_status,creative{id},created_time,updated_time";
const AD_DETAIL_FIELDS: &str = "id,name,account_id,campaign_id,adset_id,status,configured_status,effective_status,creative{id},created_time,updated_time,conversion_domain,preview_shareable_link";
const DEFAULT_PAGE_SIZE: u16 = 10;
const MAX_PAGE_SIZE: u16 = 100;
const MAX_CURSOR_CHARS: usize = 2_048;
const MAX_NAME_CHARS: usize = 256;
const MAX_METADATA_CHARS: usize = 128;
const MAX_URL_CHARS: usize = 2_048;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListAdsInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    pub ad_account_id: String,
    /// Number of ads to return, from 1 through 100. Defaults to 10.
    pub page_size: Option<u16>,
    /// Optional numeric campaign ID. Mutually exclusive with `ad_set_id`.
    pub campaign_id: Option<String>,
    /// Optional numeric ad-set ID. Mutually exclusive with `campaign_id`.
    pub ad_set_id: Option<String>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadAdInput {
    /// Numeric Meta ad ID.
    pub ad_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AdList {
    pub ads: Vec<Ad>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Ad {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub campaign_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ad_set_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configured_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creative_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversion_domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_shareable_link: Option<String>,
}

#[derive(Debug)]
struct ValidatedListAds {
    endpoint: String,
    page_size: u16,
    page_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawAdList {
    #[serde(default)]
    data: Vec<RawAd>,
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
struct RawAd {
    id: Option<String>,
    name: Option<String>,
    account_id: Option<String>,
    campaign_id: Option<String>,
    adset_id: Option<String>,
    status: Option<String>,
    configured_status: Option<String>,
    effective_status: Option<String>,
    creative: Option<Value>,
    created_time: Option<String>,
    updated_time: Option<String>,
    conversion_domain: Option<String>,
    preview_shareable_link: Option<String>,
}

pub(crate) async fn list_ads(graph: &GraphClient, input: ListAdsInput) -> ToolResponse<AdList> {
    let validated = match validate_list_input(input) {
        Ok(validated) => validated,
        Err(error) => return ToolResponse::error(error),
    };

    let mut query = vec![
        ("fields".to_owned(), AD_LIST_FIELDS.to_owned()),
        ("limit".to_owned(), validated.page_size.to_string()),
    ];
    if let Some(cursor) = validated.page_cursor {
        query.push(("after".to_owned(), cursor));
    }

    let payload = match graph.get_json(&validated.endpoint, &query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    let raw = match serde_json::from_value::<RawAdList>(payload) {
        Ok(raw) => raw,
        Err(_) => {
            return ToolResponse::error(PublicError::invalid_upstream(
                "Meta returned an unexpected ad list",
            ));
        }
    };
    if raw.data.len() > usize::from(validated.page_size) {
        return ToolResponse::error(PublicError::invalid_upstream(
            "Meta returned more ads than requested",
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
            "Meta returned an oversized ad cursor",
        ));
    }
    let ads = raw.data.into_iter().filter_map(normalize_ad).collect();
    ToolResponse::success(AdList { ads, next_cursor })
}

pub(crate) async fn read_ad(graph: &GraphClient, input: ReadAdInput) -> ToolResponse<Ad> {
    let Some(ad_id) = normalize_object_id(&input.ad_id) else {
        return ToolResponse::error(PublicError::invalid_input(
            "ad_id must be a numeric Meta ad ID",
            "Use the ID returned by list_ads",
        ));
    };

    let query = vec![("fields".to_owned(), AD_DETAIL_FIELDS.to_owned())];
    let payload = match graph.get_json(&ad_id, &query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    let raw = match serde_json::from_value::<RawAd>(payload) {
        Ok(raw) => raw,
        Err(_) => {
            return ToolResponse::error(PublicError::invalid_upstream(
                "Meta returned unexpected ad metadata",
            ));
        }
    };

    match normalize_ad(raw) {
        Some(ad) => ToolResponse::success(ad),
        None => ToolResponse::error(PublicError::invalid_upstream("Meta omitted the ad ID")),
    }
}

fn validate_list_input(input: ListAdsInput) -> Result<ValidatedListAds, PublicError> {
    let account_id = normalize_account_id(&input.ad_account_id).ok_or_else(|| {
        PublicError::invalid_input(
            "ad_account_id must be a numeric Meta account ID",
            "Use an ID such as `act_123456789` or `123456789`",
        )
    })?;
    let campaign_id = normalize_optional_object_id(input.campaign_id).map_err(|()| {
        PublicError::invalid_input(
            "campaign_id must be a numeric Meta campaign ID",
            "Use a campaign ID returned by Meta, without URL or path characters",
        )
    })?;
    let ad_set_id = normalize_optional_object_id(input.ad_set_id).map_err(|()| {
        PublicError::invalid_input(
            "ad_set_id must be a numeric Meta ad-set ID",
            "Use an ad-set ID returned by Meta, without URL or path characters",
        )
    })?;
    if campaign_id.is_some() && ad_set_id.is_some() {
        return Err(PublicError::invalid_input(
            "campaign_id and ad_set_id are mutually exclusive",
            "Choose one scope, preferring the narrowest relevant ID",
        ));
    }

    let page_size = input.page_size.unwrap_or(DEFAULT_PAGE_SIZE);
    if !(1..=MAX_PAGE_SIZE).contains(&page_size) {
        return Err(PublicError::invalid_input(
            "page_size must be between 1 and 100",
            "Choose a bounded page_size and paginate with next_cursor",
        ));
    }

    let page_cursor = input.page_cursor.filter(|cursor| !cursor.is_empty());
    if page_cursor
        .as_ref()
        .is_some_and(|cursor| cursor.chars().count() > MAX_CURSOR_CHARS)
    {
        return Err(PublicError::invalid_input(
            "page_cursor is too long",
            "Use next_cursor exactly as returned by the previous response",
        ));
    }

    let endpoint = match (ad_set_id, campaign_id) {
        (Some(ad_set_id), None) => format!("{ad_set_id}/ads"),
        (None, Some(campaign_id)) => format!("{campaign_id}/ads"),
        (None, None) => format!("{account_id}/ads"),
        (Some(_), Some(_)) => unreachable!("conflicting scopes were rejected"),
    };
    Ok(ValidatedListAds {
        endpoint,
        page_size,
        page_cursor,
    })
}

fn normalize_optional_object_id(raw: Option<String>) -> Result<Option<String>, ()> {
    match raw {
        None => Ok(None),
        Some(raw) if raw.trim().is_empty() => Ok(None),
        Some(raw) => normalize_object_id(&raw).map(Some).ok_or(()),
    }
}

fn normalize_ad(raw: RawAd) -> Option<Ad> {
    let id = normalize_object_id(raw.id.as_deref()?)?;
    Some(Ad {
        id,
        name: bounded_text(raw.name, MAX_NAME_CHARS),
        account_id: raw.account_id.as_deref().and_then(normalize_object_id),
        campaign_id: raw.campaign_id.as_deref().and_then(normalize_object_id),
        ad_set_id: raw.adset_id.as_deref().and_then(normalize_object_id),
        status: bounded_text(raw.status, MAX_METADATA_CHARS),
        configured_status: bounded_text(raw.configured_status, MAX_METADATA_CHARS),
        effective_status: bounded_text(raw.effective_status, MAX_METADATA_CHARS),
        creative_id: normalize_creative_id(raw.creative),
        created_time: bounded_text(raw.created_time, MAX_METADATA_CHARS),
        updated_time: bounded_text(raw.updated_time, MAX_METADATA_CHARS),
        conversion_domain: bounded_text(raw.conversion_domain, MAX_NAME_CHARS),
        preview_shareable_link: bounded_text(raw.preview_shareable_link, MAX_URL_CHARS),
    })
}

fn normalize_creative_id(value: Option<Value>) -> Option<String> {
    let value = value?;
    let raw = match value {
        Value::Object(mut creative) => creative.remove("id")?,
        scalar => scalar,
    };
    match raw {
        Value::String(id) => normalize_object_id(&id),
        Value::Number(id) => normalize_object_id(&id.to_string()),
        _ => None,
    }
}

fn bounded_text(value: Option<String>, max_chars: usize) -> Option<String> {
    let value = value?;
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let mut chars = value.chars();
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
        ListAdsInput, MAX_CURSOR_CHARS, RawAd, normalize_ad, normalize_object_id,
        validate_list_input,
    };

    fn input() -> ListAdsInput {
        ListAdsInput {
            ad_account_id: "123".to_owned(),
            page_size: None,
            campaign_id: None,
            ad_set_id: None,
            page_cursor: None,
        }
    }

    #[test]
    fn validates_scopes_and_preserves_opaque_cursor() {
        let mut account = input();
        account.page_cursor = Some("opaque+/= cursor".to_owned());
        let account = validate_list_input(account).unwrap();
        assert_eq!(account.endpoint, "act_123/ads");
        assert_eq!(account.page_size, 10);
        assert_eq!(account.page_cursor.as_deref(), Some("opaque+/= cursor"));

        let mut campaign = input();
        campaign.campaign_id = Some("456".to_owned());
        assert_eq!(validate_list_input(campaign).unwrap().endpoint, "456/ads");

        let mut ad_set = input();
        ad_set.ad_set_id = Some("789".to_owned());
        assert_eq!(validate_list_input(ad_set).unwrap().endpoint, "789/ads");
    }

    #[test]
    fn rejects_conflicting_unsafe_or_unbounded_inputs() {
        let mut conflicting = input();
        conflicting.campaign_id = Some("456".to_owned());
        conflicting.ad_set_id = Some("789".to_owned());
        assert!(validate_list_input(conflicting).is_err());

        let mut unsafe_account = input();
        unsafe_account.ad_account_id = "../123".to_owned();
        assert!(validate_list_input(unsafe_account).is_err());

        let mut unbounded = input();
        unbounded.page_size = Some(101);
        assert!(validate_list_input(unbounded).is_err());

        let mut oversized_cursor = input();
        oversized_cursor.page_cursor = Some("x".repeat(MAX_CURSOR_CHARS + 1));
        assert!(validate_list_input(oversized_cursor).is_err());

        assert_eq!(normalize_object_id("123"), Some("123".to_owned()));
        assert_eq!(normalize_object_id("ad_123"), None);
    }

    #[test]
    fn flattens_creative_to_its_id() {
        let raw: RawAd = serde_json::from_value(json!({
            "id": "1001",
            "name": "  Launch ad  ",
            "creative": {
                "id": "2002",
                "object_story_spec": {"large": "payload"}
            },
            "configured_status": "PAUSED",
            "effective_status": "CAMPAIGN_PAUSED"
        }))
        .unwrap();

        let ad = normalize_ad(raw).unwrap();
        assert_eq!(ad.id, "1001");
        assert_eq!(ad.name.as_deref(), Some("Launch ad"));
        assert_eq!(ad.creative_id.as_deref(), Some("2002"));
        assert_eq!(ad.effective_status.as_deref(), Some("CAMPAIGN_PAUSED"));
        assert!(
            !serde_json::to_string(&ad)
                .unwrap()
                .contains("object_story_spec")
        );
    }
}
