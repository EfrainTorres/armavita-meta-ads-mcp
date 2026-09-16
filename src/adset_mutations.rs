// Copyright (C) 2025 ArmaVita LLC
// SPDX-License-Identifier: AGPL-3.0-only

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    audience_estimate::{
        AudienceEstimateTargeting, FlexibleTargetingGroup, PublisherPlatform, TargetingId,
    },
    bounded_json::credential_value,
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{
        MAX_DIGITS, ad_account as normalize_ad_account_id, numeric_owned as normalize_numeric_id,
        numeric_value as normalize_numeric_value,
    },
    mutation_result::{ambiguous_mutation_result, mutation_error_without_blind_retry},
    node_identity::{MetaNodeKind, verify_meta_node},
};

const MAX_NAME_CHARS: usize = 256;
const MAX_DSA_CHARS: usize = 512;
const MAX_URL_CHARS: usize = 2_048;
const MAX_RENAME_CHARS: usize = 100;
const MAX_COUNTRIES: usize = 250;
const MAX_TARGETING_IDS: usize = 500;
const MAX_FLEXIBLE_GROUPS: usize = 25;
const MAX_FLEXIBLE_ITEMS: usize = 1_000;
const MAX_PLACEMENTS: usize = 32;
const MAX_ATTRIBUTION_SPECS: usize = 3;
const MAX_FREQUENCY_SPECS: usize = 4;
const MAX_TARGETING_BYTES: usize = 64 * 1024;
const MAX_NESTED_BYTES: usize = 16 * 1024;
const MAX_COPY_MAPPINGS: usize = 64;
const MAX_UNIX_TIME: u64 = 4_102_444_800; // 2100-01-01T00:00:00Z

/// Create one ad set. Targeting and budget intent are explicit; delivery defaults
/// to `PAUSED`. Repeating a successful call creates another ad set.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAdSetInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
    pub ad_account_id: String,
    /// Numeric parent campaign ID.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub campaign_id: String,
    /// Ad-set name, from 1 through 256 plain-text characters.
    #[schemars(length(min = 1, max = 256))]
    pub name: String,
    pub optimization_goal: AdSetOptimizationGoal,
    pub billing_event: AdSetBillingEvent,
    /// Explicit daily, lifetime, or parent-campaign budget intent.
    pub budget: CreateAdSetBudget,
    /// Bounded common targeting. Include explicit Advantage+ audience intent.
    pub targeting: AudienceEstimateTargeting,
    /// Delivery status. Defaults to `PAUSED`; `ACTIVE` begins eligible delivery.
    #[serde(default)]
    #[schemars(default)]
    pub status: AdSetCreateStatus,
    /// Optional coherent bid strategy and its required control.
    pub bid: Option<AdSetBid>,
    /// Optional UTC Unix start time, through 2100-01-01.
    #[schemars(range(min = 1, max = 4_102_444_800_u64))]
    pub start_time: Option<u64>,
    /// Optional UTC Unix end time. Required for a lifetime budget.
    #[schemars(range(min = 1, max = 4_102_444_800_u64))]
    pub end_time: Option<u64>,
    pub promoted_object: Option<AdSetPromotedObject>,
    pub destination_type: Option<AdSetDestinationType>,
    /// Dynamic creative can only be selected when the ad set is created.
    pub is_dynamic_creative: Option<bool>,
    pub placement_soft_opt_out: Option<PlacementSoftOptOut>,
    #[schemars(length(min = 1, max = 3))]
    pub attribution_spec: Option<Vec<AttributionSpec>>,
    #[schemars(length(min = 1, max = 4))]
    pub frequency_control_specs: Option<Vec<FrequencyControlSpec>>,
    /// DSA beneficiary text required by Meta for applicable regulated delivery.
    #[schemars(length(min = 1, max = 512))]
    pub dsa_beneficiary: Option<String>,
    /// DSA payor text required by Meta for applicable regulated delivery.
    #[schemars(length(min = 1, max = 512))]
    pub dsa_payor: Option<String>,
}

/// Update one ad set. An unchanged repeated request is idempotent, but `ACTIVE`
/// may begin delivery. Create-only dynamic-creative configuration is not exposed.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateAdSetInput {
    /// Expected owner ad-account ID, with or without the `act_` prefix.
    #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
    pub ad_account_id: String,
    /// Numeric Meta ad-set ID.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub ad_set_id: String,
    #[schemars(length(min = 1, max = 256))]
    pub name: Option<String>,
    pub status: Option<AdSetUpdateStatus>,
    pub optimization_goal: Option<AdSetOptimizationGoal>,
    pub billing_event: Option<AdSetBillingEvent>,
    pub budget: Option<UpdateAdSetBudget>,
    pub targeting: Option<AudienceEstimateTargeting>,
    pub bid: Option<AdSetBid>,
    /// Optional UTC Unix start time, through 2100-01-01.
    #[schemars(range(min = 1, max = 4_102_444_800_u64))]
    pub start_time: Option<u64>,
    /// UTC Unix end time; `0` requests an ongoing daily-budget ad set.
    #[schemars(range(max = 4_102_444_800_u64))]
    pub end_time: Option<u64>,
    pub promoted_object: Option<AdSetPromotedObject>,
    pub destination_type: Option<AdSetDestinationType>,
    pub placement_soft_opt_out: Option<PlacementSoftOptOut>,
    #[schemars(length(min = 1, max = 3))]
    pub attribution_spec: Option<Vec<AttributionSpec>>,
    #[schemars(length(min = 1, max = 4))]
    pub frequency_control_specs: Option<Vec<FrequencyControlSpec>>,
    #[schemars(length(min = 1, max = 512))]
    pub dsa_beneficiary: Option<String>,
    #[schemars(length(min = 1, max = 512))]
    pub dsa_payor: Option<String>,
}

/// Copy an ad set through Meta's `/copies` edge. The copy defaults to `PAUSED`;
/// child ads are copied only when `deep_copy` is explicitly true.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloneAdSetInput {
    /// Expected source ad-account ID, with or without the `act_` prefix.
    #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
    pub ad_account_id: String,
    /// Numeric source ad-set ID.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub ad_set_id: String,
    /// Optional numeric campaign ID for a different parent campaign.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub target_campaign_id: Option<String>,
    /// Copy child ads. Defaults to false and is subject to Meta's copy limits.
    pub deep_copy: Option<bool>,
    /// Status for the copy. Defaults to `PAUSED`.
    #[serde(default)]
    #[schemars(default)]
    pub status: AdSetCopyStatus,
    /// Optional UTC Unix start override; omit to inherit Meta's copy behavior.
    #[schemars(range(min = 1, max = 4_102_444_800_u64))]
    pub start_time: Option<u64>,
    /// Optional UTC Unix end override; omit to inherit Meta's copy behavior.
    #[schemars(range(min = 1, max = 4_102_444_800_u64))]
    pub end_time: Option<u64>,
    /// Optional typed rename behavior.
    pub rename: Option<AdSetCopyRename>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AdSetCreateStatus {
    Active,
    #[default]
    Paused,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AdSetUpdateStatus {
    Active,
    Archived,
    Paused,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AdSetCopyStatus {
    Active,
    InheritedFromSource,
    #[default]
    Paused,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AdSetOptimizationGoal {
    AdRecallLift,
    AppInstalls,
    Conversations,
    EventResponses,
    Impressions,
    LandingPageViews,
    LeadGeneration,
    LinkClicks,
    OffsiteConversions,
    PageLikes,
    PostEngagement,
    QualityLead,
    Reach,
    Thruplay,
    Value,
    VisitInstagramProfile,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AdSetBillingEvent {
    AppInstalls,
    Impressions,
    LinkClicks,
    PageLikes,
    PostEngagement,
    Purchase,
    Thruplay,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AdSetDestinationType {
    App,
    Facebook,
    FacebookPage,
    InstagramDirect,
    InstagramProfile,
    Messenger,
    OnAd,
    OnPage,
    OnPost,
    ShopAutomatic,
    Website,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CreateAdSetBudget {
    /// Daily budget in account-currency minor units.
    Daily {
        #[schemars(range(min = 1))]
        amount: u64,
    },
    /// Lifetime budget in account-currency minor units; requires `end_time`.
    Lifetime {
        #[schemars(range(min = 1))]
        amount: u64,
    },
    /// The parent campaign owns the budget.
    Campaign,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum UpdateAdSetBudget {
    Daily {
        #[schemars(range(min = 1))]
        amount: u64,
    },
    Lifetime {
        #[schemars(range(min = 1))]
        amount: u64,
    },
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "strategy", deny_unknown_fields)]
pub enum AdSetBid {
    #[serde(rename = "COST_CAP")]
    CostCap {
        #[schemars(range(min = 1))]
        amount: u64,
    },
    #[serde(rename = "LOWEST_COST_WITHOUT_CAP")]
    LowestCostWithoutCap,
    #[serde(rename = "LOWEST_COST_WITH_BID_CAP")]
    LowestCostWithBidCap {
        #[schemars(range(min = 1))]
        amount: u64,
    },
    #[serde(rename = "LOWEST_COST_WITH_MIN_ROAS")]
    LowestCostWithMinRoas {
        #[schemars(range(min = 1))]
        roas_average_floor: u32,
    },
}

/// A typed common subset of Meta's objective-dependent promoted-object shapes.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AdSetPromotedObject {
    App {
        #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
        application_id: String,
        #[schemars(length(min = 1, max = 2048), url)]
        object_store_url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        custom_event_type: Option<CustomEventType>,
    },
    Pixel {
        #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
        pixel_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        custom_event_type: Option<CustomEventType>,
    },
    Page {
        #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
        page_id: String,
    },
    ProductSet {
        #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
        product_set_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        custom_event_type: Option<CustomEventType>,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CustomEventType {
    AddPaymentInfo,
    AddToCart,
    AddToWishlist,
    CompleteRegistration,
    Contact,
    ContentView,
    InitiatedCheckout,
    Lead,
    Other,
    Purchase,
    Search,
    StartTrial,
    SubmitApplication,
    Subscribe,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlacementSoftOptOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 32))]
    pub facebook_positions: Option<Vec<crate::audience_estimate::FacebookPosition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 32))]
    pub instagram_positions: Option<Vec<crate::audience_estimate::InstagramPosition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 32))]
    pub threads_positions: Option<Vec<crate::audience_estimate::ThreadsPosition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 32))]
    pub audience_network_positions: Option<Vec<crate::audience_estimate::AudienceNetworkPosition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 32))]
    pub messenger_positions: Option<Vec<crate::audience_estimate::MessengerPosition>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttributionSpec {
    pub event_type: AttributionEventType,
    /// Provider-dependent window from 1 through 28 days.
    #[schemars(range(min = 1, max = 28))]
    pub window_days: u8,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AttributionEventType {
    ClickThrough,
    EngagedVideoView,
    ViewThrough,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FrequencyControlSpec {
    /// Meta v26 currently supports `IMPRESSIONS` for write operations.
    pub event: FrequencyControlEvent,
    /// Frequency interval, from 1 through 90 days.
    #[schemars(range(min = 1, max = 90))]
    pub interval_days: u8,
    /// Maximum frequency, from 1 through 90.
    #[schemars(range(min = 1, max = 90))]
    pub max_frequency: u8,
    #[serde(rename = "type")]
    pub control_type: FrequencyControlType,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FrequencyControlEvent {
    Impressions,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FrequencyControlType {
    Cap,
    None,
    Target,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdSetCopyRename {
    pub strategy: AdSetRenameStrategy,
    /// Optional prefix, from 1 through 100 characters.
    #[schemars(length(min = 1, max = 100))]
    pub prefix: Option<String>,
    /// Optional suffix, from 1 through 100 characters.
    #[schemars(length(min = 1, max = 100))]
    pub suffix: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
pub enum AdSetRenameStrategy {
    #[serde(rename = "DEEP_RENAME")]
    Deep,
    #[serde(rename = "NO_RENAME")]
    None,
    #[serde(rename = "ONLY_TOP_LEVEL_RENAME")]
    TopLevel,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CreatedAdSet {
    pub ad_set_id: String,
    pub status: AdSetCreateStatus,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UpdatedAdSet {
    pub ad_set_id: String,
    pub updated: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ClonedAdSet {
    pub ad_set_id: String,
    pub status: AdSetCopyStatus,
    pub copied_object_count: u16,
}

#[derive(Debug, PartialEq, Eq)]
struct MutationRequest {
    endpoint: String,
    form: Vec<(String, String)>,
}

pub(crate) async fn create_ad_set(
    graph: &GraphClient,
    input: CreateAdSetInput,
) -> ToolResponse<CreatedAdSet> {
    let status = input.status;
    let request = match build_create_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(mutation_error_without_blind_retry(
                error,
                "Check Meta Ads Manager for the ad set before trying again",
            ));
        }
    };
    match created_id(&payload, "ad set") {
        Ok(ad_set_id) => ToolResponse::success(CreatedAdSet { ad_set_id, status }),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn update_ad_set(
    graph: &GraphClient,
    input: UpdateAdSetInput,
) -> ToolResponse<UpdatedAdSet> {
    let request = match build_update_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let ad_set_id = request.endpoint.clone();
    if let Err(error) = verify_meta_node(
        graph,
        &ad_set_id,
        MetaNodeKind::AdSet,
        Some(&input.ad_account_id),
    )
    .await
    {
        return ToolResponse::error(error);
    }
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    match confirmed_update(&payload, &ad_set_id) {
        Ok(()) => ToolResponse::success(UpdatedAdSet {
            ad_set_id,
            updated: true,
        }),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn clone_ad_set(
    graph: &GraphClient,
    input: CloneAdSetInput,
) -> ToolResponse<ClonedAdSet> {
    let status = input.status;
    let request = match build_clone_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let ad_set_id = request
        .endpoint
        .strip_suffix("/copies")
        .expect("validated static suffix");
    if let Err(error) = verify_meta_node(
        graph,
        ad_set_id,
        MetaNodeKind::AdSet,
        Some(&input.ad_account_id),
    )
    .await
    {
        return ToolResponse::error(error);
    }
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(mutation_error_without_blind_retry(
                error,
                "Check Meta Ads Manager for the copied ad set before trying again",
            ));
        }
    };
    match cloned_ad_set(&payload, status) {
        Ok(ad_set) => ToolResponse::success(ad_set),
        Err(error) => ToolResponse::error(error),
    }
}

fn build_create_request(input: &CreateAdSetInput) -> Result<MutationRequest, PublicError> {
    let account_id = normalize_ad_account_id(&input.ad_account_id).ok_or_else(|| {
        PublicError::invalid_input(
            "ad_account_id must be a numeric Meta account ID",
            "Use an ID such as `act_123456789` or `123456789`",
        )
    })?;
    let campaign_id = required_numeric_id(&input.campaign_id, "campaign_id", "list_campaigns")?;
    let name = normalize_text(&input.name, MAX_NAME_CHARS, "name")?;
    validate_time_range(input.start_time, input.end_time, false)?;
    validate_targeting(&input.targeting, true)?;

    let mut form = Vec::with_capacity(18);
    form.push(("name".to_owned(), name));
    form.push(("campaign_id".to_owned(), campaign_id));
    form.push((
        "optimization_goal".to_owned(),
        wire_enum(input.optimization_goal),
    ));
    form.push(("billing_event".to_owned(), wire_enum(input.billing_event)));
    form.push(("status".to_owned(), wire_enum(input.status)));
    append_create_budget(&mut form, &input.budget, input.end_time)?;
    form.push(("targeting".to_owned(), encode_targeting(&input.targeting)?));
    append_bid(&mut form, input.bid.as_ref())?;
    append_timestamp(&mut form, "start_time", input.start_time);
    append_timestamp(&mut form, "end_time", input.end_time);
    append_common_fields(
        &mut form,
        input.promoted_object.as_ref(),
        input.destination_type,
        input.placement_soft_opt_out.as_ref(),
        input.attribution_spec.as_deref(),
        input.frequency_control_specs.as_deref(),
        input.dsa_beneficiary.as_deref(),
        input.dsa_payor.as_deref(),
        Some(input.optimization_goal),
    )?;
    validate_promoted_object_goal(input.optimization_goal, input.promoted_object.as_ref())?;
    if let Some(enabled) = input.is_dynamic_creative {
        form.push(("is_dynamic_creative".to_owned(), enabled.to_string()));
    }

    Ok(MutationRequest {
        endpoint: format!("{account_id}/adsets"),
        form,
    })
}

fn build_update_request(input: &UpdateAdSetInput) -> Result<MutationRequest, PublicError> {
    let ad_set_id = required_numeric_id(&input.ad_set_id, "ad_set_id", "list_ad_sets")?;
    validate_time_range(input.start_time, input.end_time, true)?;

    let mut form = Vec::with_capacity(17);
    if let Some(name) = &input.name {
        form.push((
            "name".to_owned(),
            normalize_text(name, MAX_NAME_CHARS, "name")?,
        ));
    }
    if let Some(status) = input.status {
        form.push(("status".to_owned(), wire_enum(status)));
    }
    if let Some(goal) = input.optimization_goal {
        form.push(("optimization_goal".to_owned(), wire_enum(goal)));
    }
    if let Some(event) = input.billing_event {
        form.push(("billing_event".to_owned(), wire_enum(event)));
    }
    if let Some(budget) = &input.budget {
        append_update_budget(&mut form, budget)?;
    }
    if let Some(targeting) = &input.targeting {
        validate_targeting(targeting, false)?;
        form.push(("targeting".to_owned(), encode_targeting(targeting)?));
    }
    append_bid(&mut form, input.bid.as_ref())?;
    append_timestamp(&mut form, "start_time", input.start_time);
    append_timestamp(&mut form, "end_time", input.end_time);
    append_common_fields(
        &mut form,
        input.promoted_object.as_ref(),
        input.destination_type,
        input.placement_soft_opt_out.as_ref(),
        input.attribution_spec.as_deref(),
        input.frequency_control_specs.as_deref(),
        input.dsa_beneficiary.as_deref(),
        input.dsa_payor.as_deref(),
        input.optimization_goal,
    )?;
    if form.is_empty() {
        return Err(PublicError::invalid_input(
            "at least one ad-set field must be provided",
            "Set a mutable delivery, budget, bidding, targeting, schedule, or compliance field",
        ));
    }

    Ok(MutationRequest {
        endpoint: ad_set_id,
        form,
    })
}

fn build_clone_request(input: &CloneAdSetInput) -> Result<MutationRequest, PublicError> {
    let ad_set_id = required_numeric_id(&input.ad_set_id, "ad_set_id", "list_ad_sets")?;
    validate_time_range(input.start_time, input.end_time, false)?;
    let mut form = vec![
        (
            "deep_copy".to_owned(),
            input.deep_copy.unwrap_or(false).to_string(),
        ),
        ("status_option".to_owned(), wire_enum(input.status)),
    ];
    if let Some(campaign_id) = &input.target_campaign_id {
        form.push((
            "campaign_id".to_owned(),
            required_numeric_id(campaign_id, "target_campaign_id", "list_campaigns")?,
        ));
    }
    append_timestamp(&mut form, "start_time", input.start_time);
    append_timestamp(&mut form, "end_time", input.end_time);
    if let Some(rename) = &input.rename {
        form.push(("rename_options".to_owned(), encode_rename(rename)?));
    }

    Ok(MutationRequest {
        endpoint: format!("{ad_set_id}/copies"),
        form,
    })
}

#[allow(clippy::too_many_arguments)]
fn append_common_fields(
    form: &mut Vec<(String, String)>,
    promoted_object: Option<&AdSetPromotedObject>,
    destination_type: Option<AdSetDestinationType>,
    placement_soft_opt_out: Option<&PlacementSoftOptOut>,
    attribution_spec: Option<&[AttributionSpec]>,
    frequency_control_specs: Option<&[FrequencyControlSpec]>,
    dsa_beneficiary: Option<&str>,
    dsa_payor: Option<&str>,
    optimization_goal: Option<AdSetOptimizationGoal>,
) -> Result<(), PublicError> {
    if let Some(promoted_object) = promoted_object {
        form.push((
            "promoted_object".to_owned(),
            encode_promoted_object(promoted_object)?,
        ));
    }
    if let Some(destination_type) = destination_type {
        form.push(("destination_type".to_owned(), wire_enum(destination_type)));
    }
    if let Some(placement) = placement_soft_opt_out {
        validate_placement_soft_opt_out(placement)?;
        form.push((
            "placement_soft_opt_out".to_owned(),
            encode_nested(placement, "placement_soft_opt_out")?,
        ));
    }
    if let Some(specs) = attribution_spec {
        validate_attribution_specs(specs)?;
        form.push((
            "attribution_spec".to_owned(),
            encode_nested(specs, "attribution_spec")?,
        ));
    }
    if let Some(specs) = frequency_control_specs {
        validate_frequency_specs(specs, optimization_goal)?;
        form.push((
            "frequency_control_specs".to_owned(),
            encode_nested(specs, "frequency_control_specs")?,
        ));
    }
    if let Some(value) = dsa_beneficiary {
        form.push((
            "dsa_beneficiary".to_owned(),
            normalize_text(value, MAX_DSA_CHARS, "dsa_beneficiary")?,
        ));
    }
    if let Some(value) = dsa_payor {
        form.push((
            "dsa_payor".to_owned(),
            normalize_text(value, MAX_DSA_CHARS, "dsa_payor")?,
        ));
    }
    Ok(())
}

fn append_create_budget(
    form: &mut Vec<(String, String)>,
    budget: &CreateAdSetBudget,
    end_time: Option<u64>,
) -> Result<(), PublicError> {
    match budget {
        CreateAdSetBudget::Daily { amount } => {
            let amount = required_amount(*amount, "daily budget")?;
            form.push(("daily_budget".to_owned(), amount.to_string()));
        }
        CreateAdSetBudget::Lifetime { amount } => {
            let amount = required_amount(*amount, "lifetime budget")?;
            if end_time.is_none_or(|value| value == 0) {
                return Err(PublicError::invalid_input(
                    "a lifetime budget requires end_time",
                    "Provide a positive UTC Unix end time",
                ));
            }
            form.push(("lifetime_budget".to_owned(), amount.to_string()));
        }
        CreateAdSetBudget::Campaign => {}
    }
    Ok(())
}

fn append_update_budget(
    form: &mut Vec<(String, String)>,
    budget: &UpdateAdSetBudget,
) -> Result<(), PublicError> {
    let (key, amount) = match budget {
        UpdateAdSetBudget::Daily { amount } => ("daily_budget", *amount),
        UpdateAdSetBudget::Lifetime { amount } => ("lifetime_budget", *amount),
    };
    let amount = required_amount(amount, "ad-set budget")?;
    form.push((key.to_owned(), amount.to_string()));
    Ok(())
}

fn append_bid(form: &mut Vec<(String, String)>, bid: Option<&AdSetBid>) -> Result<(), PublicError> {
    let Some(bid) = bid else {
        return Ok(());
    };
    match bid {
        AdSetBid::LowestCostWithoutCap => {
            form.push((
                "bid_strategy".to_owned(),
                "LOWEST_COST_WITHOUT_CAP".to_owned(),
            ));
        }
        AdSetBid::CostCap { amount } | AdSetBid::LowestCostWithBidCap { amount } => {
            let strategy = if matches!(bid, AdSetBid::CostCap { .. }) {
                "COST_CAP"
            } else {
                "LOWEST_COST_WITH_BID_CAP"
            };
            form.push(("bid_strategy".to_owned(), strategy.to_owned()));
            let amount = required_amount(*amount, "bid amount")?;
            form.push(("bid_amount".to_owned(), amount.to_string()));
        }
        AdSetBid::LowestCostWithMinRoas { roas_average_floor } => {
            if *roas_average_floor == 0 {
                return Err(PublicError::invalid_input(
                    "minimum-ROAS bidding requires a positive roas_average_floor",
                    "Use Meta's scaled integer ROAS floor",
                ));
            }
            form.push((
                "bid_strategy".to_owned(),
                "LOWEST_COST_WITH_MIN_ROAS".to_owned(),
            ));
            let floor = *roas_average_floor;
            let constraints = serde_json::json!({"roas_average_floor": floor});
            form.push((
                "bid_constraints".to_owned(),
                encode_nested(&constraints, "bid_constraints")?,
            ));
        }
    }
    Ok(())
}

fn validate_targeting(
    targeting: &AudienceEstimateTargeting,
    require_advantage_intent: bool,
) -> Result<(), PublicError> {
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
    if require_advantage_intent && targeting.targeting_automation.is_none() {
        return Err(PublicError::invalid_input(
            "ad-set creation requires explicit Advantage+ audience intent",
            "Set targeting.targeting_automation.advantage_audience to true or false",
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
    validate_targeting_lists(targeting)?;
    Ok(())
}

fn validate_ids(ids: &[TargetingId], maximum: usize) -> Result<(), PublicError> {
    if ids.is_empty() || ids.len() > maximum || ids.iter().any(|item| !strict_numeric_id(&item.id))
    {
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

fn validate_targeting_lists(targeting: &AudienceEstimateTargeting) -> Result<(), PublicError> {
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

fn encode_targeting(targeting: &AudienceEstimateTargeting) -> Result<String, PublicError> {
    let mut value = serde_json::to_value(targeting).map_err(|_| {
        PublicError::invalid_input(
            "targeting could not be encoded",
            "Use the typed targeting fields",
        )
    })?;
    if let Some(enabled) = targeting
        .targeting_automation
        .as_ref()
        .map(|automation| automation.advantage_audience)
        && let Some(setting) = value.pointer_mut("/targeting_automation/advantage_audience")
    {
        *setting = Value::from(u8::from(enabled));
    }
    encode_json_value(&value, MAX_TARGETING_BYTES, "targeting")
}

fn validate_placement_soft_opt_out(placement: &PlacementSoftOptOut) -> Result<(), PublicError> {
    let lengths = [
        placement.facebook_positions.as_ref().map(Vec::len),
        placement.instagram_positions.as_ref().map(Vec::len),
        placement.threads_positions.as_ref().map(Vec::len),
        placement.audience_network_positions.as_ref().map(Vec::len),
        placement.messenger_positions.as_ref().map(Vec::len),
    ];
    let present = lengths.into_iter().flatten().collect::<Vec<_>>();
    if present.is_empty()
        || present
            .iter()
            .any(|length| *length == 0 || *length > MAX_PLACEMENTS)
    {
        return Err(PublicError::invalid_input(
            "placement_soft_opt_out must contain bounded nonempty position lists",
            "Provide 1 through 32 current positions in at least one platform field",
        ));
    }
    Ok(())
}

fn validate_attribution_specs(specs: &[AttributionSpec]) -> Result<(), PublicError> {
    if specs.is_empty() || specs.len() > MAX_ATTRIBUTION_SPECS {
        return Err(PublicError::invalid_input(
            "attribution_spec must contain 1 through 3 entries",
            "Provide at most one window for each attribution event type",
        ));
    }
    let mut seen = Vec::with_capacity(specs.len());
    for spec in specs {
        if !(1..=28).contains(&spec.window_days) || seen.contains(&spec.event_type) {
            return Err(PublicError::invalid_input(
                "attribution windows must be unique and from 1 through 28 days",
                "Use one provider-supported window per event type",
            ));
        }
        seen.push(spec.event_type);
    }
    Ok(())
}

fn validate_frequency_specs(
    specs: &[FrequencyControlSpec],
    optimization_goal: Option<AdSetOptimizationGoal>,
) -> Result<(), PublicError> {
    if specs.is_empty() || specs.len() > MAX_FREQUENCY_SPECS {
        return Err(PublicError::invalid_input(
            "frequency_control_specs must contain 1 through 4 entries",
            "Provide a small set of current frequency controls",
        ));
    }
    if optimization_goal.is_some_and(|goal| {
        !matches!(
            goal,
            AdSetOptimizationGoal::Reach | AdSetOptimizationGoal::Thruplay
        )
    }) {
        return Err(PublicError::invalid_input(
            "frequency controls require REACH or THRUPLAY optimization",
            "Remove frequency_control_specs or choose a compatible optimization goal",
        ));
    }
    if specs.iter().any(|spec| {
        !(1..=90).contains(&spec.interval_days) || !(1..=90).contains(&spec.max_frequency)
    }) {
        return Err(PublicError::invalid_input(
            "frequency controls must use values from 1 through 90",
            "Correct interval_days and max_frequency",
        ));
    }
    Ok(())
}

fn encode_promoted_object(object: &AdSetPromotedObject) -> Result<String, PublicError> {
    match object {
        AdSetPromotedObject::App {
            application_id,
            object_store_url,
            ..
        } => {
            validate_promoted_id(application_id)?;
            validate_https_url(object_store_url)?;
        }
        AdSetPromotedObject::Pixel { pixel_id, .. } => validate_promoted_id(pixel_id)?,
        AdSetPromotedObject::Page { page_id } => validate_promoted_id(page_id)?,
        AdSetPromotedObject::ProductSet { product_set_id, .. } => {
            validate_promoted_id(product_set_id)?;
        }
    }
    let mut value = serde_json::to_value(object).map_err(|_| {
        PublicError::invalid_input(
            "promoted_object could not be encoded",
            "Use one typed promoted-object shape",
        )
    })?;
    value
        .as_object_mut()
        .expect("promoted object serializes as an object")
        .remove("kind");
    encode_json_value(&value, MAX_NESTED_BYTES, "promoted_object")
}

fn validate_promoted_id(value: &str) -> Result<(), PublicError> {
    if !strict_numeric_id(value) {
        return Err(PublicError::invalid_input(
            "promoted-object IDs must be numeric Meta IDs",
            "Use IDs returned by Meta read tools",
        ));
    }
    Ok(())
}

fn validate_promoted_object_goal(
    goal: AdSetOptimizationGoal,
    object: Option<&AdSetPromotedObject>,
) -> Result<(), PublicError> {
    if matches!(goal, AdSetOptimizationGoal::AppInstalls)
        && !matches!(object, Some(AdSetPromotedObject::App { .. }))
    {
        return Err(PublicError::invalid_input(
            "app-install optimization requires an app promoted_object",
            "Provide numeric application_id and an HTTPS object_store_url",
        ));
    }
    Ok(())
}

fn encode_rename(rename: &AdSetCopyRename) -> Result<String, PublicError> {
    #[derive(Serialize)]
    struct RenameOptions<'a> {
        rename_strategy: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        rename_prefix: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        rename_suffix: Option<&'a str>,
    }
    let prefix = rename
        .prefix
        .as_deref()
        .map(|value| normalize_affix(value, "rename prefix"))
        .transpose()?;
    let suffix = rename
        .suffix
        .as_deref()
        .map(|value| normalize_affix(value, "rename suffix"))
        .transpose()?;
    if matches!(rename.strategy, AdSetRenameStrategy::None)
        && (prefix.is_some() || suffix.is_some())
    {
        return Err(PublicError::invalid_input(
            "NO_RENAME cannot include a rename prefix or suffix",
            "Remove prefix and suffix or choose a renaming strategy",
        ));
    }
    encode_nested(
        &RenameOptions {
            rename_strategy: wire_enum(rename.strategy),
            rename_prefix: prefix.as_deref(),
            rename_suffix: suffix.as_deref(),
        },
        "rename_options",
    )
}

fn validate_time_range(
    start_time: Option<u64>,
    end_time: Option<u64>,
    allow_zero_end: bool,
) -> Result<(), PublicError> {
    if start_time.is_some_and(|value| value == 0 || value > MAX_UNIX_TIME)
        || end_time.is_some_and(|value| value > MAX_UNIX_TIME || (!allow_zero_end && value == 0))
        || start_time
            .zip(end_time)
            .is_some_and(|(start, end)| end != 0 && end <= start)
    {
        return Err(PublicError::invalid_input(
            "schedule timestamps are invalid or unordered",
            "Use UTC Unix seconds through 2100 with end_time later than start_time",
        ));
    }
    Ok(())
}

fn append_timestamp(form: &mut Vec<(String, String)>, key: &str, value: Option<u64>) {
    if let Some(value) = value {
        form.push((key.to_owned(), value.to_string()));
    }
}

fn required_amount(amount: u64, field: &str) -> Result<u64, PublicError> {
    if amount == 0 || amount > i64::MAX as u64 {
        Err(PublicError::invalid_input(
            format!("{field} must be a positive Meta-compatible integer"),
            "Use account-currency minor units within signed 64-bit range",
        ))
    } else {
        Ok(amount)
    }
}

fn validate_https_url(raw: &str) -> Result<(), PublicError> {
    if raw.len() > MAX_URL_CHARS {
        return Err(invalid_store_url());
    }
    let url = reqwest::Url::parse(raw).map_err(|_| invalid_store_url())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || credential_value(raw)
    {
        return Err(invalid_store_url());
    }
    Ok(())
}

fn invalid_store_url() -> PublicError {
    PublicError::invalid_input(
        "object_store_url must be a bounded HTTPS URL without credentials or fragments",
        "Use the app's canonical Apple, Google, or other store URL",
    )
}

fn wire_enum<T: Serialize>(value: T) -> String {
    match serde_json::to_value(value).expect("enum serialization cannot fail") {
        Value::String(value) => value,
        _ => unreachable!("wire enums serialize as strings"),
    }
}

fn encode_nested<T: Serialize + ?Sized>(value: &T, field: &str) -> Result<String, PublicError> {
    let value = serde_json::to_value(value).map_err(|_| {
        PublicError::invalid_input(
            format!("{field} could not be encoded"),
            "Use the typed nested fields",
        )
    })?;
    encode_json_value(&value, MAX_NESTED_BYTES, field)
}

fn encode_json_value(value: &Value, maximum: usize, field: &str) -> Result<String, PublicError> {
    let encoded = serde_json::to_string(value).map_err(|_| {
        PublicError::invalid_input(
            format!("{field} could not be encoded"),
            "Use the typed nested fields",
        )
    })?;
    if encoded.len() > maximum {
        return Err(PublicError::invalid_input(
            format!("{field} exceeds its {maximum}-byte safety limit"),
            "Narrow the nested object",
        ));
    }
    Ok(encoded)
}

fn normalize_text(raw: &str, maximum: usize, field: &str) -> Result<String, PublicError> {
    let value = raw.trim();
    if value.is_empty() || value.chars().count() > maximum || value.chars().any(char::is_control) {
        return Err(PublicError::invalid_input(
            format!("{field} must contain 1 to {maximum} characters without controls"),
            format!("Provide shorter plain-text {field}"),
        ));
    }
    Ok(value.to_owned())
}

fn normalize_affix(raw: &str, field: &str) -> Result<String, PublicError> {
    if raw.trim().is_empty()
        || raw.chars().count() > MAX_RENAME_CHARS
        || raw.chars().any(char::is_control)
    {
        return Err(PublicError::invalid_input(
            format!("{field} must contain 1 to {MAX_RENAME_CHARS} characters without controls"),
            format!("Provide a shorter plain-text {field}"),
        ));
    }
    Ok(raw.to_owned())
}

fn strict_numeric_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_DIGITS
        && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn required_numeric_id(raw: &str, field: &str, source: &str) -> Result<String, PublicError> {
    normalize_numeric_id(raw).ok_or_else(|| {
        PublicError::invalid_input(
            format!("{field} must be a numeric Meta ID"),
            format!("Use the ID returned by {source}"),
        )
    })
}

fn created_id(payload: &Value, resource: &str) -> Result<String, PublicError> {
    payload
        .get("id")
        .and_then(normalize_numeric_value)
        .ok_or_else(|| ambiguous_result(format!("Meta did not confirm the new {resource} ID")))
}

fn confirmed_update(payload: &Value, ad_set_id: &str) -> Result<(), PublicError> {
    if payload.get("success").and_then(Value::as_bool) == Some(true)
        || payload
            .get("id")
            .and_then(normalize_numeric_value)
            .is_some_and(|id| id == ad_set_id)
    {
        return Ok(());
    }
    Err(ambiguous_result("Meta did not confirm the ad-set update"))
}

fn cloned_ad_set(payload: &Value, status: AdSetCopyStatus) -> Result<ClonedAdSet, PublicError> {
    let ad_set_id = payload
        .get("copied_adset_id")
        .and_then(normalize_numeric_value)
        .ok_or_else(|| ambiguous_result("Meta did not confirm the copied ad-set ID"))?;
    let copied_object_count = match payload.get("ad_object_ids") {
        None | Some(Value::Null) => 0,
        Some(Value::Array(values)) if values.len() <= MAX_COPY_MAPPINGS => {
            u16::try_from(values.len()).expect("copy mapping limit fits u16")
        }
        _ => {
            return Err(ambiguous_result(
                "Meta returned an invalid copied-object summary",
            ));
        }
    };
    Ok(ClonedAdSet {
        ad_set_id,
        status,
        copied_object_count,
    })
}

fn ambiguous_result(message: impl Into<String>) -> PublicError {
    ambiguous_mutation_result(
        message,
        "Verify the result in Meta Ads Manager before retrying",
    )
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{
        AdSetBid, AdSetBillingEvent, AdSetCopyRename, AdSetCopyStatus, AdSetCreateStatus,
        AdSetDestinationType, AdSetOptimizationGoal, AdSetRenameStrategy, AdSetUpdateStatus,
        AttributionEventType, AttributionSpec, CloneAdSetInput, CreateAdSetBudget,
        CreateAdSetInput, PlacementSoftOptOut, UpdateAdSetBudget, UpdateAdSetInput,
        build_clone_request, build_create_request, build_update_request, cloned_ad_set,
    };

    fn targeting() -> crate::audience_estimate::AudienceEstimateTargeting {
        serde_json::from_value(json!({
            "geo_locations": {"countries": ["US"]},
            "age_min": 21,
            "age_max": 45,
            "publisher_platforms": ["facebook"],
            "facebook_positions": ["feed"],
            "targeting_automation": {"advantage_audience": false}
        }))
        .expect("valid targeting")
    }

    #[test]
    fn builds_exact_paused_create_form() {
        let request = build_create_request(&CreateAdSetInput {
            ad_account_id: "act_123".to_owned(),
            campaign_id: "456".to_owned(),
            name: "  Prospecting  ".to_owned(),
            optimization_goal: AdSetOptimizationGoal::LinkClicks,
            billing_event: AdSetBillingEvent::Impressions,
            budget: CreateAdSetBudget::Daily { amount: 2_500 },
            targeting: targeting(),
            status: AdSetCreateStatus::Paused,
            bid: Some(AdSetBid::LowestCostWithBidCap { amount: 300 }),
            start_time: None,
            end_time: None,
            promoted_object: None,
            destination_type: Some(AdSetDestinationType::Website),
            is_dynamic_creative: Some(false),
            placement_soft_opt_out: None,
            attribution_spec: Some(vec![AttributionSpec {
                event_type: AttributionEventType::ClickThrough,
                window_days: 7,
            }]),
            frequency_control_specs: None,
            dsa_beneficiary: Some("ArmaVita LLC".to_owned()),
            dsa_payor: Some("ArmaVita LLC".to_owned()),
        })
        .expect("valid request");

        assert_eq!(request.endpoint, "act_123/adsets");
        assert_eq!(
            request.form,
            vec![
                ("name".to_owned(), "Prospecting".to_owned()),
                ("campaign_id".to_owned(), "456".to_owned()),
                ("optimization_goal".to_owned(), "LINK_CLICKS".to_owned()),
                ("billing_event".to_owned(), "IMPRESSIONS".to_owned()),
                ("status".to_owned(), "PAUSED".to_owned()),
                ("daily_budget".to_owned(), "2500".to_owned()),
                (
                    "targeting".to_owned(),
                    "{\"age_max\":45,\"age_min\":21,\"facebook_positions\":[\"feed\"],\"geo_locations\":{\"countries\":[\"US\"]},\"publisher_platforms\":[\"facebook\"],\"targeting_automation\":{\"advantage_audience\":0}}".to_owned(),
                ),
                (
                    "bid_strategy".to_owned(),
                    "LOWEST_COST_WITH_BID_CAP".to_owned(),
                ),
                ("bid_amount".to_owned(), "300".to_owned()),
                ("destination_type".to_owned(), "WEBSITE".to_owned()),
                (
                    "attribution_spec".to_owned(),
                    "[{\"event_type\":\"CLICK_THROUGH\",\"window_days\":7}]".to_owned(),
                ),
                ("dsa_beneficiary".to_owned(), "ArmaVita LLC".to_owned()),
                ("dsa_payor".to_owned(), "ArmaVita LLC".to_owned()),
                ("is_dynamic_creative".to_owned(), "false".to_owned()),
            ]
        );
    }

    #[test]
    fn builds_exact_update_form() {
        let request = build_update_request(&UpdateAdSetInput {
            ad_account_id: "act_123".to_owned(),
            ad_set_id: "789".to_owned(),
            name: Some("Retargeting".to_owned()),
            status: Some(AdSetUpdateStatus::Paused),
            optimization_goal: None,
            billing_event: None,
            budget: Some(UpdateAdSetBudget::Lifetime { amount: 9_000 }),
            targeting: None,
            bid: Some(AdSetBid::LowestCostWithMinRoas {
                roas_average_floor: 20_000,
            }),
            start_time: Some(1_800_000_000),
            end_time: Some(1_800_086_400),
            promoted_object: None,
            destination_type: None,
            placement_soft_opt_out: None,
            attribution_spec: None,
            frequency_control_specs: None,
            dsa_beneficiary: None,
            dsa_payor: None,
        })
        .expect("valid request");

        assert_eq!(request.endpoint, "789");
        assert_eq!(
            request.form,
            vec![
                ("name".to_owned(), "Retargeting".to_owned()),
                ("status".to_owned(), "PAUSED".to_owned()),
                ("lifetime_budget".to_owned(), "9000".to_owned()),
                (
                    "bid_strategy".to_owned(),
                    "LOWEST_COST_WITH_MIN_ROAS".to_owned(),
                ),
                (
                    "bid_constraints".to_owned(),
                    "{\"roas_average_floor\":20000}".to_owned(),
                ),
                ("start_time".to_owned(), "1800000000".to_owned()),
                ("end_time".to_owned(), "1800086400".to_owned()),
            ]
        );
    }

    #[test]
    fn builds_exact_paused_clone_form() {
        let request = build_clone_request(&CloneAdSetInput {
            ad_account_id: "act_123".to_owned(),
            ad_set_id: "789".to_owned(),
            target_campaign_id: Some("456".to_owned()),
            deep_copy: Some(true),
            status: AdSetCopyStatus::Paused,
            start_time: None,
            end_time: None,
            rename: Some(AdSetCopyRename {
                strategy: AdSetRenameStrategy::Deep,
                prefix: None,
                suffix: Some(" - August".to_owned()),
            }),
        })
        .expect("valid request");

        assert_eq!(request.endpoint, "789/copies");
        assert_eq!(
            request.form,
            vec![
                ("deep_copy".to_owned(), "true".to_owned()),
                ("status_option".to_owned(), "PAUSED".to_owned()),
                ("campaign_id".to_owned(), "456".to_owned()),
                (
                    "rename_options".to_owned(),
                    "{\"rename_strategy\":\"DEEP_RENAME\",\"rename_suffix\":\" - August\"}"
                        .to_owned(),
                ),
            ]
        );
    }

    #[test]
    fn rejects_implicit_targeting_invalid_bids_and_empty_updates() {
        let input: CreateAdSetInput = serde_json::from_value(json!({
            "ad_account_id": "123",
            "campaign_id": "456",
            "name": "Test",
            "optimization_goal": "LINK_CLICKS",
            "billing_event": "IMPRESSIONS",
            "budget": {"kind": "campaign"},
            "targeting": {"geo_locations": {"countries": ["US"]}}
        }))
        .expect("closed valid shape");
        assert!(build_create_request(&input).is_err());

        let malformed_bid = json!({
            "ad_account_id": "123",
            "campaign_id": "456",
            "name": "Test",
            "optimization_goal": "LINK_CLICKS",
            "billing_event": "IMPRESSIONS",
            "budget": {"kind": "campaign"},
            "targeting": {
                "geo_locations": {"countries": ["US"]},
                "targeting_automation": {"advantage_audience": false}
            },
            "bid": {"strategy": "COST_CAP"}
        });
        assert!(serde_json::from_value::<CreateAdSetInput>(malformed_bid).is_err());

        let empty: UpdateAdSetInput =
            serde_json::from_value(json!({"ad_account_id": "123", "ad_set_id": "789"}))
                .expect("closed valid shape");
        assert!(build_update_request(&empty).is_err());
    }

    #[test]
    fn rejects_removed_placements_credentials_and_unbounded_lists() {
        let base = json!({
            "ad_account_id": "123",
            "campaign_id": "456",
            "name": "Test",
            "optimization_goal": "LINK_CLICKS",
            "billing_event": "IMPRESSIONS",
            "budget": {"kind": "campaign"},
            "targeting": {
                "geo_locations": {"countries": ["US"]},
                "targeting_automation": {"advantage_audience": true}
            }
        });

        let mut value = base.clone();
        value["targeting"]["instagram_positions"] = json!(["explore"]);
        assert!(serde_json::from_value::<CreateAdSetInput>(value).is_err());

        let mut value = base.clone();
        value["meta_access_token"] = json!("secret");
        assert!(serde_json::from_value::<CreateAdSetInput>(value).is_err());

        let mut value = base;
        value["placement_soft_opt_out"] = json!({
            "facebook_positions": vec!["feed"; 33]
        });
        let input = serde_json::from_value::<CreateAdSetInput>(value).expect("typed positions");
        assert!(build_create_request(&input).is_err());
    }

    #[test]
    fn encodes_typed_soft_opt_out_and_validates_copy_response() {
        let placement: PlacementSoftOptOut = serde_json::from_value(json!({
            "facebook_positions": ["marketplace"],
            "instagram_positions": ["stream"]
        }))
        .expect("current positions");
        assert_eq!(
            serde_json::to_value(placement).expect("serialize"),
            json!({
                "facebook_positions": ["marketplace"],
                "instagram_positions": ["stream"]
            })
        );

        let copy = cloned_ad_set(
            &json!({
                "copied_adset_id": "900",
                "ad_object_ids": [
                    {"ad_object_type": "ad_set", "source_id": "789", "copied_id": "900"},
                    {"ad_object_type": "ad", "source_id": "10", "copied_id": "11"}
                ]
            }),
            AdSetCopyStatus::Paused,
        )
        .expect("documented copy response");
        assert_eq!(copy.ad_set_id, "900");
        assert_eq!(copy.status, AdSetCopyStatus::Paused);
        assert_eq!(copy.copied_object_count, 2);
    }

    #[test]
    fn active_create_status_is_explicit() {
        let status = AdSetCreateStatus::Active;
        let value = serde_json::to_value(status).expect("serialize status");
        assert_eq!(value, Value::String("ACTIVE".to_owned()));
    }
}
