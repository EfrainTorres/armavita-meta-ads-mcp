use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{MAX_DIGITS, ad_account as normalize_account_id},
};

const MAX_COUNTRIES: usize = 250;
const MAX_TARGETING_IDS: usize = 500;
const MAX_FLEXIBLE_GROUPS: usize = 25;
const MAX_FLEXIBLE_ITEMS: usize = 1_000;
const MAX_PLACEMENTS: usize = 32;
const MAX_TARGETING_BYTES: usize = 64 * 1024;

type EstimateRequest = (String, Vec<(String, String)>);

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EstimateAudienceSizeInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    pub ad_account_id: String,
    /// Bounded common v26 targeting. At least one country is required.
    pub targeting: AudienceEstimateTargeting,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudienceEstimateTargeting {
    pub geo_locations: EstimateGeoLocations,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_min: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_max: Option<u8>,
    /// Suggested age range for Advantage+ audience, as `[minimum, maximum]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_range: Option<[u8; 2]>,
    /// `1` targets men and `2` targets women. Omit for all genders.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genders: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_audiences: Option<Vec<TargetingId>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded_custom_audiences: Option<Vec<TargetingId>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interests: Option<Vec<TargetingId>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behaviors: Option<Vec<TargetingId>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flexible_spec: Option<Vec<FlexibleTargetingGroup>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_platforms: Option<Vec<DevicePlatform>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher_platforms: Option<Vec<PublisherPlatform>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facebook_positions: Option<Vec<FacebookPosition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instagram_positions: Option<Vec<InstagramPosition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threads_positions: Option<Vec<ThreadsPosition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience_network_positions: Option<Vec<AudienceNetworkPosition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messenger_positions: Option<Vec<MessengerPosition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_age_unknown: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub targeting_automation: Option<TargetingAutomation>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EstimateGeoLocations {
    /// One through 250 two-letter uppercase country codes.
    #[schemars(length(min = 1, max = 250), inner(length(min = 2, max = 2)))]
    pub countries: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetingId {
    /// Numeric Meta targeting or audience ID.
    pub id: String,
}

/// Common flexible-targeting dimensions used by the Python server.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlexibleTargetingGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_audiences: Option<Vec<TargetingId>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interests: Option<Vec<TargetingId>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behaviors: Option<Vec<TargetingId>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetingAutomation {
    /// Explicit Advantage+ audience intent. Sent to Meta as `1` or `0`.
    pub advantage_audience: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DevicePlatform {
    ConnectedTv,
    Desktop,
    Mobile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PublisherPlatform {
    Facebook,
    Instagram,
    Threads,
    Messenger,
    AudienceNetwork,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FacebookPosition {
    Feed,
    RightHandColumn,
    Marketplace,
    Story,
    Search,
    InstreamVideo,
    FacebookReels,
    FacebookReelsOverlay,
    ProfileFeed,
    Notification,
}

/// Current v26 positions; removed Instagram `explore` is rejected.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InstagramPosition {
    Stream,
    Story,
    ExploreHome,
    Reels,
    ProfileFeed,
    IgSearch,
    ProfileReels,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThreadsPosition {
    ThreadsStream,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AudienceNetworkPosition {
    Classic,
    RewardedVideo,
}

/// Current v26 positions; removed Messenger `story` is rejected.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessengerPosition {
    SponsoredMessages,
}

/// Meta's range estimate. `-1` bounds mean unavailable, not zero.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AudienceSizeEstimate {
    pub estimate_ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub users_lower_bound: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub users_upper_bound: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct RawEstimatePage {
    data: Vec<RawEstimate>,
}

#[derive(Debug, Deserialize)]
struct RawEstimate {
    estimate_ready: Option<bool>,
    users_lower_bound: Option<i64>,
    users_upper_bound: Option<i64>,
}

pub(crate) async fn estimate_audience_size(
    graph: &GraphClient,
    input: EstimateAudienceSizeInput,
) -> ToolResponse<AudienceSizeEstimate> {
    let (endpoint, query) = match build_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.get_json(&endpoint, &query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };

    match parse_estimate(payload) {
        Ok(estimate) => ToolResponse::success(estimate),
        Err(error) => ToolResponse::error(error),
    }
}

fn build_request(input: &EstimateAudienceSizeInput) -> Result<EstimateRequest, PublicError> {
    let account_id = normalize_account_id(&input.ad_account_id).ok_or_else(|| {
        PublicError::invalid_input(
            "ad_account_id must be a numeric Meta account ID",
            "Use an ID such as `act_123456789` or `123456789`",
        )
    })?;
    validate_targeting(&input.targeting)?;

    let mut targeting = serde_json::to_value(&input.targeting).map_err(|_| {
        PublicError::invalid_input(
            "targeting could not be encoded",
            "Use the typed targeting fields",
        )
    })?;
    if let Some(enabled) = input
        .targeting
        .targeting_automation
        .as_ref()
        .map(|automation| automation.advantage_audience)
        && let Some(setting) = targeting.pointer_mut("/targeting_automation/advantage_audience")
    {
        *setting = Value::from(u8::from(enabled));
    }
    let targeting_json = serde_json::to_string(&targeting).map_err(|_| {
        PublicError::invalid_input(
            "targeting could not be encoded",
            "Use the typed targeting fields",
        )
    })?;
    if targeting_json.len() > MAX_TARGETING_BYTES {
        return Err(PublicError::invalid_input(
            "targeting exceeds the 64 KiB safety limit",
            "Narrow the targeting specification",
        ));
    }

    Ok((
        format!("{account_id}/reachestimate"),
        vec![("targeting_spec".to_owned(), targeting_json)],
    ))
}

fn validate_targeting(targeting: &AudienceEstimateTargeting) -> Result<(), PublicError> {
    let countries = &targeting.geo_locations.countries;
    if countries.is_empty()
        || countries.len() > MAX_COUNTRIES
        || countries.iter().any(|country| {
            country.len() != 2 || !country.bytes().all(|byte| byte.is_ascii_uppercase())
        })
    {
        return Err(PublicError::invalid_input(
            "countries must contain 1 through 250 two-letter uppercase codes",
            "Use codes such as `US`, `GB`, or `CA`",
        ));
    }
    if targeting
        .age_min
        .is_some_and(|age| !(13..=65).contains(&age))
        || targeting
            .age_max
            .is_some_and(|age| !(13..=65).contains(&age))
        || targeting
            .age_min
            .zip(targeting.age_max)
            .is_some_and(|(minimum, maximum)| minimum > maximum)
        || targeting.age_range.is_some_and(|[minimum, maximum]| {
            !(18..=65).contains(&minimum) || maximum > 65 || minimum > maximum
        })
    {
        return Err(PublicError::invalid_input(
            "age targeting is outside Meta's supported range",
            "Use ordered ages from 13 through 65; Advantage+ age_range starts at 18",
        ));
    }
    if targeting.genders.as_ref().is_some_and(|values| {
        values.is_empty() || values.len() > 2 || values.iter().any(|value| !matches!(value, 1 | 2))
    }) {
        return Err(PublicError::invalid_input(
            "genders must contain only `1` and/or `2`",
            "Omit genders for all genders",
        ));
    }

    for ids in [
        targeting.custom_audiences.as_deref(),
        targeting.excluded_custom_audiences.as_deref(),
        targeting.interests.as_deref(),
        targeting.behaviors.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        validate_ids(ids, MAX_TARGETING_IDS)?;
    }
    validate_flexible_spec(targeting.flexible_spec.as_deref())?;
    validate_nonempty_lists(targeting)?;
    Ok(())
}

fn validate_ids(ids: &[TargetingId], maximum: usize) -> Result<(), PublicError> {
    if ids.is_empty() || ids.len() > maximum || ids.iter().any(|item| !valid_numeric_id(&item.id)) {
        return Err(PublicError::invalid_input(
            "targeting lists must contain bounded numeric Meta IDs",
            format!("Provide 1 through {maximum} numeric IDs per list"),
        ));
    }
    Ok(())
}

fn validate_flexible_spec(groups: Option<&[FlexibleTargetingGroup]>) -> Result<(), PublicError> {
    let Some(groups) = groups else {
        return Ok(());
    };
    if groups.is_empty() || groups.len() > MAX_FLEXIBLE_GROUPS {
        return Err(PublicError::invalid_input(
            "flexible_spec must contain 1 through 25 groups",
            "Split or simplify the flexible targeting expression",
        ));
    }
    for group in groups {
        let lists = [
            group.custom_audiences.as_deref(),
            group.interests.as_deref(),
            group.behaviors.as_deref(),
        ];
        let item_count = lists
            .iter()
            .flatten()
            .map(|items| items.len())
            .sum::<usize>();
        if item_count == 0 || item_count > MAX_FLEXIBLE_ITEMS {
            return Err(PublicError::invalid_input(
                "each flexible_spec group must contain 1 through 1,000 items",
                "Remove empty groups or narrow large groups",
            ));
        }
        for ids in lists.into_iter().flatten() {
            validate_ids(ids, MAX_FLEXIBLE_ITEMS)?;
        }
    }
    Ok(())
}

fn validate_nonempty_lists(targeting: &AudienceEstimateTargeting) -> Result<(), PublicError> {
    let lengths = [
        targeting.device_platforms.as_ref().map(Vec::len),
        targeting.publisher_platforms.as_ref().map(Vec::len),
        targeting.facebook_positions.as_ref().map(Vec::len),
        targeting.instagram_positions.as_ref().map(Vec::len),
        targeting.threads_positions.as_ref().map(Vec::len),
        targeting.audience_network_positions.as_ref().map(Vec::len),
        targeting.messenger_positions.as_ref().map(Vec::len),
    ];
    if lengths
        .into_iter()
        .flatten()
        .any(|length| length == 0 || length > MAX_PLACEMENTS)
    {
        return Err(PublicError::invalid_input(
            "placement lists must contain 1 through 32 values",
            "Omit a placement field to use Meta's defaults",
        ));
    }
    if let Some(platforms) = &targeting.publisher_platforms {
        let missing_platform = (targeting.facebook_positions.is_some()
            && !platforms.contains(&PublisherPlatform::Facebook))
            || (targeting.instagram_positions.is_some()
                && !platforms.contains(&PublisherPlatform::Instagram))
            || (targeting.threads_positions.is_some()
                && !platforms.contains(&PublisherPlatform::Threads))
            || (targeting.audience_network_positions.is_some()
                && !platforms.contains(&PublisherPlatform::AudienceNetwork))
            || (targeting.messenger_positions.is_some()
                && !platforms.contains(&PublisherPlatform::Messenger));
        if missing_platform {
            return Err(PublicError::invalid_input(
                "a positions list lacks its matching publisher platform",
                "Add the publisher platform or omit publisher_platforms to use Meta's defaults",
            ));
        }
    }
    Ok(())
}

fn parse_estimate(payload: Value) -> Result<AudienceSizeEstimate, PublicError> {
    let mut page = serde_json::from_value::<RawEstimatePage>(payload).map_err(|_| {
        PublicError::invalid_upstream("Meta returned an unexpected reach-estimate response")
    })?;
    if page.data.len() != 1 {
        return Err(PublicError::invalid_upstream(
            "Meta did not return exactly one reach estimate",
        ));
    }
    let raw = page.data.pop().expect("length checked above");
    let estimate_ready = raw
        .estimate_ready
        .ok_or_else(|| PublicError::invalid_upstream("Meta omitted reach-estimate readiness"))?;
    let valid_bounds = match (raw.users_lower_bound, raw.users_upper_bound) {
        (Some(-1), Some(-1)) => true,
        (Some(lower), Some(upper)) => lower >= 0 && lower <= upper,
        (None, None) => !estimate_ready,
        _ => false,
    };
    if !valid_bounds {
        return Err(PublicError::invalid_upstream(
            "Meta returned inconsistent reach-estimate bounds",
        ));
    }

    Ok(AudienceSizeEstimate {
        estimate_ready,
        users_lower_bound: raw.users_lower_bound,
        users_upper_bound: raw.users_upper_bound,
    })
}

fn valid_numeric_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_DIGITS
        && value.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{AudienceSizeEstimate, EstimateAudienceSizeInput, build_request, parse_estimate};

    fn minimal_input() -> EstimateAudienceSizeInput {
        serde_json::from_value(json!({
            "ad_account_id": "123",
            "targeting": {
                "geo_locations": {"countries": ["US"]},
                "age_min": 20,
                "age_max": 40,
                "targeting_automation": {"advantage_audience": false}
            }
        }))
        .expect("valid input")
    }

    #[test]
    fn builds_only_the_current_reach_estimate_request() {
        let (endpoint, query) = build_request(&minimal_input()).expect("valid request");
        assert_eq!(endpoint, "act_123/reachestimate");
        assert_eq!(query.len(), 1);
        assert_eq!(query[0].0, "targeting_spec");
        let targeting: Value = serde_json::from_str(&query[0].1).expect("targeting JSON");
        assert_eq!(targeting["geo_locations"]["countries"], json!(["US"]));
        assert_eq!(targeting["targeting_automation"]["advantage_audience"], 0);
        assert!(targeting.get("access_token").is_none());
        assert!(targeting.get("age_range").is_none());
    }

    #[test]
    fn rejects_removed_placements_and_credential_fields() {
        let mut input = serde_json::to_value(minimal_input()).expect("serialize input");
        input["targeting"]["instagram_positions"] = json!(["explore"]);
        assert!(serde_json::from_value::<EstimateAudienceSizeInput>(input).is_err());

        let mut input = serde_json::to_value(minimal_input()).expect("serialize input");
        input["targeting"]["messenger_positions"] = json!(["story"]);
        assert!(serde_json::from_value::<EstimateAudienceSizeInput>(input).is_err());

        let mut input = serde_json::to_value(minimal_input()).expect("serialize input");
        input["targeting"]["facebook_positions"] = json!(["video_feeds"]);
        assert!(serde_json::from_value::<EstimateAudienceSizeInput>(input).is_err());

        let mut input = serde_json::to_value(minimal_input()).expect("serialize input");
        input["meta_access_token"] = json!("secret");
        assert!(serde_json::from_value::<EstimateAudienceSizeInput>(input).is_err());
    }

    #[test]
    fn requires_countries_and_numeric_ids() {
        let mut input = minimal_input();
        input.targeting.geo_locations.countries.clear();
        assert!(build_request(&input).is_err());

        let mut input = minimal_input();
        input.targeting.interests = Some(vec![super::TargetingId {
            id: "not-an-id".to_owned(),
        }]);
        assert!(build_request(&input).is_err());
    }

    #[test]
    fn requires_matching_publisher_platforms() {
        let mut input = minimal_input();
        input.targeting.publisher_platforms = Some(vec![super::PublisherPlatform::Facebook]);
        input.targeting.instagram_positions = Some(vec![super::InstagramPosition::Stream]);
        assert!(build_request(&input).is_err());
    }

    #[test]
    fn preserves_bounds_without_computing_a_midpoint() {
        let AudienceSizeEstimate {
            estimate_ready,
            users_lower_bound,
            users_upper_bound,
        } = parse_estimate(json!({
            "data": [{
                "estimate_ready": true,
                "users_lower_bound": 100,
                "users_upper_bound": 301
            }]
        }))
        .expect("valid estimate");
        assert!(estimate_ready);
        assert_eq!(users_lower_bound, Some(100));
        assert_eq!(users_upper_bound, Some(301));
    }

    #[test]
    fn preserves_documented_unavailable_and_pending_states() {
        let unavailable = parse_estimate(json!({
            "data": [{
                "estimate_ready": true,
                "users_lower_bound": -1,
                "users_upper_bound": -1
            }]
        }))
        .expect("documented unavailable estimate");
        assert_eq!(unavailable.users_lower_bound, Some(-1));
        assert_eq!(unavailable.users_upper_bound, Some(-1));

        let pending = parse_estimate(json!({
            "data": [{"estimate_ready": false}]
        }))
        .expect("pending estimate");
        assert_eq!(pending.users_lower_bound, None);
        assert_eq!(pending.users_upper_bound, None);
    }
}
