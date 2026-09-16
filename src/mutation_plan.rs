// Copyright (C) 2025 ArmaVita LLC
// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use reqwest::Url;
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use subtle::ConstantTimeEq;

use crate::{
    bounded_json::{credential_key, credential_value, request_control_key},
    error::{GraphError, PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{ad_account, numeric_owned},
    mutation_result::mutation_error_without_blind_retry,
    node_identity::{MetaNodeKind, verify_meta_node},
    safety::validate_removal_acknowledgement,
};

pub(crate) const APPLY_ACKNOWLEDGEMENT: &str = "APPLY_LIVE_META_ADS_CHANGES";

const PLAN_TTL: Duration = Duration::from_secs(15 * 60);
const APPLY_LEASE: Duration = Duration::from_secs(60);
const TERMINAL_TOMBSTONE_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_PLANS: usize = 32;
const MAX_REASON_CHARS: usize = 256;
const MAX_CALLER_FORM_PAIRS: usize = 63;
const MAX_FORM_PAIRS: usize = 64;
const MAX_SERIALIZED_BYTES: usize = 64 * 1024;
const MAX_JSON_DEPTH: usize = 8;
const MAX_JSON_NODES: usize = 1_024;
const MAX_OBJECT_KEYS: usize = 128;
const MAX_ARRAY_ITEMS: usize = 256;
const MAX_KEY_BYTES: usize = 128;
const MAX_TOP_LEVEL_KEY_BYTES: usize = 64;
const MAX_STRING_BYTES: usize = 16 * 1024;
const MAX_PREVIEW_SCALARS: usize = 16;
const MAX_PREVIEW_PATH_CHARS: usize = 256;
const MAX_PREVIEW_STRING_CHARS: usize = 128;
const EXECUTION_OPTIONS: &str = "[\"validate_only\"]";

// v26.0 AdAccount.create_custom_audience codegen. The generated WhatsApp field remains in the
// source-of-truth list, but the server-wide no-WhatsApp policy rejects it before this allowlist.
const CUSTOM_AUDIENCE_CREATE_FIELDS: &[&str] = &[
    "allowed_domains",
    "associated_audience_id",
    "audience_labels",
    "claim_objective",
    "content_type",
    "countries",
    "creation_params",
    "customer_file_source",
    "dataset_id",
    "description",
    "enable_fetch_or_create",
    "event_source_group",
    "event_sources",
    "exclusions",
    "facebook_page_id",
    "inclusionOperator",
    "inclusions",
    "is_snapshot",
    "is_value_based",
    "list_of_accounts",
    "lookalike_spec",
    "marketing_message_channels",
    "name",
    "opt_out_link",
    "origin_audience_id",
    "parent_audience_id",
    "partner_reference_key",
    "pixel_id",
    "prefill",
    "product_set_id",
    "regulated_audience_spec",
    "retention_days",
    "rev_share_policy_id",
    "rule",
    "rule_aggregation",
    "subscription_info",
    "subtype",
    "usage_restriction",
    "use_for_products",
    "use_in_campaigns",
    "video_group_ids",
    "whats_app_business_phone_number_id",
];

// v26.0 CustomAudience.api_update codegen.
const CUSTOM_AUDIENCE_UPDATE_FIELDS: &[&str] = &[
    "acting_account_id",
    "allowed_domains",
    "audience_labels",
    "claim_objective",
    "content_type",
    "countries",
    "customer_file_source",
    "description",
    "enable_fetch_or_create",
    "event_source_group",
    "event_sources",
    "exclusions",
    "inclusionOperator",
    "inclusions",
    "lookalike_spec",
    "name",
    "opt_out_link",
    "parent_audience_id",
    "product_set_id",
    "retention_days",
    "rev_share_policy_id",
    "rule",
    "rule_aggregation",
    "tags",
    "use_for_products",
    "use_in_campaigns",
];

// v26.0 AdCreative.api_update is intentionally narrow: creative content is immutable after
// creation. execution_options is server-owned and appended only to the validation request.
const AD_CREATIVE_UPDATE_FIELDS: &[&str] = &["account_id", "adlabels", "name", "status"];

// v26.0 AdAccount.create_reach_frequency_prediction codegen.
const REACH_FREQUENCY_CREATE_FIELDS: &[&str] = &[
    "action",
    "ad_formats",
    "auction_entry_option_index",
    "budget",
    "buying_type",
    "campaign_group_id",
    "day_parting_schedule",
    "deal_id",
    "destination_id",
    "destination_ids",
    "end_time",
    "exceptions",
    "existing_campaign_id",
    "expiration_time",
    "frequency_cap",
    "grp_buying",
    "impression",
    "instream_packages",
    "interval_frequency_cap_reset_period",
    "is_balanced_frequency",
    "is_bonus_media",
    "is_conversion_goal",
    "is_full_view",
    "is_higher_average_frequency",
    "is_reach_and_frequency_io_buying",
    "is_reserved_buying",
    "meta_moment_maker_spec",
    "num_curve_points",
    "objective",
    "optimization_goal",
    "prediction_mode",
    "reach",
    "rf_prediction_id",
    "rf_prediction_id_to_release",
    "rf_prediction_id_to_share",
    "start_time",
    "stop_time",
    "story_event_type",
    "target_cpm",
    "target_frequency",
    "target_frequency_reset_period",
    "target_spec",
    "trending_topics_spec",
    "video_view_length_constraint",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MutationResourceType {
    Campaign,
    AdSet,
    Ad,
    AdCreative,
    CustomAudience,
    ReachFrequencyPrediction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MutationActionKind {
    Create,
    Update,
    Delete,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MutationActionInput {
    Create {
        resource_type: MutationResourceType,
        /// Numeric Meta ad-account ID, with or without `act_`.
        #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
        ad_account_id: String,
        /// Official Graph v26 form fields. Credentials, excluded messaging fields, and execution_options are rejected.
        #[schemars(schema_with = "mutation_fields_schema")]
        fields: Map<String, Value>,
    },
    Update {
        resource_type: MutationResourceType,
        /// Numeric Meta ad-account ID, with or without `act_`.
        #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
        ad_account_id: String,
        /// Numeric Meta object ID.
        #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
        object_id: String,
        /// Official Graph v26 form fields. Credentials, excluded messaging fields, and execution_options are rejected.
        #[schemars(schema_with = "mutation_fields_schema")]
        fields: Map<String, Value>,
    },
    Delete {
        resource_type: MutationResourceType,
        /// Numeric Meta ad-account ID, with or without `act_`.
        #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
        ad_account_id: String,
        /// Numeric Meta object ID.
        #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
        object_id: String,
    },
}

fn mutation_fields_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    let mut schema = Map::<String, Value>::json_schema(generator);
    schema.insert("minProperties".into(), 1_u64.into());
    schema.insert(
        "maxProperties".into(),
        (MAX_CALLER_FORM_PAIRS as u64).into(),
    );
    schema
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BuildMutationPlanInput {
    /// Short operator-visible reason retained with the local plan preview.
    #[schemars(length(min = 1, max = 256))]
    pub reason: String,
    /// Exactly one frozen Graph mutation request. A plan is never described as atomic.
    pub action: MutationActionInput,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetMutationPlanInput {
    #[schemars(length(min = 32, max = 32), regex(pattern = "^[0-9a-f]{32}$"))]
    pub plan_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyMutationPlanInput {
    #[schemars(length(min = 32, max = 32), regex(pattern = "^[0-9a-f]{32}$"))]
    pub plan_id: String,
    #[schemars(length(min = 64, max = 64), regex(pattern = "^[0-9a-f]{64}$"))]
    pub confirmation_token: String,
    /// Exact explicit approval phrase returned in the build guidance.
    #[schemars(
        length(min = 27, max = 27),
        regex(pattern = "^APPLY_LIVE_META_ADS_CHANGES$")
    )]
    pub apply_acknowledgement: String,
    /// Required for deletes, status=DELETED, and reach/frequency cancellation/release actions.
    #[schemars(
        length(min = 25, max = 25),
        regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
    )]
    pub removal_acknowledgement: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscardMutationPlanInput {
    #[schemars(length(min = 32, max = 32), regex(pattern = "^[0-9a-f]{32}$"))]
    pub plan_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Pending,
    Applying,
    Applied,
    OutcomeUnknown,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ScalarPreview {
    pub path: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MutationPlanSummary {
    pub reason: String,
    pub action: MutationActionKind,
    pub resource_type: MutationResourceType,
    pub target_id: String,
    pub target_account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_target_name: Option<String>,
    pub target_identity_verified: bool,
    pub request_count: u8,
    pub provider_validated: bool,
    pub destructive: bool,
    pub elevated_risk: bool,
    pub removal_acknowledgement_required: bool,
    pub field_count: u8,
    pub field_names: Vec<String>,
    pub omitted_field_name_count: u8,
    pub preview: Vec<ScalarPreview>,
    pub omitted_preview_scalar_count: u16,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MutationPlanView {
    pub plan_id: String,
    pub status: PlanStatus,
    pub expires_in_seconds: u16,
    pub summary: MutationPlanSummary,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MutationPlanReceipt {
    pub plan: MutationPlanView,
    pub confirmation_token: String,
    pub apply_acknowledgement: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removal_acknowledgement: Option<&'static str>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AppliedMutationPlan {
    pub plan_id: String,
    pub status: PlanStatus,
    pub action: MutationActionKind,
    pub resource_type: MutationResourceType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    pub provider_confirmed: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DiscardedMutationPlan {
    pub plan_id: String,
    pub discarded: bool,
}

#[derive(Clone)]
pub(crate) struct MutationPlanStore {
    state: Arc<Mutex<HashMap<String, StoredPlan>>>,
}

#[derive(Clone)]
struct StoredPlan {
    token: [u8; 32],
    expires_at: Instant,
    apply_lease_until: Option<Instant>,
    status: PlanStatus,
    plan: Arc<FrozenPlan>,
}

#[derive(Clone)]
struct FrozenPlan {
    request: Arc<FrozenRequest>,
    summary: Arc<MutationPlanSummary>,
}

#[derive(Clone)]
struct FrozenRequest {
    method: RequestMethod,
    endpoint: Arc<str>,
    form: Arc<[(String, String)]>,
}

#[derive(Clone, Copy)]
enum RequestMethod {
    Post,
    Delete,
}

struct PreparedPlan {
    request: FrozenRequest,
    summary: MutationPlanSummary,
    validation_form: Option<Vec<(String, String)>>,
}

struct PlanTarget {
    object_id: String,
    account_id: String,
}

struct ApplyHandle {
    plan_id: String,
    plan: Arc<FrozenPlan>,
}

#[derive(Clone, Copy)]
enum ApplyFinish {
    Applied,
    Pending,
    OutcomeUnknown,
}

impl MutationPlanStore {
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn insert(&self, prepared: PreparedPlan) -> Result<MutationPlanReceipt, PublicError> {
        let mut plans = self.lock()?;
        purge_expired(&mut plans);
        if plans.len() >= MAX_PLANS {
            evict_oldest_applied(&mut plans);
        }
        if plans.len() >= MAX_PLANS {
            return Err(plan_error(
                "The local Meta mutation-plan capacity is full",
                "Discard unused plans or wait for their 15-minute expiry",
            ));
        }

        let mut plan_id = random_hex::<16>()?;
        for _ in 0..3 {
            if !plans.contains_key(&plan_id) {
                break;
            }
            plan_id = random_hex::<16>()?;
        }
        if plans.contains_key(&plan_id) {
            return Err(internal_error(
                "Secure plan ID generation collided repeatedly",
            ));
        }

        let token = random_bytes::<32>()?;
        let plan = Arc::new(FrozenPlan {
            request: Arc::new(prepared.request),
            summary: Arc::new(prepared.summary),
        });
        let stored = StoredPlan {
            token,
            expires_at: Instant::now() + PLAN_TTL,
            apply_lease_until: None,
            status: PlanStatus::Pending,
            plan,
        };
        let view = plan_view(&plan_id, &stored)?;
        let removal_acknowledgement = stored
            .plan
            .summary
            .removal_acknowledgement_required
            .then_some(crate::safety::REMOVAL_ACKNOWLEDGEMENT);
        plans.insert(plan_id, stored);
        Ok(MutationPlanReceipt {
            plan: view,
            confirmation_token: hex_lower(&token),
            apply_acknowledgement: APPLY_ACKNOWLEDGEMENT,
            removal_acknowledgement,
        })
    }

    fn get(&self, plan_id: &str) -> Result<MutationPlanView, PublicError> {
        validate_plan_id(plan_id)?;
        let mut plans = self.lock()?;
        purge_expired(&mut plans);
        let stored = plans.get(plan_id).ok_or_else(plan_not_found)?;
        plan_view(plan_id, stored)
    }

    fn begin_apply(&self, input: &ApplyMutationPlanInput) -> Result<ApplyHandle, PublicError> {
        validate_apply_acknowledgement(&input.apply_acknowledgement)?;
        validate_plan_id(&input.plan_id)?;
        let token = decode_token(&input.confirmation_token)?;
        let mut plans = self.lock()?;
        purge_expired(&mut plans);
        let stored = plans.get_mut(&input.plan_id).ok_or_else(plan_not_found)?;
        if !bool::from(stored.token.ct_eq(&token)) {
            return Err(plan_confirmation_error());
        }
        validate_removal_acknowledgement(
            stored.plan.summary.removal_acknowledgement_required,
            input.removal_acknowledgement.as_deref(),
        )?;
        if stored.status != PlanStatus::Pending {
            return Err(plan_error(
                "The Meta mutation plan is not pending",
                "Inspect its status; never retry an applied or outcome-unknown write blindly",
            ));
        }
        stored.status = PlanStatus::Applying;
        stored.apply_lease_until = Some(Instant::now() + APPLY_LEASE);
        Ok(ApplyHandle {
            plan_id: input.plan_id.clone(),
            plan: Arc::clone(&stored.plan),
        })
    }

    fn finish(&self, plan_id: &str, finish: ApplyFinish) -> Result<(), PublicError> {
        let mut plans = self.lock()?;
        let stored = plans.get_mut(plan_id).ok_or_else(plan_not_found)?;
        let now = Instant::now();
        if transition_stale_apply(stored, now) {
            return Err(stale_apply_error());
        }
        if stored.status != PlanStatus::Applying {
            return Err(plan_error(
                "The Meta mutation plan is no longer applying",
                "Inspect its status and reconcile current provider state before any retry",
            ));
        }
        stored.status = match finish {
            ApplyFinish::Applied => PlanStatus::Applied,
            ApplyFinish::Pending => PlanStatus::Pending,
            ApplyFinish::OutcomeUnknown => PlanStatus::OutcomeUnknown,
        };
        stored.apply_lease_until = None;
        if matches!(finish, ApplyFinish::Applied | ApplyFinish::OutcomeUnknown) {
            stored.expires_at = now + TERMINAL_TOMBSTONE_TTL;
        }
        Ok(())
    }

    fn discard(&self, plan_id: &str) -> Result<DiscardedMutationPlan, PublicError> {
        validate_plan_id(plan_id)?;
        let mut plans = self.lock()?;
        purge_expired(&mut plans);
        let stored = plans.get(plan_id).ok_or_else(plan_not_found)?;
        if stored.status == PlanStatus::Applying {
            return Err(plan_error(
                "The Meta mutation plan is currently applying",
                "Wait for the in-flight request to finish before discarding it",
            ));
        }
        if matches!(
            stored.status,
            PlanStatus::Applied | PlanStatus::OutcomeUnknown
        ) {
            return Err(plan_error(
                "A terminal Meta mutation-plan tombstone cannot be discarded",
                "Reconcile current provider state; the tombstone expires automatically after 15 minutes",
            ));
        }
        plans.remove(plan_id).ok_or_else(plan_not_found)?;
        Ok(DiscardedMutationPlan {
            plan_id: plan_id.to_owned(),
            discarded: true,
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, HashMap<String, StoredPlan>>, PublicError> {
        self.state
            .lock()
            .map_err(|_| internal_error("The local mutation-plan store is unavailable"))
    }
}

impl std::fmt::Debug for MutationPlanStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MutationPlanStore")
            .finish_non_exhaustive()
    }
}

pub(crate) async fn build_mutation_plan(
    graph: &GraphClient,
    plans: &MutationPlanStore,
    input: BuildMutationPlanInput,
) -> ToolResponse<MutationPlanReceipt> {
    let mut prepared = match prepare_plan(input) {
        Ok(prepared) => prepared,
        Err(error) => return ToolResponse::error(error),
    };
    if prepared.summary.action != MutationActionKind::Create {
        let Some(kind) = prepared.summary.resource_type.node_kind() else {
            return ToolResponse::error(internal_error(
                "The mutation-plan resource has no identity proof",
            ));
        };
        let verified = match verify_meta_node(
            graph,
            &prepared.summary.target_id,
            kind,
            Some(&prepared.summary.target_account_id),
        )
        .await
        {
            Ok(verified) => verified,
            Err(error) => return ToolResponse::error(error),
        };
        prepared.summary.target_id = verified.object_id;
        prepared.summary.target_account_id = verified.account_id;
        prepared.summary.verified_target_name = verified.name;
        prepared.summary.target_identity_verified = true;
    }
    if let Some(validation_form) = prepared.validation_form.as_deref() {
        let payload = match graph
            .post_form_json(prepared.request.endpoint.as_ref(), validation_form)
            .await
        {
            Ok(payload) => payload,
            Err(error) => return ToolResponse::error(PublicError::from(error)),
        };
        if !validation_confirmed(&payload) {
            return ToolResponse::error(PublicError::invalid_upstream(
                "Meta did not confirm the validate-only mutation request",
            ));
        }
    }
    match plans.insert(prepared) {
        Ok(receipt) => ToolResponse::success(receipt),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) fn get_mutation_plan(
    plans: &MutationPlanStore,
    input: GetMutationPlanInput,
) -> ToolResponse<MutationPlanView> {
    match plans.get(&input.plan_id) {
        Ok(plan) => ToolResponse::success(plan),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn apply_mutation_plan(
    graph: &GraphClient,
    plans: &MutationPlanStore,
    input: ApplyMutationPlanInput,
) -> ToolResponse<AppliedMutationPlan> {
    let handle = match plans.begin_apply(&input) {
        Ok(handle) => handle,
        Err(error) => return ToolResponse::error(error),
    };
    let request = &handle.plan.request;
    let result = match request.method {
        RequestMethod::Post => {
            graph
                .post_form_json(request.endpoint.as_ref(), request.form.as_ref())
                .await
        }
        RequestMethod::Delete => graph.delete_json(request.endpoint.as_ref(), &[]).await,
    };

    let payload = match result {
        Ok(payload) => payload,
        Err(error) => {
            let ambiguous = ambiguous_graph_error(&error);
            let finish = if ambiguous {
                ApplyFinish::OutcomeUnknown
            } else {
                ApplyFinish::Pending
            };
            if let Err(store_error) = plans.finish(&handle.plan_id, finish) {
                return ToolResponse::error(store_error);
            }
            let public = if ambiguous {
                mutation_error_without_blind_retry(
                    error,
                    "Read the target from Meta and reconcile it before building another plan",
                )
            } else {
                PublicError::from(error)
            };
            return ToolResponse::error(public);
        }
    };

    let Some(object_id) = confirmed_result(
        &payload,
        handle.plan.summary.action,
        request.endpoint.as_ref(),
    ) else {
        if let Err(error) = plans.finish(&handle.plan_id, ApplyFinish::OutcomeUnknown) {
            return ToolResponse::error(error);
        }
        return ToolResponse::error(ambiguous_result());
    };
    if let Err(error) = plans.finish(&handle.plan_id, ApplyFinish::Applied) {
        return ToolResponse::error(error);
    }
    ToolResponse::success(AppliedMutationPlan {
        plan_id: handle.plan_id,
        status: PlanStatus::Applied,
        action: handle.plan.summary.action,
        resource_type: handle.plan.summary.resource_type,
        object_id,
        provider_confirmed: true,
    })
}

pub(crate) fn discard_mutation_plan(
    plans: &MutationPlanStore,
    input: DiscardMutationPlanInput,
) -> ToolResponse<DiscardedMutationPlan> {
    match plans.discard(&input.plan_id) {
        Ok(discarded) => ToolResponse::success(discarded),
        Err(error) => ToolResponse::error(error),
    }
}

fn prepare_plan(input: BuildMutationPlanInput) -> Result<PreparedPlan, PublicError> {
    let reason = bounded_reason(&input.reason)?;
    let mut prepared = match input.action {
        MutationActionInput::Create {
            resource_type,
            ad_account_id,
            fields,
        } => prepare_write(
            MutationActionKind::Create,
            resource_type,
            ad_account_id,
            None,
            fields,
        ),
        MutationActionInput::Update {
            resource_type,
            ad_account_id,
            object_id,
            fields,
        } => prepare_write(
            MutationActionKind::Update,
            resource_type,
            ad_account_id,
            Some(object_id),
            fields,
        ),
        MutationActionInput::Delete {
            resource_type,
            ad_account_id,
            object_id,
        } => prepare_delete(resource_type, ad_account_id, object_id),
    }?;
    prepared.summary.reason = reason;
    Ok(prepared)
}

fn prepare_write(
    action: MutationActionKind,
    resource_type: MutationResourceType,
    account_id: String,
    object_id: Option<String>,
    mut fields: Map<String, Value>,
) -> Result<PreparedPlan, PublicError> {
    if resource_type == MutationResourceType::ReachFrequencyPrediction
        && action != MutationActionKind::Create
    {
        return Err(unsupported_action(resource_type));
    }
    if fields.is_empty() {
        return Err(invalid("fields must contain at least one Graph v26 field"));
    }
    if fields.len() > MAX_CALLER_FORM_PAIRS {
        return Err(invalid(
            "fields exceeds the 63-pair caller limit; one slot is reserved for server safety",
        ));
    }
    if fields
        .get("status")
        .is_some_and(|status| !status.is_string())
    {
        return Err(invalid("status must be a string Graph enum value"));
    }
    validate_dynamic_fields(&fields)?;
    validate_resource_fields(resource_type, action, &fields)?;

    if action == MutationActionKind::Create
        && matches!(
            resource_type,
            MutationResourceType::Campaign | MutationResourceType::AdSet | MutationResourceType::Ad
        )
    {
        if !fields.contains_key("status") && fields.len() == MAX_CALLER_FORM_PAIRS {
            return Err(invalid(
                "a 63-field delivery create must include status=PAUSED because no injection slot remains",
            ));
        }
        match fields.get("status") {
            None => {
                fields.insert("status".to_owned(), Value::String("PAUSED".to_owned()));
            }
            Some(Value::String(status)) if status == "PAUSED" => {}
            Some(Value::String(status)) if status == "ACTIVE" => {
                return Err(invalid(
                    "campaign, ad-set, and ad creates cannot start ACTIVE; use PAUSED",
                ));
            }
            Some(_) => {
                return Err(invalid(
                    "campaign, ad-set, and ad creates require status=PAUSED",
                ));
            }
        }
    }

    // Revalidate the server-injected PAUSED node and all final sizing limits.
    validate_dynamic_fields(&fields)?;
    let account = ad_account(&account_id)
        .ok_or_else(|| invalid("ad_account_id must be a numeric Meta account ID"))?;
    let endpoint = match action {
        MutationActionKind::Create => format!("{account}/{}", resource_type.create_edge()),
        MutationActionKind::Update => numeric_owned(object_id.as_deref().unwrap_or_default())
            .ok_or_else(|| invalid("object_id must be a numeric Meta object ID"))?,
        MutationActionKind::Delete => unreachable!("writes contain create or update only"),
    };
    let form = form_from_fields(&fields)?;
    let provider_validated = resource_type.provider_validates(action);
    let validation_form = provider_validated
        .then(|| validation_form(&form))
        .transpose()?;
    validate_form(&form)?;

    let (destructive, elevated_risk) = risk_flags(action, resource_type, &fields)?;
    let summary = plan_summary(
        action,
        resource_type,
        PlanTarget {
            object_id: endpoint_target(&endpoint),
            account_id: account,
        },
        provider_validated,
        destructive,
        elevated_risk,
        &fields,
    );
    Ok(PreparedPlan {
        request: FrozenRequest {
            method: RequestMethod::Post,
            endpoint: Arc::from(endpoint),
            form: Arc::from(form),
        },
        summary,
        validation_form,
    })
}

fn prepare_delete(
    resource_type: MutationResourceType,
    account_id: String,
    object_id: String,
) -> Result<PreparedPlan, PublicError> {
    if resource_type == MutationResourceType::ReachFrequencyPrediction {
        return Err(unsupported_action(resource_type));
    }
    let endpoint = numeric_owned(&object_id)
        .ok_or_else(|| invalid("object_id must be a numeric Meta object ID"))?;
    let account = ad_account(&account_id)
        .ok_or_else(|| invalid("ad_account_id must be a numeric Meta account ID"))?;
    Ok(PreparedPlan {
        request: FrozenRequest {
            method: RequestMethod::Delete,
            endpoint: Arc::from(endpoint.clone()),
            form: Arc::from([]),
        },
        summary: MutationPlanSummary {
            reason: String::new(),
            action: MutationActionKind::Delete,
            resource_type,
            target_id: endpoint,
            target_account_id: account,
            verified_target_name: None,
            target_identity_verified: false,
            request_count: 1,
            provider_validated: false,
            destructive: true,
            elevated_risk: false,
            removal_acknowledgement_required: true,
            field_count: 0,
            field_names: Vec::new(),
            omitted_field_name_count: 0,
            preview: Vec::new(),
            omitted_preview_scalar_count: 0,
        },
        validation_form: None,
    })
}

impl MutationResourceType {
    fn create_edge(self) -> &'static str {
        match self {
            Self::Campaign => "campaigns",
            Self::AdSet => "adsets",
            Self::Ad => "ads",
            Self::AdCreative => "adcreatives",
            Self::CustomAudience => "customaudiences",
            Self::ReachFrequencyPrediction => "reachfrequencypredictions",
        }
    }

    fn provider_validates(self, action: MutationActionKind) -> bool {
        action != MutationActionKind::Delete
            && matches!(
                self,
                Self::Campaign | Self::AdSet | Self::Ad | Self::AdCreative
            )
    }

    fn node_kind(self) -> Option<MetaNodeKind> {
        match self {
            Self::Campaign => Some(MetaNodeKind::Campaign),
            Self::AdSet => Some(MetaNodeKind::AdSet),
            Self::Ad => Some(MetaNodeKind::Ad),
            Self::AdCreative => Some(MetaNodeKind::AdCreative),
            Self::CustomAudience => Some(MetaNodeKind::CustomAudience),
            Self::ReachFrequencyPrediction => None,
        }
    }
}

fn validate_resource_fields(
    resource_type: MutationResourceType,
    action: MutationActionKind,
    fields: &Map<String, Value>,
) -> Result<(), PublicError> {
    let allowlist = match (resource_type, action) {
        (MutationResourceType::CustomAudience, MutationActionKind::Create) => {
            Some(CUSTOM_AUDIENCE_CREATE_FIELDS)
        }
        (MutationResourceType::CustomAudience, MutationActionKind::Update) => {
            Some(CUSTOM_AUDIENCE_UPDATE_FIELDS)
        }
        (MutationResourceType::AdCreative, MutationActionKind::Update) => {
            Some(AD_CREATIVE_UPDATE_FIELDS)
        }
        (MutationResourceType::ReachFrequencyPrediction, MutationActionKind::Create) => {
            Some(REACH_FREQUENCY_CREATE_FIELDS)
        }
        _ => None,
    };
    if let Some(allowlist) = allowlist
        && let Some(field) = fields
            .keys()
            .find(|field| !allowlist.contains(&field.as_str()))
    {
        return Err(invalid(format!(
            "{field} is not an official Graph v26 field for this resource action"
        )));
    }

    if resource_type == MutationResourceType::ReachFrequencyPrediction {
        validate_reach_frequency_action(fields)?;
    }
    Ok(())
}

fn validate_reach_frequency_action(fields: &Map<String, Value>) -> Result<(), PublicError> {
    let action = match fields.get("action") {
        None => "quote",
        Some(Value::String(action))
            if matches!(action.as_str(), "quote" | "reserve" | "cancel") =>
        {
            action
        }
        Some(_) => {
            return Err(invalid(
                "reach/frequency action must be quote, reserve, or cancel",
            ));
        }
    };
    if matches!(action, "reserve" | "cancel") {
        let id = fields
            .get("rf_prediction_id")
            .and_then(Value::as_str)
            .and_then(numeric_owned);
        if id.is_none() {
            return Err(invalid(
                "reach/frequency reserve or cancel requires numeric rf_prediction_id",
            ));
        }
    }
    if fields
        .get("rf_prediction_id_to_release")
        .is_some_and(|value| value.as_str().and_then(numeric_owned).is_none())
    {
        return Err(invalid(
            "rf_prediction_id_to_release must be a numeric Meta prediction ID",
        ));
    }
    Ok(())
}

fn validate_dynamic_fields(fields: &Map<String, Value>) -> Result<(), PublicError> {
    if fields.len() > MAX_FORM_PAIRS {
        return Err(invalid("fields exceeds the 64-pair request limit"));
    }
    let serialized = serde_json::to_vec(fields)
        .map_err(|_| invalid("fields could not be serialized as bounded JSON"))?;
    if serialized.len() > MAX_SERIALIZED_BYTES {
        return Err(invalid("fields exceeds the 64 KiB serialized limit"));
    }
    let mut nodes = 1_usize;
    validate_object(fields, 0, &mut nodes)
}

fn validate_object(
    object: &Map<String, Value>,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), PublicError> {
    if depth > MAX_JSON_DEPTH || object.len() > MAX_OBJECT_KEYS {
        return Err(invalid("fields exceeds the JSON depth or object-key limit"));
    }
    for (key, value) in object {
        if key.is_empty()
            || key.len() > MAX_KEY_BYTES
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        {
            return Err(invalid("fields contains an invalid or oversized key"));
        }
        if depth == 0 && key.len() > MAX_TOP_LEVEL_KEY_BYTES {
            return Err(invalid("a top-level Graph field name exceeds 64 bytes"));
        }
        if depth == 0 && request_control_key(key) {
            return Err(invalid("Graph request-control fields are server-owned"));
        }
        if key.eq_ignore_ascii_case("execution_options") {
            return Err(invalid(
                "execution_options is server-owned and cannot be supplied",
            ));
        }
        if credential_key(key) || whatsapp_text(key) {
            return Err(invalid("credentials and WhatsApp fields are not accepted"));
        }
        validate_value(value, depth + 1, nodes)?;
    }
    Ok(())
}

fn validate_value(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), PublicError> {
    *nodes = nodes.saturating_add(1);
    if *nodes > MAX_JSON_NODES || depth > MAX_JSON_DEPTH {
        return Err(invalid("fields exceeds the 8-depth or 1,024-node limit"));
    }
    match value {
        Value::Object(object) => validate_object(object, depth, nodes),
        Value::Array(items) => {
            if items.len() > MAX_ARRAY_ITEMS {
                return Err(invalid("a fields array exceeds 256 items"));
            }
            for item in items {
                validate_value(item, depth + 1, nodes)?;
            }
            Ok(())
        }
        Value::String(text) => {
            if text.len() > MAX_STRING_BYTES {
                return Err(invalid("a fields string exceeds 16 KiB"));
            }
            if credential_value(text)
                || (text.len() >= 40
                    && text.starts_with("EAA")
                    && text.bytes().all(|byte| byte.is_ascii_alphanumeric()))
                || whatsapp_text(text)
            {
                return Err(invalid(
                    "credentials and WhatsApp-specific values are not accepted",
                ));
            }
            Ok(())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
    }
}

fn whatsapp_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("whatsapp")
        || lower.contains("whats_app")
        || lower.contains("whats-app")
        || lower.contains("wa.me/")
}

fn form_from_fields(fields: &Map<String, Value>) -> Result<Vec<(String, String)>, PublicError> {
    let mut keys = fields.keys().collect::<Vec<_>>();
    keys.sort_unstable();
    let mut form = Vec::with_capacity(keys.len());
    for key in keys {
        let value = &fields[key];
        let encoded = match value {
            Value::String(value) => value.clone(),
            Value::Bool(value) => value.to_string(),
            Value::Number(value) => value.to_string(),
            Value::Null | Value::Array(_) | Value::Object(_) => serde_json::to_string(value)
                .map_err(|_| invalid("a field value could not be serialized"))?,
        };
        form.push((key.clone(), encoded));
    }
    Ok(form)
}

fn validation_form(form: &[(String, String)]) -> Result<Vec<(String, String)>, PublicError> {
    let mut validation = Vec::with_capacity(form.len() + 1);
    validation.extend_from_slice(form);
    validation.push(("execution_options".to_owned(), EXECUTION_OPTIONS.to_owned()));
    validate_form(&validation)?;
    Ok(validation)
}

fn validate_form(form: &[(String, String)]) -> Result<(), PublicError> {
    if form.len() > MAX_FORM_PAIRS {
        return Err(invalid(
            "the final Graph request exceeds 64 form pairs after safety fields",
        ));
    }
    let mut url = Url::parse("https://form.invalid/")
        .map_err(|_| internal_error("Could not initialize the form-size validator"))?;
    {
        let mut serializer = url.query_pairs_mut();
        for (key, value) in form {
            serializer.append_pair(key, value);
        }
    }
    if url.query().map_or(0, str::len) > MAX_SERIALIZED_BYTES {
        return Err(invalid(
            "the final Graph form exceeds the 64 KiB serialized limit",
        ));
    }
    Ok(())
}

fn risk_flags(
    action: MutationActionKind,
    resource_type: MutationResourceType,
    fields: &Map<String, Value>,
) -> Result<(bool, bool), PublicError> {
    if action == MutationActionKind::Update
        && fields
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|status| status.eq_ignore_ascii_case("DELETED"))
    {
        return Ok((true, false));
    }
    if resource_type == MutationResourceType::ReachFrequencyPrediction {
        let releases = fields.contains_key("rf_prediction_id_to_release");
        return match fields
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("quote")
        {
            "reserve" => Ok((releases, true)),
            "cancel" => Ok((true, false)),
            "quote" => Ok((releases, false)),
            _ => Err(invalid("invalid reach/frequency action")),
        };
    }
    Ok((false, false))
}

fn plan_summary(
    action: MutationActionKind,
    resource_type: MutationResourceType,
    target: PlanTarget,
    provider_validated: bool,
    destructive: bool,
    elevated_risk: bool,
    fields: &Map<String, Value>,
) -> MutationPlanSummary {
    let mut field_names = fields.keys().cloned().collect::<Vec<_>>();
    field_names.sort_unstable();
    let omitted_field_name_count = field_names.len().saturating_sub(MAX_PREVIEW_SCALARS);
    field_names.truncate(MAX_PREVIEW_SCALARS);
    let (preview, total_scalars) = scalar_preview(fields);
    MutationPlanSummary {
        reason: String::new(),
        action,
        resource_type,
        target_id: target.object_id,
        target_account_id: target.account_id,
        verified_target_name: None,
        target_identity_verified: false,
        request_count: 1,
        provider_validated,
        destructive,
        elevated_risk,
        removal_acknowledgement_required: destructive,
        field_count: u8::try_from(fields.len()).unwrap_or(u8::MAX),
        field_names,
        omitted_field_name_count: u8::try_from(omitted_field_name_count).unwrap_or(u8::MAX),
        omitted_preview_scalar_count: u16::try_from(
            total_scalars.saturating_sub(MAX_PREVIEW_SCALARS),
        )
        .unwrap_or(u16::MAX),
        preview,
    }
}

fn scalar_preview(fields: &Map<String, Value>) -> (Vec<ScalarPreview>, usize) {
    let mut preview = Vec::with_capacity(MAX_PREVIEW_SCALARS);
    let mut total = 0_usize;
    let mut keys = fields.keys().collect::<Vec<_>>();
    keys.sort_unstable_by(|left, right| preview_key(left).cmp(&preview_key(right)));
    for key in keys {
        collect_preview(&fields[key], key, &mut preview, &mut total);
    }
    (preview, total)
}

fn collect_preview(value: &Value, path: &str, preview: &mut Vec<ScalarPreview>, total: &mut usize) {
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable_by(|left, right| preview_key(left).cmp(&preview_key(right)));
            for key in keys {
                let child = bounded_path(&format!("{path}.{key}"));
                collect_preview(&object[key], &child, preview, total);
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let child = bounded_path(&format!("{path}[{index}]"));
                collect_preview(item, &child, preview, total);
            }
        }
        _ => {
            *total = total.saturating_add(1);
            if preview.len() < MAX_PREVIEW_SCALARS {
                preview.push(ScalarPreview {
                    path: bounded_path(path),
                    value: preview_value(path, value),
                });
            }
        }
    }
}

fn preview_key(key: &str) -> (u8, &str) {
    let lower = key.to_ascii_lowercase();
    let priority = if matches!(
        lower.as_str(),
        "action" | "rf_prediction_id" | "rf_prediction_id_to_release" | "status"
    ) {
        0
    } else if lower.contains("budget")
        || lower.contains("bid")
        || lower.contains("spend")
        || lower.contains("cpm")
    {
        1
    } else if matches!(
        lower.as_str(),
        "end_time" | "expiration_time" | "rule" | "start_time" | "stop_time"
    ) {
        2
    } else if lower.ends_with("_id")
        || matches!(
            lower.as_str(),
            "destination" | "promoted_object" | "target_spec" | "targeting" | "tracking_specs"
        )
    {
        3
    } else {
        4
    };
    (priority, key)
}

fn preview_value(path: &str, value: &Value) -> String {
    if preview_sensitive(path) {
        return "[redacted]".to_owned();
    }
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => preview_string(value),
        Value::Array(_) | Value::Object(_) => unreachable!("preview visits scalar leaves"),
    }
}

fn preview_string(value: &str) -> String {
    if value.chars().any(char::is_control)
        || value.chars().count() > MAX_PREVIEW_STRING_CHARS
        || looks_binary(value)
    {
        return "[redacted]".to_owned();
    }
    if let Ok(mut url) = Url::parse(value)
        && matches!(url.scheme(), "http" | "https")
    {
        url.set_query(None);
        url.set_fragment(None);
        return url.into();
    }
    value.to_owned()
}

fn preview_sensitive(path: &str) -> bool {
    path.split(['.', '['])
        .map(|part| part.trim_end_matches(']'))
        .any(|part| {
            credential_key(part)
                || matches!(
                    part.to_ascii_lowercase().as_str(),
                    "data"
                        | "email"
                        | "emails"
                        | "first_name"
                        | "last_name"
                        | "phone"
                        | "phones"
                        | "rule"
                        | "user_data"
                        | "users"
                )
        })
}

fn looks_binary(value: &str) -> bool {
    value.len() >= 64
        && (value.bytes().all(|byte| byte.is_ascii_hexdigit())
            || value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=' | b'-' | b'_')
            }))
}

fn bounded_path(path: &str) -> String {
    let mut chars = path.chars();
    let mut bounded = chars
        .by_ref()
        .take(MAX_PREVIEW_PATH_CHARS)
        .collect::<String>();
    if chars.next().is_some() {
        bounded.push('…');
    }
    bounded
}

fn endpoint_target(endpoint: &str) -> String {
    endpoint.split('/').next().unwrap_or(endpoint).to_owned()
}

fn validation_confirmed(payload: &Value) -> bool {
    payload.get("success").and_then(Value::as_bool) == Some(true)
        || payload.get("id").and_then(numeric_value).is_some()
}

fn confirmed_result(
    payload: &Value,
    action: MutationActionKind,
    expected_object_id: &str,
) -> Option<Option<String>> {
    if action == MutationActionKind::Create {
        return payload
            .get("id")
            .or_else(|| payload.get("rf_prediction_id"))
            .and_then(numeric_value)
            .map(Some);
    }
    if action == MutationActionKind::Update
        && let Some(returned_id) = payload.get("id")
    {
        return numeric_value(returned_id)
            .filter(|returned_id| returned_id == expected_object_id)
            .map(|_| Some(expected_object_id.to_owned()));
    }
    let success = payload.get("success").and_then(Value::as_bool) == Some(true)
        || payload.as_bool() == Some(true);
    success.then(|| Some(expected_object_id.to_owned()))
}

fn numeric_value(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => numeric_owned(value),
        Value::Number(value) => value.as_u64().map(|number| number.to_string()),
        _ => None,
    }
}

fn ambiguous_graph_error(error: &GraphError) -> bool {
    matches!(
        error,
        GraphError::Transport { .. }
            | GraphError::InvalidJson
            | GraphError::ResponseTooLarge { .. }
            | GraphError::Api {
                retryable: true,
                ..
            }
    )
}

fn purge_expired(plans: &mut HashMap<String, StoredPlan>) {
    let now = Instant::now();
    plans.retain(|_, plan| {
        transition_stale_apply(plan, now);
        plan.status == PlanStatus::Applying || plan.expires_at > now
    });
}

fn evict_oldest_applied(plans: &mut HashMap<String, StoredPlan>) {
    let oldest = plans
        .iter()
        .filter(|(_, plan)| plan.status == PlanStatus::Applied)
        .min_by_key(|(_, plan)| plan.expires_at)
        .map(|(plan_id, _)| plan_id.clone());
    if let Some(plan_id) = oldest {
        plans.remove(&plan_id);
    }
}

fn transition_stale_apply(plan: &mut StoredPlan, now: Instant) -> bool {
    if plan.status == PlanStatus::Applying
        && plan.apply_lease_until.is_none_or(|lease| lease <= now)
    {
        plan.status = PlanStatus::OutcomeUnknown;
        plan.apply_lease_until = None;
        plan.expires_at = now + TERMINAL_TOMBSTONE_TTL;
        true
    } else {
        false
    }
}

fn plan_view(plan_id: &str, stored: &StoredPlan) -> Result<MutationPlanView, PublicError> {
    let expiry = if stored.status == PlanStatus::Applying {
        stored.apply_lease_until.unwrap_or(stored.expires_at)
    } else {
        stored.expires_at
    };
    let expires = expiry.saturating_duration_since(Instant::now());
    let expires_in_seconds = u16::try_from(expires.as_secs().min(PLAN_TTL.as_secs()))
        .map_err(|_| internal_error("The local mutation-plan expiry is invalid"))?;
    Ok(MutationPlanView {
        plan_id: plan_id.to_owned(),
        status: stored.status,
        expires_in_seconds,
        summary: (*stored.plan.summary).clone(),
    })
}

fn validate_apply_acknowledgement(acknowledgement: &str) -> Result<(), PublicError> {
    if acknowledgement == APPLY_ACKNOWLEDGEMENT {
        Ok(())
    } else {
        Err(PublicError::invalid_input(
            "The Meta Ads live-apply acknowledgement is invalid",
            "Obtain explicit operator approval, then pass APPLY_LIVE_META_ADS_CHANGES exactly",
        ))
    }
}

fn validate_plan_id(plan_id: &str) -> Result<(), PublicError> {
    if valid_hex(plan_id, 32) {
        Ok(())
    } else {
        Err(plan_not_found())
    }
}

fn decode_token(value: &str) -> Result<[u8; 32], PublicError> {
    if !valid_hex(value, 64) {
        return Err(plan_confirmation_error());
    }
    let mut token = [0_u8; 32];
    for (destination, pair) in token
        .iter_mut()
        .zip(value.as_bytes().as_chunks::<2>().0.iter())
    {
        let high = hex_nibble(pair[0]).ok_or_else(plan_confirmation_error)?;
        let low = hex_nibble(pair[1]).ok_or_else(plan_confirmation_error)?;
        *destination = (high << 4) | low;
    }
    Ok(token)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn valid_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn random_bytes<const N: usize>() -> Result<[u8; N], PublicError> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes)
        .map_err(|_| internal_error("Secure local plan-token generation failed"))?;
    Ok(bytes)
}

fn random_hex<const N: usize>() -> Result<String, PublicError> {
    Ok(hex_lower(&random_bytes::<N>()?))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn invalid(message: impl Into<String>) -> PublicError {
    PublicError::invalid_input(
        message,
        "Use one supported Graph v26 field set within the published bounds",
    )
}

fn bounded_reason(reason: &str) -> Result<String, PublicError> {
    let reason = reason.trim();
    if reason.is_empty()
        || reason.chars().count() > MAX_REASON_CHARS
        || reason.chars().any(char::is_control)
    {
        return Err(invalid(
            "reason must be non-empty, at most 256 characters, and free of controls",
        ));
    }
    Ok(reason.to_owned())
}

fn unsupported_action(resource_type: MutationResourceType) -> PublicError {
    invalid(format!(
        "{resource_type:?} does not support that action in the v26 mutation-plan registry"
    ))
}

fn plan_not_found() -> PublicError {
    plan_error(
        "The Meta mutation plan was not found or has expired",
        "Reconcile current provider state before deciding whether to build a new plan",
    )
}

fn stale_apply_error() -> PublicError {
    plan_error(
        "The Meta mutation-plan apply lease expired; its outcome is unknown",
        "Read the target from Meta and reconcile it before building or applying another plan",
    )
}

fn plan_confirmation_error() -> PublicError {
    plan_error(
        "The Meta mutation-plan confirmation token is invalid",
        "Pass the exact confirmation_token returned when the plan was built",
    )
}

fn plan_error(message: &str, action: &str) -> PublicError {
    PublicError {
        code: "PLAN_ERROR".to_owned(),
        message: message.to_owned(),
        retryable: false,
        action: Some(action.to_owned()),
    }
}

fn internal_error(message: &str) -> PublicError {
    PublicError {
        code: "INTERNAL_ERROR".to_owned(),
        message: message.to_owned(),
        retryable: false,
        action: Some("Restart the local MCP server if the problem persists".to_owned()),
    }
}

fn ambiguous_result() -> PublicError {
    PublicError {
        code: "AMBIGUOUS_MUTATION_RESULT".to_owned(),
        message: "Meta returned success HTTP status without a provable mutation result".to_owned(),
        retryable: false,
        action: Some(
            "Read the target from Meta and reconcile it before building another plan".to_owned(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use rmcp::schemars::schema_for;
    use serde_json::{Value, json};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    use super::{
        AD_CREATIVE_UPDATE_FIELDS, APPLY_ACKNOWLEDGEMENT, ApplyFinish, ApplyMutationPlanInput,
        BuildMutationPlanInput, CUSTOM_AUDIENCE_CREATE_FIELDS, CUSTOM_AUDIENCE_UPDATE_FIELDS,
        MAX_CALLER_FORM_PAIRS, MAX_FORM_PAIRS, MAX_PLANS, MutationActionInput, MutationPlanStore,
        MutationResourceType, PlanStatus, REACH_FREQUENCY_CREATE_FIELDS, TERMINAL_TOMBSTONE_TTL,
        apply_mutation_plan, build_mutation_plan, prepare_plan,
    };
    use crate::{MetaConfig, error::ToolResponse, graph::GraphClient};

    fn create(resource_type: MutationResourceType, fields: Value) -> BuildMutationPlanInput {
        BuildMutationPlanInput {
            reason: "focused test".to_owned(),
            action: MutationActionInput::Create {
                resource_type,
                ad_account_id: "act_123".to_owned(),
                fields: fields.as_object().unwrap().clone(),
            },
        }
    }

    fn update(resource_type: MutationResourceType, fields: Value) -> BuildMutationPlanInput {
        BuildMutationPlanInput {
            reason: "focused test".to_owned(),
            action: MutationActionInput::Update {
                resource_type,
                ad_account_id: "act_123".to_owned(),
                object_id: "456".to_owned(),
                fields: fields.as_object().unwrap().clone(),
            },
        }
    }

    async fn read_http_request(socket: &mut TcpStream) -> Vec<u8> {
        let mut request = Vec::with_capacity(2 * 1024);
        let mut buffer = [0_u8; 1024];
        loop {
            let bytes_read = socket.read(&mut buffer).await.unwrap();
            if bytes_read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..bytes_read]);
            let Some(headers_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
            else {
                continue;
            };
            let body_start = headers_end + 4;
            let headers = String::from_utf8_lossy(&request[..headers_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or_default();
            if request.len() >= body_start + content_length {
                break;
            }
        }
        request
    }

    fn http_response(status: &str, body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    async fn scripted_graph(
        responses: Vec<Option<Vec<u8>>>,
    ) -> (GraphClient, tokio::task::JoinHandle<(Vec<Vec<u8>>, bool)>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut requests = Vec::with_capacity(responses.len());
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                requests.push(read_http_request(&mut socket).await);
                if let Some(response) = response {
                    socket.write_all(&response).await.unwrap();
                }
            }
            let unexpected_retry =
                tokio::time::timeout(Duration::from_millis(200), listener.accept())
                    .await
                    .is_ok();
            (requests, unexpected_retry)
        });
        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        ))
        .unwrap();
        (graph, server)
    }

    fn receipt(response: ToolResponse<super::MutationPlanReceipt>) -> super::MutationPlanReceipt {
        match response {
            ToolResponse::Success { data } => data,
            ToolResponse::Error { error } => panic!("unexpected plan error: {error:?}"),
        }
    }

    fn apply_input(
        receipt: &super::MutationPlanReceipt,
        acknowledgement: &str,
    ) -> ApplyMutationPlanInput {
        ApplyMutationPlanInput {
            plan_id: receipt.plan.plan_id.clone(),
            confirmation_token: receipt.confirmation_token.clone(),
            apply_acknowledgement: acknowledgement.to_owned(),
            removal_acknowledgement: None,
        }
    }

    #[test]
    fn create_edges_and_validation_modes_are_exact() {
        for (resource, edge, validated) in [
            (MutationResourceType::Campaign, "campaigns", true),
            (MutationResourceType::AdSet, "adsets", true),
            (MutationResourceType::Ad, "ads", true),
            (MutationResourceType::AdCreative, "adcreatives", true),
            (
                MutationResourceType::CustomAudience,
                "customaudiences",
                false,
            ),
            (
                MutationResourceType::ReachFrequencyPrediction,
                "reachfrequencypredictions",
                false,
            ),
        ] {
            let fields = match resource {
                MutationResourceType::CustomAudience => json!({"name": "bounded"}),
                MutationResourceType::ReachFrequencyPrediction => json!({"budget": 100}),
                _ => json!({"name": "bounded"}),
            };
            let plan = prepare_plan(create(resource, fields)).unwrap();
            assert_eq!(plan.request.endpoint.as_ref(), format!("act_123/{edge}"));
            assert_eq!(plan.summary.provider_validated, validated);
            assert_eq!(plan.validation_form.is_some(), validated);
            assert_eq!(plan.summary.request_count, 1);
            assert_eq!(plan.summary.target_account_id, "act_123");
            assert!(!plan.summary.target_identity_verified);
            assert!(plan.summary.verified_target_name.is_none());
        }
    }

    #[test]
    fn delivery_creates_are_forced_paused_and_validation_is_server_owned() {
        for resource in [
            MutationResourceType::Campaign,
            MutationResourceType::AdSet,
            MutationResourceType::Ad,
        ] {
            let plan = prepare_plan(create(resource, json!({"name": "safe"}))).unwrap();
            assert!(
                plan.request
                    .form
                    .iter()
                    .any(|pair| pair == &("status".into(), "PAUSED".into()))
            );
            assert!(
                !plan
                    .request
                    .form
                    .iter()
                    .any(|(key, _)| key == "execution_options")
            );
            assert!(plan.validation_form.unwrap().iter().any(|pair| {
                pair == &("execution_options".into(), "[\"validate_only\"]".into())
            }));
            assert!(prepare_plan(create(resource, json!({"status": "ACTIVE"}))).is_err());
        }
        assert!(
            prepare_plan(create(
                MutationResourceType::Campaign,
                json!({"execution_options": ["validate_only"]}),
            ))
            .is_err()
        );
    }

    #[test]
    fn all_five_delivery_and_audience_resources_support_cud() {
        for resource in [
            MutationResourceType::Campaign,
            MutationResourceType::AdSet,
            MutationResourceType::Ad,
            MutationResourceType::AdCreative,
            MutationResourceType::CustomAudience,
        ] {
            let create_fields = match resource {
                MutationResourceType::CustomAudience => json!({"name": "new"}),
                _ => json!({"name": "new"}),
            };
            assert!(prepare_plan(create(resource, create_fields)).is_ok());
            let update_fields = match resource {
                MutationResourceType::CustomAudience => json!({"retention_days": 30}),
                _ => json!({"name": "updated"}),
            };
            assert!(prepare_plan(update(resource, update_fields)).is_ok());
            assert!(
                prepare_plan(BuildMutationPlanInput {
                    reason: "focused test".to_owned(),
                    action: MutationActionInput::Delete {
                        resource_type: resource,
                        ad_account_id: "act_123".to_owned(),
                        object_id: "456".to_owned(),
                    },
                })
                .is_ok()
            );
        }
    }

    #[test]
    fn custom_audience_lists_cover_rules_retention_sources_and_reject_unproven_fields() {
        for field in [
            "subtype",
            "rule",
            "rule_aggregation",
            "retention_days",
            "event_source_group",
            "event_sources",
            "facebook_page_id",
        ] {
            assert!(CUSTOM_AUDIENCE_CREATE_FIELDS.contains(&field));
        }
        for field in [
            "rule",
            "rule_aggregation",
            "retention_days",
            "event_source_group",
            "event_sources",
        ] {
            assert!(CUSTOM_AUDIENCE_UPDATE_FIELDS.contains(&field));
        }
        for unsupported in ["page_ids", "app_ids", "data_processing_options"] {
            assert!(
                prepare_plan(create(
                    MutationResourceType::CustomAudience,
                    json!({unsupported: "1"}),
                ))
                .is_err()
            );
        }
        assert!(
            prepare_plan(create(
                MutationResourceType::CustomAudience,
                json!({"whats_app_business_phone_number_id": "1"}),
            ))
            .is_err()
        );
    }

    #[test]
    fn ad_creative_update_is_immutable_and_reach_frequency_is_create_only() {
        assert_eq!(
            AD_CREATIVE_UPDATE_FIELDS,
            ["account_id", "adlabels", "name", "status"]
        );
        assert!(
            prepare_plan(update(
                MutationResourceType::AdCreative,
                json!({"object_story_spec": {"page_id": "1"}}),
            ))
            .is_err()
        );
        assert!(REACH_FREQUENCY_CREATE_FIELDS.contains(&"rf_prediction_id"));
        assert!(
            prepare_plan(update(
                MutationResourceType::ReachFrequencyPrediction,
                json!({"action": "quote"}),
            ))
            .is_err()
        );
    }

    #[test]
    fn cancellation_release_and_deleted_status_require_removal_approval() {
        for action in ["reserve", "cancel"] {
            let plan = prepare_plan(create(
                MutationResourceType::ReachFrequencyPrediction,
                json!({"action": action, "rf_prediction_id": "99"}),
            ))
            .unwrap();
            assert_eq!(plan.summary.destructive, action == "cancel");
            assert_eq!(plan.summary.elevated_risk, action == "reserve");
        }
        let deleted = prepare_plan(update(
            MutationResourceType::Campaign,
            json!({"status": "DELETED"}),
        ))
        .unwrap();
        assert!(deleted.summary.removal_acknowledgement_required);
        let release = prepare_plan(create(
            MutationResourceType::ReachFrequencyPrediction,
            json!({"rf_prediction_id_to_release": "99"}),
        ))
        .unwrap();
        assert!(release.summary.removal_acknowledgement_required);
        assert!(
            prepare_plan(create(
                MutationResourceType::ReachFrequencyPrediction,
                json!({"rf_prediction_id_to_release": "not-an-id"}),
            ))
            .is_err()
        );
        assert!(
            prepare_plan(update(MutationResourceType::Campaign, json!({"status": 3}),)).is_err()
        );
    }

    #[test]
    fn nested_credentials_whatsapp_and_bounds_are_rejected() {
        for fields in [
            json!({"targeting": {"access_token": "secret"}}),
            json!({"targeting": {"refresh_token": "secret"}}),
            json!({"targeting": {"provider_secret": "secret"}}),
            json!({"targeting": {"provider_api_key": "secret"}}),
            json!({"creative": {"url": "https://wa.me/123"}}),
            json!({"creative": {"caption": "EAA0000000000000000000000000000000000000"}}),
            json!({"creative": {"caption": "token=secret"}}),
            json!({"creative": {"caption": "api_key=secret"}}),
            json!({"destination": "https://user:synthetic-secret@example.test/path"}),
            json!({"destination": "https://example.test/?%61ccess_token=synthetic-secret"}),
        ] {
            assert!(prepare_plan(update(MutationResourceType::Campaign, fields)).is_err());
        }
        let mut fields = serde_json::Map::new();
        for index in 0..MAX_FORM_PAIRS {
            fields.insert(format!("field_{index}"), json!(index));
        }
        // Provider validation owns one of the 64 final form slots.
        assert!(
            prepare_plan(BuildMutationPlanInput {
                reason: "focused test".to_owned(),
                action: MutationActionInput::Update {
                    resource_type: MutationResourceType::Campaign,
                    ad_account_id: "act_123".into(),
                    object_id: "1".into(),
                    fields,
                },
            })
            .is_err()
        );

        let mut update_fields = serde_json::Map::new();
        for index in 0..MAX_CALLER_FORM_PAIRS {
            update_fields.insert(format!("field_{index}"), json!(index));
        }
        let update = prepare_plan(BuildMutationPlanInput {
            reason: "focused test".to_owned(),
            action: MutationActionInput::Update {
                resource_type: MutationResourceType::Campaign,
                ad_account_id: "act_123".into(),
                object_id: "1".into(),
                fields: update_fields.clone(),
            },
        })
        .unwrap();
        assert_eq!(update.validation_form.unwrap().len(), MAX_FORM_PAIRS);
        assert!(
            prepare_plan(BuildMutationPlanInput {
                reason: "focused test".to_owned(),
                action: MutationActionInput::Create {
                    resource_type: MutationResourceType::Campaign,
                    ad_account_id: "1".into(),
                    fields: update_fields,
                },
            })
            .is_err()
        );
    }

    #[test]
    fn previews_are_bounded_redacted_and_strip_url_secrets() {
        let plan = prepare_plan(update(
            MutationResourceType::Campaign,
            json!({
                "destination": "https://example.test/path?utm_source=value#fragment",
                "rule": {"email": "person@example.test"},
                "targeting": (0..20).collect::<Vec<_>>()
            }),
        ))
        .unwrap();
        assert_eq!(plan.summary.preview.len(), 16);
        assert!(plan.summary.omitted_preview_scalar_count > 0);
        assert!(
            plan.summary
                .preview
                .iter()
                .any(|item| item.value == "https://example.test/path")
        );
        assert!(
            plan.summary
                .preview
                .iter()
                .any(|item| item.value == "[redacted]")
        );

        let priority = prepare_plan(update(
            MutationResourceType::Campaign,
            json!({
                "a_noise": (0..30).collect::<Vec<_>>(),
                "daily_budget": 2500,
                "status": "DELETED"
            }),
        ))
        .unwrap();
        assert_eq!(priority.summary.preview[0].path, "status");
        assert_eq!(priority.summary.preview[1].path, "daily_budget");
    }

    #[test]
    fn store_uses_capability_token_and_claims_before_apply() {
        let store = MutationPlanStore::new();
        let receipt = store
            .insert(
                prepare_plan(create(
                    MutationResourceType::CustomAudience,
                    json!({"name": "bounded"}),
                ))
                .unwrap(),
            )
            .unwrap();
        assert_eq!(receipt.plan.plan_id.len(), 32);
        assert_eq!(receipt.confirmation_token.len(), 64);
        assert_eq!(receipt.apply_acknowledgement, APPLY_ACKNOWLEDGEMENT);
        let input = ApplyMutationPlanInput {
            plan_id: receipt.plan.plan_id.clone(),
            confirmation_token: receipt.confirmation_token,
            apply_acknowledgement: APPLY_ACKNOWLEDGEMENT.to_owned(),
            removal_acknowledgement: None,
        };
        store.begin_apply(&input).unwrap();
        assert_eq!(
            store.get(&input.plan_id).unwrap().status,
            PlanStatus::Applying
        );
        store.finish(&input.plan_id, ApplyFinish::Pending).unwrap();
        assert_eq!(
            store.get(&input.plan_id).unwrap().status,
            PlanStatus::Pending
        );
        assert_eq!(
            store.get(&input.plan_id).unwrap().summary.reason,
            "focused test"
        );
    }

    #[test]
    fn destructive_plan_requires_exact_removal_acknowledgement_before_claim() {
        let store = MutationPlanStore::new();
        let receipt = store
            .insert(
                prepare_plan(BuildMutationPlanInput {
                    reason: "focused delete approval".to_owned(),
                    action: MutationActionInput::Delete {
                        resource_type: MutationResourceType::Campaign,
                        ad_account_id: "act_123".to_owned(),
                        object_id: "456".to_owned(),
                    },
                })
                .unwrap(),
            )
            .unwrap();
        let mut input = apply_input(&receipt, APPLY_ACKNOWLEDGEMENT);

        assert!(store.begin_apply(&input).is_err());
        assert_eq!(
            store.get(&input.plan_id).unwrap().status,
            PlanStatus::Pending
        );

        input.removal_acknowledgement = Some("CONFIRM_META_ADS_REMOVAL".to_owned());
        assert!(store.begin_apply(&input).is_err());
        assert_eq!(
            store.get(&input.plan_id).unwrap().status,
            PlanStatus::Pending
        );

        input.removal_acknowledgement = Some(crate::safety::REMOVAL_ACKNOWLEDGEMENT.to_owned());
        assert!(store.begin_apply(&input).is_ok());
        assert_eq!(
            store.get(&input.plan_id).unwrap().status,
            PlanStatus::Applying
        );
    }

    #[test]
    fn abandoned_apply_claim_becomes_a_retained_outcome_unknown_tombstone() {
        let store = MutationPlanStore::new();
        let receipt = store
            .insert(
                prepare_plan(create(
                    MutationResourceType::CustomAudience,
                    json!({"name": "bounded"}),
                ))
                .unwrap(),
            )
            .unwrap();
        let input = ApplyMutationPlanInput {
            plan_id: receipt.plan.plan_id,
            confirmation_token: receipt.confirmation_token,
            apply_acknowledgement: APPLY_ACKNOWLEDGEMENT.to_owned(),
            removal_acknowledgement: None,
        };
        store.begin_apply(&input).unwrap();
        store
            .state
            .lock()
            .unwrap()
            .get_mut(&input.plan_id)
            .unwrap()
            .apply_lease_until = Some(Instant::now() - Duration::from_secs(1));
        let transitioned_after = Instant::now();
        let tombstone = store.get(&input.plan_id).unwrap();
        assert_eq!(tombstone.status, PlanStatus::OutcomeUnknown);
        assert!(
            store
                .state
                .lock()
                .unwrap()
                .get(&input.plan_id)
                .unwrap()
                .expires_at
                >= transitioned_after + TERMINAL_TOMBSTONE_TTL
        );
        let error = store
            .finish(&input.plan_id, ApplyFinish::Applied)
            .unwrap_err();
        assert!(
            error
                .action
                .as_deref()
                .is_some_and(|action| action.contains("reconcile"))
        );
        assert!(store.discard(&input.plan_id).is_err());
        assert_eq!(
            store.get(&input.plan_id).unwrap().status,
            PlanStatus::OutcomeUnknown
        );
    }

    #[test]
    fn terminal_finishes_refresh_retention_even_near_original_expiry() {
        for (finish, expected_status) in [
            (ApplyFinish::Applied, PlanStatus::Applied),
            (ApplyFinish::OutcomeUnknown, PlanStatus::OutcomeUnknown),
        ] {
            let store = MutationPlanStore::new();
            let receipt = store
                .insert(
                    prepare_plan(create(
                        MutationResourceType::CustomAudience,
                        json!({"name": "bounded"}),
                    ))
                    .unwrap(),
                )
                .unwrap();
            let input = ApplyMutationPlanInput {
                plan_id: receipt.plan.plan_id,
                confirmation_token: receipt.confirmation_token,
                apply_acknowledgement: APPLY_ACKNOWLEDGEMENT.to_owned(),
                removal_acknowledgement: None,
            };
            store.begin_apply(&input).unwrap();
            store
                .state
                .lock()
                .unwrap()
                .get_mut(&input.plan_id)
                .unwrap()
                .expires_at = Instant::now() - Duration::from_secs(1);
            let finished_after = Instant::now();
            store.finish(&input.plan_id, finish).unwrap();

            let tombstone = store.get(&input.plan_id).unwrap();
            assert_eq!(tombstone.status, expected_status);
            assert!(
                store
                    .state
                    .lock()
                    .unwrap()
                    .get(&input.plan_id)
                    .unwrap()
                    .expires_at
                    >= finished_after + TERMINAL_TOMBSTONE_TTL
            );
            assert!(store.discard(&input.plan_id).is_err());
        }
    }

    #[test]
    fn full_store_evicts_only_the_oldest_applied_tombstone() {
        let store = MutationPlanStore::new();
        let plan_ids = fill_plan_store(&store);
        {
            let mut plans = store.state.lock().unwrap();
            let now = Instant::now();
            let oldest = plans.get_mut(&plan_ids[0]).unwrap();
            oldest.status = PlanStatus::Applied;
            oldest.expires_at = now + Duration::from_secs(60);
            let newer = plans.get_mut(&plan_ids[1]).unwrap();
            newer.status = PlanStatus::Applied;
            newer.expires_at = now + Duration::from_secs(120);
        }

        let inserted = store
            .insert(
                prepare_plan(create(
                    MutationResourceType::CustomAudience,
                    json!({"name": "replacement"}),
                ))
                .unwrap(),
            )
            .unwrap();

        let plans = store.state.lock().unwrap();
        assert_eq!(plans.len(), MAX_PLANS);
        assert!(!plans.contains_key(&plan_ids[0]));
        assert!(plans.contains_key(&plan_ids[1]));
        assert!(plans.contains_key(&inserted.plan.plan_id));
        assert!(
            plan_ids[2..]
                .iter()
                .all(|plan_id| plans.contains_key(plan_id))
        );
    }

    #[test]
    fn full_store_never_evicts_pending_applying_or_unknown_plans() {
        let store = MutationPlanStore::new();
        let plan_ids = fill_plan_store(&store);
        {
            let mut plans = store.state.lock().unwrap();
            let now = Instant::now();
            for (index, plan_id) in plan_ids.iter().enumerate() {
                let plan = plans.get_mut(plan_id).unwrap();
                match index % 3 {
                    0 => plan.status = PlanStatus::Pending,
                    1 => {
                        plan.status = PlanStatus::Applying;
                        plan.apply_lease_until = Some(now + Duration::from_secs(60));
                    }
                    _ => {
                        plan.status = PlanStatus::OutcomeUnknown;
                        plan.expires_at = now + TERMINAL_TOMBSTONE_TTL;
                    }
                }
            }
        }

        assert!(
            store
                .insert(
                    prepare_plan(create(
                        MutationResourceType::CustomAudience,
                        json!({"name": "blocked"}),
                    ))
                    .unwrap(),
                )
                .is_err()
        );
        let plans = store.state.lock().unwrap();
        assert_eq!(plans.len(), MAX_PLANS);
        assert!(plan_ids.iter().all(|plan_id| plans.contains_key(plan_id)));
    }

    #[test]
    fn expired_or_missing_plan_requires_provider_reconciliation() {
        let store = MutationPlanStore::new();
        let receipt = store
            .insert(
                prepare_plan(create(
                    MutationResourceType::CustomAudience,
                    json!({"name": "bounded"}),
                ))
                .unwrap(),
            )
            .unwrap();
        store
            .state
            .lock()
            .unwrap()
            .get_mut(&receipt.plan.plan_id)
            .unwrap()
            .expires_at = Instant::now() - Duration::from_secs(1);

        for plan_id in [&receipt.plan.plan_id, "00000000000000000000000000000000"] {
            let error = store.get(plan_id).unwrap_err();
            assert_eq!(error.code, "PLAN_ERROR");
            assert!(
                error
                    .action
                    .as_deref()
                    .is_some_and(|action| action.starts_with("Reconcile current provider state"))
            );
        }
    }

    fn fill_plan_store(store: &MutationPlanStore) -> Vec<String> {
        (0..MAX_PLANS)
            .map(|index| {
                store
                    .insert(
                        prepare_plan(create(
                            MutationResourceType::CustomAudience,
                            json!({"name": format!("plan-{index}")}),
                        ))
                        .unwrap(),
                    )
                    .unwrap()
                    .plan
                    .plan_id
            })
            .collect()
    }

    #[test]
    fn public_build_schema_is_closed_and_action_is_tagged() {
        let schema = serde_json::to_value(schema_for!(BuildMutationPlanInput)).unwrap();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
        let action = schema.pointer("/$defs/MutationActionInput").unwrap();
        assert!(action.to_string().contains("\"type\""));
        assert!(schema.to_string().contains("reach_frequency_prediction"));
        assert!(
            action.to_string().contains("\"maxProperties\":63"),
            "{action}"
        );
        let variants = action["oneOf"].as_array().unwrap();
        for kind in ["update", "delete"] {
            let variant = variants
                .iter()
                .find(|variant| variant.pointer("/properties/type/const") == Some(&json!(kind)))
                .unwrap();
            assert!(
                variant["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("ad_account_id")),
                "{variant}"
            );
        }
    }

    #[tokio::test]
    async fn graph_request_controls_are_rejected_before_any_provider_request() {
        let (graph, server) = scripted_graph(Vec::new()).await;
        let plans = MutationPlanStore::new();
        for resource in [
            MutationResourceType::Campaign,
            MutationResourceType::AdSet,
            MutationResourceType::Ad,
        ] {
            for key in [
                "method",
                "MeThOd",
                "http_method",
                "_method",
                "batch",
                "relative_url",
                "attached_files",
                "depends_on",
                "omit_response_on_success",
                "suppress_http_code",
                "execution.options",
                "http-method",
                "relative.url",
            ] {
                let response =
                    build_mutation_plan(&graph, &plans, update(resource, json!({(key): "delete"})))
                        .await;
                assert!(
                    matches!(response, ToolResponse::Error { ref error } if error.code == "INVALID_INPUT"),
                    "{resource:?} accepted request control {key}",
                );
            }
        }
        assert!(plans.state.lock().unwrap().is_empty());
        let (requests, unexpected_request) = server.await.unwrap();
        assert!(requests.is_empty());
        assert!(!unexpected_request);
    }

    #[tokio::test]
    async fn lifecycle_validates_then_applies_one_frozen_non_validation_request() {
        let (graph, server) = scripted_graph(vec![
            Some(http_response(
                "200 OK",
                r#"{"id":"456","name":"Campaign","account_id":"123","objective":"OUTCOME_SALES"}"#,
            )),
            Some(http_response("200 OK", r#"{"success":true}"#)),
            Some(http_response("200 OK", r#"{"id":"456"}"#)),
        ])
        .await;
        let plans = MutationPlanStore::new();
        let receipt = receipt(
            build_mutation_plan(
                &graph,
                &plans,
                update(MutationResourceType::Campaign, json!({"name": "frozen"})),
            )
            .await,
        );
        assert!(receipt.plan.summary.provider_validated);
        assert!(receipt.plan.summary.target_identity_verified);
        assert_eq!(receipt.plan.summary.target_account_id, "act_123");
        assert_eq!(
            receipt.plan.summary.verified_target_name.as_deref(),
            Some("Campaign")
        );

        assert!(matches!(
            apply_mutation_plan(&graph, &plans, apply_input(&receipt, "yes")).await,
            ToolResponse::Error { .. }
        ));
        let applied =
            apply_mutation_plan(&graph, &plans, apply_input(&receipt, APPLY_ACKNOWLEDGEMENT)).await;
        let ToolResponse::Success { data: applied } = applied else {
            panic!("expected a proven apply result");
        };
        assert_eq!(applied.object_id.as_deref(), Some("456"));
        assert!(matches!(
            apply_mutation_plan(&graph, &plans, apply_input(&receipt, APPLY_ACKNOWLEDGEMENT),)
                .await,
            ToolResponse::Error { .. }
        ));

        let (requests, unexpected_retry) = server.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert!(!unexpected_retry);
        let identity = String::from_utf8_lossy(&requests[0]);
        let validation = String::from_utf8_lossy(&requests[1]);
        let apply = String::from_utf8_lossy(&requests[2]);
        assert!(identity.starts_with("GET /456?"));
        assert!(identity.contains("account_id"));
        assert!(identity.contains("objective"));
        assert!(validation.starts_with("POST /456 HTTP/1.1\r\n"));
        assert!(validation.contains("execution_options=%5B%22validate_only%22%5D"));
        assert!(apply.starts_with("POST /456 HTTP/1.1\r\n"));
        assert!(!apply.contains("execution_options"));
        assert!(apply.ends_with("name=frozen"));
    }

    #[tokio::test]
    async fn build_stores_only_after_provider_validation_proof() {
        let (graph, server) = scripted_graph(vec![
            Some(http_response(
                "200 OK",
                r#"{"id":"456","account_id":"123","objective":"OUTCOME_SALES"}"#,
            )),
            Some(http_response("200 OK", "{}")),
        ])
        .await;
        let plans = MutationPlanStore::new();
        assert!(matches!(
            build_mutation_plan(
                &graph,
                &plans,
                update(MutationResourceType::Campaign, json!({"name": "unproven"})),
            )
            .await,
            ToolResponse::Error { .. }
        ));
        assert!(plans.state.lock().unwrap().is_empty());
        let (requests, unexpected_retry) = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(!unexpected_retry);
    }

    #[tokio::test]
    async fn identity_mismatch_stops_before_validation_or_storage() {
        for payload in [
            r#"{"id":"456","account_id":"123","optimization_goal":"LINK_CLICKS"}"#,
            r#"{"id":"456","account_id":"999","objective":"OUTCOME_SALES"}"#,
        ] {
            let (graph, server) =
                scripted_graph(vec![Some(http_response("200 OK", payload))]).await;
            let plans = MutationPlanStore::new();
            assert!(matches!(
                build_mutation_plan(
                    &graph,
                    &plans,
                    update(MutationResourceType::Campaign, json!({"name": "blocked"})),
                )
                .await,
                ToolResponse::Error { .. }
            ));
            assert!(plans.state.lock().unwrap().is_empty());
            let (requests, unexpected_retry) = server.await.unwrap();
            assert_eq!(requests.len(), 1);
            assert!(String::from_utf8_lossy(&requests[0]).starts_with("GET /456?"));
            assert!(!unexpected_retry);
        }
    }

    #[tokio::test]
    async fn delete_plan_retains_verified_identity_without_writing() {
        let (graph, server) = scripted_graph(vec![Some(http_response(
            "200 OK",
            r#"{"id":"456","name":"Verified campaign","account_id":"123","objective":"OUTCOME_SALES"}"#,
        ))])
        .await;
        let plans = MutationPlanStore::new();
        let receipt = receipt(
            build_mutation_plan(
                &graph,
                &plans,
                BuildMutationPlanInput {
                    reason: "focused delete proof".to_owned(),
                    action: MutationActionInput::Delete {
                        resource_type: MutationResourceType::Campaign,
                        ad_account_id: "123".to_owned(),
                        object_id: "456".to_owned(),
                    },
                },
            )
            .await,
        );
        assert!(receipt.plan.summary.target_identity_verified);
        assert_eq!(receipt.plan.summary.target_account_id, "act_123");
        assert_eq!(
            receipt.plan.summary.verified_target_name.as_deref(),
            Some("Verified campaign")
        );
        assert_eq!(plans.state.lock().unwrap().len(), 1);

        let (requests, unexpected_retry) = server.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(String::from_utf8_lossy(&requests[0]).starts_with("GET /456?"));
        assert!(!unexpected_retry);
    }

    #[tokio::test]
    async fn definite_provider_rejection_restores_pending_without_retry() {
        let (graph, server) = scripted_graph(vec![Some(http_response(
            "400 Bad Request",
            r#"{"error":{"code":100,"message":"invalid field"}}"#,
        ))])
        .await;
        let plans = MutationPlanStore::new();
        let receipt = plans
            .insert(
                prepare_plan(update(
                    MutationResourceType::CustomAudience,
                    json!({"name": "bounded"}),
                ))
                .unwrap(),
            )
            .unwrap();
        assert!(matches!(
            apply_mutation_plan(&graph, &plans, apply_input(&receipt, APPLY_ACKNOWLEDGEMENT),)
                .await,
            ToolResponse::Error { .. }
        ));
        assert_eq!(
            plans.get(&receipt.plan.plan_id).unwrap().status,
            PlanStatus::Pending
        );
        let (requests, unexpected_retry) = server.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(!unexpected_retry);
    }

    #[tokio::test]
    async fn uncertain_apply_outcomes_are_terminal_and_never_retried() {
        for response in [
            None,
            Some(http_response("200 OK", "{")),
            Some(http_response("200 OK", "{}")),
            Some(http_response("200 OK", r#"{"id":"999"}"#)),
        ] {
            let (graph, server) = scripted_graph(vec![response]).await;
            let plans = MutationPlanStore::new();
            let receipt = plans
                .insert(
                    prepare_plan(update(
                        MutationResourceType::CustomAudience,
                        json!({"name": "bounded"}),
                    ))
                    .unwrap(),
                )
                .unwrap();
            assert!(matches!(
                apply_mutation_plan(&graph, &plans, apply_input(&receipt, APPLY_ACKNOWLEDGEMENT),)
                    .await,
                ToolResponse::Error { .. }
            ));
            assert_eq!(
                plans.get(&receipt.plan.plan_id).unwrap().status,
                PlanStatus::OutcomeUnknown
            );
            assert!(matches!(
                apply_mutation_plan(&graph, &plans, apply_input(&receipt, APPLY_ACKNOWLEDGEMENT),)
                    .await,
                ToolResponse::Error { .. }
            ));
            let (requests, unexpected_retry) = server.await.unwrap();
            assert_eq!(requests.len(), 1);
            assert!(!unexpected_retry);
        }
    }
}
