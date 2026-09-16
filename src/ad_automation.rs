// Copyright (C) 2025 ArmaVita LLC
// SPDX-License-Identifier: AGPL-3.0-only

use rmcp::{
    Json,
    handler::server::wrapper::Parameters,
    schemars::{self, JsonSchema},
    tool, tool_router,
};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    graph_tools::{self, GraphData, Params, ReadOptions},
    mutation_plan::APPLY_ACKNOWLEDGEMENT,
    safety::validate_removal_acknowledgement,
    server::MetaAdsServer,
};

// Verified against Meta's Business SDK 26.0.1. SPLIT_TEST_V2 additionally comes from
// the current split-testing guide and Business/ad_studies reference.
const RULE_FIELDS: &[&str] = &[
    "evaluation_spec",
    "execution_spec",
    "name",
    "schedule_spec",
    "status",
];
const STUDY_FIELDS: &[&str] = &[
    "cells",
    "client_business",
    "confidence_level",
    "cooldown_start_time",
    "creative_test_config",
    "description",
    "end_time",
    "name",
    "objectives",
    "observation_end_time",
    "start_time",
    "type",
    "viewers",
];
const CELL_FIELDS: &[&str] = &[
    "adaccounts",
    "ads",
    "adsets",
    "campaigns",
    "creation_template",
    "description",
    "name",
];
const OBJECTIVE_FIELDS: &[&str] = &[
    "adspixels",
    "applications",
    "customconversions",
    "is_primary",
    "name",
    "offline_conversion_data_sets",
    "offsite_datasets",
    "product_catalogs",
    "product_sets",
    "type",
];
const VALUE_CREATE_FIELDS: &[&str] = &["entry_point", "name", "product_type", "rules"];
const VALUE_UPDATE_FIELDS: &[&str] = &["entry_point", "is_default_setting", "name", "rules"];

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AutomationRead<T> {
    pub operation: T,
    #[serde(default)]
    pub options: ReadOptions,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AutomationChange<T> {
    pub operation: T,
    /// Required for every write except preview; these operations may affect live delivery.
    #[schemars(
        length(min = 27, max = 27),
        regex(pattern = "^APPLY_LIVE_META_ADS_CHANGES$")
    )]
    pub apply_acknowledgement: Option<String>,
    /// Required for deletions, including status=DELETED updates.
    #[schemars(
        length(min = 25, max = 25),
        regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
    )]
    pub removal_acknowledgement: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AdRuleRead {
    List {
        ad_account_id: String,
    },
    Read {
        rule_id: String,
    },
    /// Rules governing a campaign, ad set or ad.
    GoverningObject {
        object_id: String,
        pass_evaluation: Option<bool>,
    },
    History {
        rule_id: String,
        /// Meta history action, for example PAUSED or CHANGED_BUDGET.
        action_filter: Option<String>,
        hide_no_changes: Option<bool>,
        object_id: Option<String>,
    },
    AccountHistory {
        ad_account_id: String,
        action_filter: Option<String>,
        evaluation_type: Option<RuleEvaluationType>,
        hide_no_changes: Option<bool>,
        object_id: Option<String>,
    },
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum RuleEvaluationType {
    Schedule,
    Trigger,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AdRuleChange {
    Create {
        ad_account_id: String,
        /// name, evaluation_spec, execution_spec, schedule_spec, status, ui_creation_source. Defaults to DISABLED.
        fields: Map<String, Value>,
    },
    Update {
        rule_id: String,
        /// name, evaluation_spec, execution_spec, schedule_spec, status.
        fields: Map<String, Value>,
    },
    Delete {
        rule_id: String,
    },
    /// Evaluate an existing rule without executing its actions.
    Preview {
        rule_id: String,
    },
    /// Execute the rule now; it may change bids, budgets or delivery.
    Execute {
        rule_id: String,
    },
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AdStudyRead {
    List {
        business_id: String,
    },
    ListAccount {
        ad_account_id: String,
    },
    ImpactingAccount {
        ad_account_id: String,
    },
    /// Studies containing a campaign or ad set.
    ListDeliveryObject {
        object_id: String,
    },
    Read {
        study_id: String,
    },
    Cells {
        study_id: String,
    },
    ReadCell {
        cell_id: String,
    },
    CellEntities {
        cell_id: String,
        entity: StudyCellEntity,
    },
    Objectives {
        study_id: String,
    },
    /// Includes results and last_updated_results by default.
    Results {
        objective_id: String,
        breakdowns: Option<Vec<StudyBreakdown>>,
        /// Optional Meta result date selector.
        ds: Option<String>,
    },
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StudyCellEntity {
    AdAccounts,
    AdSets,
    Campaigns,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StudyBreakdown {
    Age,
    CellId,
    Country,
    Gender,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AdStudyChange {
    Create {
        business_id: String,
        /// Required: name, cells, start_time, end_time. Optional v26 study fields include type, objectives, confidence_level, creative_test_config.
        fields: Map<String, Value>,
    },
    Update {
        study_id: String,
        fields: Map<String, Value>,
    },
    UpdateCell {
        cell_id: String,
        /// name, description, adaccounts, ads, adsets, campaigns, creation_template.
        fields: Map<String, Value>,
    },
    UpdateObjective {
        objective_id: String,
        /// name, type, is_primary, adspixels, applications, customconversions, offsite_datasets, offline_conversion_data_sets, product_catalogs, product_sets.
        fields: Map<String, Value>,
    },
    /// Delete the study using Meta's DELETE endpoint; does not delete its ads.
    Delete { study_id: String },
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ValueRuleRead {
    List {
        ad_account_id: String,
        product_type: Option<ValueRuleProduct>,
        status: Option<ValueRuleStatus>,
    },
    Read {
        rule_set_id: String,
    },
    Rules {
        rule_set_id: String,
    },
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum ValueRuleProduct {
    Audience,
    LeadgenAds,
    OmniChannel,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum ValueRuleStatus {
    Active,
    Deleted,
    Draft,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ValueRuleChange {
    Create {
        ad_account_id: String,
        /// name, rules (array), product_type, entry_point.
        fields: Map<String, Value>,
    },
    Update {
        rule_set_id: String,
        /// name, rules (array), entry_point, is_default_setting.
        fields: Map<String, Value>,
    },
    Delete {
        rule_set_id: String,
    },
}

#[derive(Debug, PartialEq)]
enum Method {
    Get,
    Post,
    Delete,
}

#[derive(Debug)]
struct Request {
    method: Method,
    endpoint: String,
    params: Params,
    proof: Option<(&'static str, &'static [&'static str])>,
}

impl Request {
    fn get(endpoint: String, options: &ReadOptions, defaults: &str) -> Result<Self, PublicError> {
        Ok(Self {
            method: Method::Get,
            endpoint,
            params: graph_tools::read_params(options, defaults)?,
            proof: None,
        })
    }

    fn post(endpoint: String, params: Params) -> Self {
        Self {
            method: Method::Post,
            endpoint,
            params,
            proof: None,
        }
    }

    fn delete(endpoint: String) -> Self {
        Self {
            method: Method::Delete,
            endpoint,
            params: Vec::new(),
            proof: None,
        }
    }

    fn verify(mut self, fields: &'static str, required: &'static [&'static str]) -> Self {
        self.proof = Some((fields, required));
        self
    }

    async fn send(self, graph: &GraphClient) -> ToolResponse<GraphData> {
        if let Some((fields, required)) = self.proof {
            let object_id = self.endpoint.split('/').next().expect("validated endpoint");
            if let Err(error) =
                crate::node_identity::verify_object(graph, object_id, fields, required).await
            {
                return ToolResponse::error(error);
            }
        }
        match self.method {
            Method::Get => graph_tools::read(graph, &self.endpoint, self.params).await,
            Method::Post => graph_tools::write(graph, &self.endpoint, self.params).await,
            Method::Delete => graph_tools::delete(graph, &self.endpoint, self.params).await,
        }
    }
}

fn rule_read(input: AutomationRead<AdRuleRead>) -> Result<Request, PublicError> {
    if let AdRuleRead::GoverningObject {
        object_id,
        pass_evaluation,
    } = &input.operation
    {
        let mut request = Request::get(
            format!(
                "{}/adrules_governed",
                graph_tools::id(object_id, "object_id")?
            ),
            &input.options,
            "id,name,status",
        )?;
        if let Some(pass) = pass_evaluation {
            request
                .params
                .push(("pass_evaluation".into(), pass.to_string()));
        }
        return Ok(request);
    }
    let (endpoint, defaults, filters) = match input.operation {
        AdRuleRead::List { ad_account_id } => (
            format!("{}/adrules_library", graph_tools::account(&ad_account_id)?),
            "id,name,status,updated_time",
            None,
        ),
        AdRuleRead::Read { rule_id } => (
            graph_tools::id(&rule_id, "rule_id")?,
            "id,name,status,evaluation_spec,execution_spec,schedule_spec",
            None,
        ),
        AdRuleRead::GoverningObject { .. } => unreachable!("handled above"),
        AdRuleRead::History {
            rule_id,
            action_filter,
            hide_no_changes,
            object_id,
        } => (
            format!("{}/history", graph_tools::id(&rule_id, "rule_id")?),
            "timestamp,is_manual,exception_code,results",
            Some((action_filter, hide_no_changes, object_id, None)),
        ),
        AdRuleRead::AccountHistory {
            ad_account_id,
            action_filter,
            evaluation_type,
            hide_no_changes,
            object_id,
        } => (
            format!("{}/adrules_history", graph_tools::account(&ad_account_id)?),
            "rule_id,timestamp,is_manual,exception_code,results",
            Some((action_filter, hide_no_changes, object_id, evaluation_type)),
        ),
    };
    let mut request = Request::get(endpoint, &input.options, defaults)?;
    if let Some((action, hide, object, evaluation_type)) = filters {
        if let Some(action) = action {
            let action = graph_tools::text(&action, "action_filter", 80)?;
            if !action.bytes().all(|b| b.is_ascii_uppercase() || b == b'_') {
                return Err(invalid("action_filter must be a Meta history action"));
            }
            request.params.push(("action".into(), action));
        }
        if let Some(hide) = hide {
            request
                .params
                .push(("hide_no_changes".into(), hide.to_string()));
        }
        if let Some(object) = object {
            request
                .params
                .push(("object_id".into(), graph_tools::id(&object, "object_id")?));
        }
        if let Some(evaluation_type) = evaluation_type {
            request.params.push((
                "evaluation_type".into(),
                match evaluation_type {
                    RuleEvaluationType::Schedule => "SCHEDULE",
                    RuleEvaluationType::Trigger => "TRIGGER",
                }
                .into(),
            ));
        }
    }
    Ok(request)
}

fn rule_change(input: AutomationChange<AdRuleChange>) -> Result<Request, PublicError> {
    let preview = matches!(input.operation, AdRuleChange::Preview { .. });
    acknowledge(!preview, input.apply_acknowledgement.as_deref())?;
    let removed = matches!(&input.operation, AdRuleChange::Delete { .. })
        || matches!(&input.operation, AdRuleChange::Create { fields, .. } | AdRuleChange::Update { fields, .. } if fields.get("status").and_then(Value::as_str) == Some("DELETED"));
    validate_removal_acknowledgement(removed, input.removal_acknowledgement.as_deref())?;
    match input.operation {
        AdRuleChange::Create {
            ad_account_id,
            mut fields,
        } => {
            required(&fields, &["name", "evaluation_spec", "execution_spec"])?;
            fields
                .entry("status")
                .or_insert_with(|| Value::String("DISABLED".into()));
            validate_rule(&fields)?;
            let mut allowed = RULE_FIELDS.to_vec();
            allowed.push("ui_creation_source");
            Ok(Request::post(
                format!("{}/adrules_library", graph_tools::account(&ad_account_id)?),
                write_fields(&fields, &allowed)?,
            ))
        }
        AdRuleChange::Update { rule_id, fields } => {
            validate_rule(&fields)?;
            Ok(Request::post(
                graph_tools::id(&rule_id, "rule_id")?,
                write_fields(&fields, RULE_FIELDS)?,
            )
            .verify(
                "evaluation_spec,execution_spec",
                &["evaluation_spec", "execution_spec"],
            ))
        }
        AdRuleChange::Delete { rule_id } => Ok(Request::delete(graph_tools::id(
            &rule_id, "rule_id",
        )?)
        .verify(
            "evaluation_spec,execution_spec",
            &["evaluation_spec", "execution_spec"],
        )),
        AdRuleChange::Preview { rule_id } => Ok(Request::post(
            format!("{}/preview", graph_tools::id(&rule_id, "rule_id")?),
            Vec::new(),
        )
        .verify(
            "evaluation_spec,execution_spec",
            &["evaluation_spec", "execution_spec"],
        )),
        AdRuleChange::Execute { rule_id } => Ok(Request::post(
            format!("{}/execute", graph_tools::id(&rule_id, "rule_id")?),
            Vec::new(),
        )
        .verify(
            "evaluation_spec,execution_spec",
            &["evaluation_spec", "execution_spec"],
        )),
    }
}

fn study_read(input: AutomationRead<AdStudyRead>) -> Result<Request, PublicError> {
    const STUDY: &str = "id,name,type,start_time,end_time,canceled_time";
    const CELL: &str = "id,name,treatment_percentage,control_percentage,ad_entities_count,ad_ids";
    let (endpoint, defaults, filters) = match input.operation {
        AdStudyRead::ListAccount { ad_account_id } => (
            format!("{}/ad_studies", graph_tools::account(&ad_account_id)?),
            STUDY,
            None,
        ),
        AdStudyRead::ImpactingAccount { ad_account_id } => (
            format!(
                "{}/impacting_ad_studies",
                graph_tools::account(&ad_account_id)?
            ),
            STUDY,
            None,
        ),
        AdStudyRead::ListDeliveryObject { object_id } => (
            format!("{}/ad_studies", graph_tools::id(&object_id, "object_id")?),
            STUDY,
            None,
        ),
        AdStudyRead::List { business_id } => (
            format!(
                "{}/ad_studies",
                graph_tools::id(&business_id, "business_id")?
            ),
            STUDY,
            None,
        ),
        AdStudyRead::Read { study_id } => (graph_tools::id(&study_id, "study_id")?, STUDY, None),
        AdStudyRead::Cells { study_id } => (
            format!("{}/cells", graph_tools::id(&study_id, "study_id")?),
            CELL,
            None,
        ),
        AdStudyRead::ReadCell { cell_id } => (graph_tools::id(&cell_id, "cell_id")?, CELL, None),
        AdStudyRead::CellEntities { cell_id, entity } => {
            let edge = match entity {
                StudyCellEntity::AdAccounts => "adaccounts",
                StudyCellEntity::AdSets => "adsets",
                StudyCellEntity::Campaigns => "campaigns",
            };
            (
                format!("{}/{edge}", graph_tools::id(&cell_id, "cell_id")?),
                "id,name",
                None,
            )
        }
        AdStudyRead::Objectives { study_id } => (
            format!("{}/objectives", graph_tools::id(&study_id, "study_id")?),
            "id,name,type,is_primary,last_updated_results",
            None,
        ),
        AdStudyRead::Results {
            objective_id,
            breakdowns,
            ds,
        } => (
            graph_tools::id(&objective_id, "objective_id")?,
            "id,name,type,results,last_updated_results",
            Some((breakdowns, ds)),
        ),
    };
    let mut request = Request::get(endpoint, &input.options, defaults)?;
    if let Some((breakdowns, ds)) = filters {
        if let Some(breakdowns) = breakdowns {
            if breakdowns.is_empty() || breakdowns.len() > 4 {
                return Err(invalid("breakdowns must contain 1–4 values"));
            }
            let values: Vec<_> = breakdowns
                .into_iter()
                .map(|b| match b {
                    StudyBreakdown::Age => "age",
                    StudyBreakdown::CellId => "cell_id",
                    StudyBreakdown::Country => "country",
                    StudyBreakdown::Gender => "gender",
                })
                .collect();
            request.params.push((
                "breakdowns".into(),
                graph_tools::json(&serde_json::json!(values), "breakdowns")?,
            ));
        }
        if let Some(ds) = ds {
            request
                .params
                .push(("ds".into(), graph_tools::text(&ds, "ds", 64)?));
        }
    }
    Ok(request)
}

fn study_change(input: AutomationChange<AdStudyChange>) -> Result<Request, PublicError> {
    acknowledge(true, input.apply_acknowledgement.as_deref())?;
    validate_removal_acknowledgement(
        matches!(input.operation, AdStudyChange::Delete { .. }),
        input.removal_acknowledgement.as_deref(),
    )?;
    match input.operation {
        AdStudyChange::Create {
            business_id,
            mut fields,
        } => {
            required(&fields, &["name", "cells", "start_time", "end_time"])?;
            fields
                .entry("type")
                .or_insert_with(|| Value::String("SPLIT_TEST".into()));
            validate_study(&fields, true)?;
            Ok(Request::post(
                format!(
                    "{}/ad_studies",
                    graph_tools::id(&business_id, "business_id")?
                ),
                write_fields(&fields, STUDY_FIELDS)?,
            ))
        }
        AdStudyChange::Update { study_id, fields } => {
            validate_study(&fields, false)?;
            Ok(Request::post(
                graph_tools::id(&study_id, "study_id")?,
                write_fields(&fields, STUDY_FIELDS)?,
            )
            .verify("cells.limit(1)", &["cells"]))
        }
        AdStudyChange::UpdateCell { cell_id, fields } => Ok(Request::post(
            graph_tools::id(&cell_id, "cell_id")?,
            write_fields(&fields, CELL_FIELDS)?,
        )
        .verify("ad_entities_count", &["ad_entities_count"])),
        AdStudyChange::UpdateObjective {
            objective_id,
            fields,
        } => Ok(Request::post(
            graph_tools::id(&objective_id, "objective_id")?,
            write_fields(&fields, OBJECTIVE_FIELDS)?,
        )
        .verify("is_primary,type", &["is_primary", "type"])),
        AdStudyChange::Delete { study_id } => {
            Ok(Request::delete(graph_tools::id(&study_id, "study_id")?)
                .verify("cells.limit(1)", &["cells"]))
        }
    }
}

fn value_read(input: AutomationRead<ValueRuleRead>) -> Result<Request, PublicError> {
    let (endpoint, filters) = match input.operation {
        ValueRuleRead::List {
            ad_account_id,
            product_type,
            status,
        } => (
            format!("{}/value_rule_set", graph_tools::account(&ad_account_id)?),
            Some((product_type, status)),
        ),
        ValueRuleRead::Read { rule_set_id } => {
            (graph_tools::id(&rule_set_id, "rule_set_id")?, None)
        }
        ValueRuleRead::Rules { rule_set_id } => {
            // Meta types this edge as objects rather than publishing a stable field list.
            return Request::get(
                format!("{}/rules", graph_tools::id(&rule_set_id, "rule_set_id")?),
                &input.options,
                "",
            );
        }
    };
    let mut request = Request::get(
        endpoint,
        &input.options,
        "id,name,status,product_type,is_default_setting",
    )?;
    if let Some((product, status)) = filters {
        if let Some(product) = product {
            request.params.push((
                "product_type".into(),
                match product {
                    ValueRuleProduct::Audience => "AUDIENCE",
                    ValueRuleProduct::LeadgenAds => "LEADGEN_ADS",
                    ValueRuleProduct::OmniChannel => "OMNI_CHANNEL",
                }
                .into(),
            ));
        }
        if let Some(status) = status {
            request.params.push((
                "status".into(),
                match status {
                    ValueRuleStatus::Active => "ACTIVE",
                    ValueRuleStatus::Deleted => "DELETED",
                    ValueRuleStatus::Draft => "DRAFT",
                }
                .into(),
            ));
        }
    }
    Ok(request)
}

fn value_change(input: AutomationChange<ValueRuleChange>) -> Result<Request, PublicError> {
    acknowledge(true, input.apply_acknowledgement.as_deref())?;
    validate_removal_acknowledgement(
        matches!(input.operation, ValueRuleChange::Delete { .. }),
        input.removal_acknowledgement.as_deref(),
    )?;
    match input.operation {
        ValueRuleChange::Create {
            ad_account_id,
            fields,
        } => {
            required(&fields, &["name", "rules"])?;
            validate_value_rule(&fields)?;
            Ok(Request::post(
                format!("{}/value_rule_set", graph_tools::account(&ad_account_id)?),
                write_fields(&fields, VALUE_CREATE_FIELDS)?,
            ))
        }
        ValueRuleChange::Update {
            rule_set_id,
            fields,
        } => {
            validate_value_rule(&fields)?;
            Ok(Request::post(
                graph_tools::id(&rule_set_id, "rule_set_id")?,
                write_fields(&fields, VALUE_UPDATE_FIELDS)?,
            )
            .verify(
                "product_type,is_default_setting",
                &["product_type", "is_default_setting"],
            ))
        }
        ValueRuleChange::Delete { rule_set_id } => Ok(Request::post(
            format!(
                "{}/delete_rule_set",
                graph_tools::id(&rule_set_id, "rule_set_id")?
            ),
            vec![("status".into(), "DELETED".into())],
        )
        .verify(
            "product_type,is_default_setting",
            &["product_type", "is_default_setting"],
        )),
    }
}

pub(crate) fn acknowledge(required: bool, value: Option<&str>) -> Result<(), PublicError> {
    if value.is_some_and(|v| v != APPLY_ACKNOWLEDGEMENT) || (required && value.is_none()) {
        return Err(invalid(
            "Explicit approval requires apply_acknowledgement=APPLY_LIVE_META_ADS_CHANGES",
        ));
    }
    Ok(())
}

fn write_fields(fields: &Map<String, Value>, allowed: &[&str]) -> Result<Params, PublicError> {
    if fields.is_empty() {
        return Err(invalid("fields must include at least one change"));
    }
    graph_tools::form_fields(fields, allowed)
}

fn required(fields: &Map<String, Value>, keys: &[&str]) -> Result<(), PublicError> {
    if keys.iter().any(|key| !fields.contains_key(*key)) {
        return Err(invalid("Required fields are missing for this operation"));
    }
    Ok(())
}

fn optional_enum(
    fields: &Map<String, Value>,
    key: &str,
    allowed: &[&str],
) -> Result<(), PublicError> {
    if fields
        .get(key)
        .is_some_and(|value| !value.as_str().is_some_and(|v| allowed.contains(&v)))
    {
        return Err(invalid(format!("{key} is not a supported Meta value")));
    }
    Ok(())
}

fn validate_name(fields: &Map<String, Value>) -> Result<(), PublicError> {
    if let Some(name) = fields.get("name") {
        graph_tools::text(
            name.as_str()
                .ok_or_else(|| invalid("name must be a string"))?,
            "name",
            256,
        )?;
    }
    Ok(())
}

fn validate_rule(fields: &Map<String, Value>) -> Result<(), PublicError> {
    validate_name(fields)?;
    optional_enum(
        fields,
        "status",
        &["ENABLED", "DISABLED", "DELETED", "HAS_ISSUES"],
    )?;
    for key in ["evaluation_spec", "execution_spec", "schedule_spec"] {
        if fields
            .get(key)
            .is_some_and(|value| !value.as_object().is_some_and(|v| !v.is_empty()))
        {
            return Err(invalid(format!("{key} must be a nonempty object")));
        }
    }
    Ok(())
}

fn validate_study(fields: &Map<String, Value>, create: bool) -> Result<(), PublicError> {
    validate_name(fields)?;
    optional_enum(
        fields,
        "type",
        &[
            "SPLIT_TEST",
            "SPLIT_TEST_V2",
            "LIFT",
            "CONTINUOUS_LIFT_CONFIG",
            "GEO_LIFT",
            "BACKEND_AB_TESTING",
            "CREATIVE_SPEND_ENFORCEMENT",
            "PORTFOLIO_OPTIMIZER",
            "VERSION_CONTROL",
        ],
    )?;
    for key in [
        "start_time",
        "end_time",
        "cooldown_start_time",
        "observation_end_time",
    ] {
        if fields
            .get(key)
            .is_some_and(|value| !value.as_u64().is_some_and(|v| v > 0 && v <= 4_102_444_800))
        {
            return Err(invalid(format!(
                "{key} must be a positive Unix timestamp before 2100"
            )));
        }
    }
    if let (Some(start), Some(end)) = (
        fields.get("start_time").and_then(Value::as_u64),
        fields.get("end_time").and_then(Value::as_u64),
    ) && end <= start
    {
        return Err(invalid("end_time must be after start_time"));
    }
    if let Some(cells) = fields.get("cells") {
        let cells = cells
            .as_array()
            .filter(|v| !v.is_empty() && v.len() <= 150)
            .ok_or_else(|| invalid("cells must contain 1–150 objects"))?;
        if cells.iter().any(|v| !v.is_object()) {
            return Err(invalid("Each study cell must be an object"));
        }
    }
    if let Some(config) = fields.get("creative_test_config") {
        let config = config
            .as_object()
            .ok_or_else(|| invalid("creative_test_config must be an object"))?;
        if config.contains_key("daily_budget") == config.contains_key("lifetime_budget_percentage")
        {
            return Err(invalid(
                "Choose daily_budget or lifetime_budget_percentage for the creative test",
            ));
        }
        if fields.get("type").and_then(Value::as_str) != Some("SPLIT_TEST_V2") {
            return Err(invalid("creative_test_config requires type=SPLIT_TEST_V2"));
        }
        for (key, value) in config {
            let valid = match key.as_str() {
                "daily_budget" => value
                    .as_u64()
                    .is_some_and(|v| v > 0 && v <= i64::MAX as u64),
                "lifetime_budget_percentage" => {
                    value.as_f64().is_some_and(|v| v > 0.0 && v <= 100.0)
                }
                _ => false,
            };
            if !valid {
                return Err(invalid(
                    "Creative-test budget must be positive; a percentage cannot exceed 100",
                ));
            }
        }
    }
    if create && fields.get("type").and_then(Value::as_str) == Some("SPLIT_TEST_V2") {
        required(
            fields,
            &[
                "creative_test_config",
                "cooldown_start_time",
                "observation_end_time",
            ],
        )?;
        let cells = fields
            .get("cells")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("Creative tests require cells"))?;
        if !(2..=5).contains(&cells.len())
            || fields.get("cooldown_start_time") != fields.get("start_time")
            || fields.get("observation_end_time") != fields.get("end_time")
        {
            return Err(invalid(
                "Creative tests need 2–5 cells with matching cooldown/start and observation/end times",
            ));
        }
        let mut total = 0_u64;
        for cell in cells {
            let ads = cell
                .get("ads")
                .and_then(Value::as_array)
                .filter(|ads| ads.len() == 1)
                .ok_or_else(|| invalid("Each creative-test cell requires exactly one ad"))?;
            let ad = ads[0]
                .as_str()
                .ok_or_else(|| invalid("Creative-test ad IDs must be numeric strings"))?;
            graph_tools::id(ad, "ads")?;
            let share = cell
                .get("treatment_percentage")
                .and_then(Value::as_u64)
                .filter(|n| *n > 0 && *n <= 100)
                .ok_or_else(|| {
                    invalid("Creative-test treatment percentages must be positive integers")
                })?;
            total += share;
        }
        if total != 100 {
            return Err(invalid("Creative-test cell percentages must total 100"));
        }
    }
    Ok(())
}

fn validate_value_rule(fields: &Map<String, Value>) -> Result<(), PublicError> {
    validate_name(fields)?;
    optional_enum(
        fields,
        "product_type",
        &["AUDIENCE", "LEADGEN_ADS", "OMNI_CHANNEL"],
    )?;
    optional_enum(
        fields,
        "entry_point",
        &[
            "ADVERTISING_SETTINGS",
            "L2_AUDIENCE",
            "L2_CONVERSION_LOCATION",
            "L2_GLOBAL",
            "L2_NCA_GOAL",
            "L2_PLACEMENT",
        ],
    )?;
    if fields.get("rules").is_some_and(|value| {
        !value.as_array().is_some_and(|rules| {
            !rules.is_empty() && rules.len() <= 256 && rules.iter().all(Value::is_object)
        })
    }) {
        return Err(invalid("rules must be a nonempty bounded array of objects"));
    }
    if fields
        .get("is_default_setting")
        .is_some_and(|v| !v.is_boolean())
    {
        return Err(invalid("is_default_setting must be a boolean"));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> PublicError {
    PublicError::invalid_input(message, "Use the documented operation and Meta v26 fields")
}

async fn dispatch(
    request: Result<Request, PublicError>,
    graph: &GraphClient,
) -> ToolResponse<GraphData> {
    match request {
        Ok(request) => request.send(graph).await,
        Err(error) => ToolResponse::error(error),
    }
}

#[tool_router(router = automation_router, vis = "pub(crate)")]
impl MetaAdsServer {
    #[tool(
        name = "read_ad_rules",
        description = "List or inspect Meta automated rules and execution history.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn read_ad_rules(
        &self,
        Parameters(input): Parameters<AutomationRead<AdRuleRead>>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        dispatch(rule_read(input), &self.graph)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "manage_ad_rules",
        description = "Create disabled rules, update, preview, execute or delete. Live writes need explicit approval; preview only evaluates.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn manage_ad_rules(
        &self,
        Parameters(input): Parameters<AutomationChange<AdRuleChange>>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        dispatch(rule_change(input), &self.graph)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "read_ad_studies",
        description = "List Business A/B studies, inspect cells and objectives, or retrieve objective results.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn read_ad_studies(
        &self,
        Parameters(input): Parameters<AutomationRead<AdStudyRead>>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        dispatch(study_read(input), &self.graph)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "manage_ad_studies",
        description = "Create or edit Business studies, cells and objectives, or delete a study. Study eligibility remains subject to Meta.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn manage_ad_studies(
        &self,
        Parameters(input): Parameters<AutomationChange<AdStudyChange>>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        dispatch(study_change(input), &self.graph)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "read_value_rules",
        description = "List Meta value-rule sets or inspect a set and its rules.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn read_value_rules(
        &self,
        Parameters(input): Parameters<AutomationRead<ValueRuleRead>>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        dispatch(value_read(input), &self.graph)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "manage_value_rules",
        description = "Create, update or delete value-rule sets. Attach existing sets through an ad-set mutation plan using value_rule_set_id.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn manage_value_rules(
        &self,
        Parameters(input): Parameters<AutomationChange<ValueRuleChange>>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        dispatch(value_change(input), &self.graph)
            .await
            .into_mcp_result()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn change<T: serde::de::DeserializeOwned>(operation: Value) -> AutomationChange<T> {
        serde_json::from_value(
            json!({"operation": operation, "apply_acknowledgement": APPLY_ACKNOWLEDGEMENT}),
        )
        .unwrap()
    }

    #[test]
    fn rule_request_is_disabled_by_default_and_preserves_nested_execution_options() {
        let input = change(
            json!({"action":"create","ad_account_id":"42","fields":{"name":"Budget guard","evaluation_spec":{"evaluation_type":"SCHEDULE","filters":[]},"execution_spec":{"execution_type":"CHANGE_BUDGET","execution_options":[{"field":"change_spec","value":{"amount":10}}]}}}),
        );
        let request = rule_change(input).unwrap();
        assert_eq!(request.endpoint, "act_42/adrules_library");
        assert!(
            request
                .params
                .contains(&("status".into(), "DISABLED".into()))
        );
        assert!(
            request
                .params
                .iter()
                .any(|(key, value)| key == "execution_spec" && value.contains("execution_options"))
        );
        for fields in [
            json!({"name":"x","execution_options":["validate_only"]}),
            json!({"execution_spec":{"access_token":"secret"}}),
            json!({"name":"x","method":"DELETE"}),
        ] {
            assert!(
                rule_change(change(
                    json!({"action":"update","rule_id":"1","fields":fields})
                ))
                .is_err()
            );
        }
    }

    #[test]
    fn live_and_removal_guards_cover_alternate_delete_status() {
        let mut execute = change(json!({"action":"execute","rule_id":"1"}));
        execute.apply_acknowledgement = None;
        assert!(rule_change(execute).is_err());
        let mut preview = change(json!({"action":"preview","rule_id":"1"}));
        preview.apply_acknowledgement = None;
        assert_eq!(rule_change(preview).unwrap().endpoint, "1/preview");
        assert!(
            rule_change(change(
                json!({"action":"update","rule_id":"1","fields":{"status":"DELETED"}})
            ))
            .is_err()
        );
        let mut delete = change(json!({"action":"delete","rule_set_id":"1"}));
        assert!(
            value_change(change::<ValueRuleChange>(
                json!({"action":"delete","rule_set_id":"1"})
            ))
            .is_err()
        );
        delete.removal_acknowledgement = Some(crate::safety::REMOVAL_ACKNOWLEDGEMENT.into());
        let request = value_change(delete).unwrap();
        assert_eq!(request.method, Method::Post);
        assert_eq!(request.endpoint, "1/delete_rule_set");
        assert_eq!(request.params, [("status".into(), "DELETED".into())]);
    }

    #[test]
    fn studies_use_business_parent_and_objective_results() {
        let fields = json!({"name":"A/B","start_time":1_800_000_000,"end_time":1_800_086_400,"cells":[{"name":"A","treatment_percentage":50,"adsets":["1"]},{"name":"B","treatment_percentage":50,"adsets":["2"]}]});
        let request = study_change(change(
            json!({"action":"create","business_id":"42","fields":fields}),
        ))
        .unwrap();
        assert_eq!(request.endpoint, "42/ad_studies");
        assert!(
            study_change(change(
                json!({"action":"create","business_id":"act_42","fields":fields})
            ))
            .is_err()
        );
        let results = study_read(serde_json::from_value(json!({"operation":{"action":"results","objective_id":"99","breakdowns":["cell_id"]}})).unwrap()).unwrap();
        assert_eq!(results.endpoint, "99");
        assert!(
            results
                .params
                .iter()
                .any(|(key, value)| key == "fields" && value.contains("results"))
        );
        assert!(
            results
                .params
                .contains(&("breakdowns".into(), "[\"cell_id\"]".into()))
        );
        assert!(
            study_change(change(
                json!({"action":"update","study_id":"1","fields":{"start_time":4,"end_time":2}})
            ))
            .is_err()
        );
    }

    #[tokio::test]
    async fn node_proof_prevents_deleting_a_different_graph_resource() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let graph = GraphClient::new(&crate::MetaConfig::for_test(
            format!("http://{}", listener.local_addr().unwrap()),
            Some("test-token"),
        ))
        .unwrap();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for reply in [
                json!({"id":"1","name":"A campaign"}),
                json!({"id":"1","evaluation_spec":{},"execution_spec":{}}),
                json!({"success":true}),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let mut chunk = [0; 1024];
                    let count = socket.read(&mut chunk).await.unwrap();
                    assert!(count > 0 && request.len() < 8192);
                    request.extend_from_slice(&chunk[..count]);
                }
                requests.push(String::from_utf8(request).unwrap());
                let body = reply.to_string();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
            requests
        });
        for should_succeed in [false, true] {
            let mut input = change(json!({"action":"delete","rule_id":"1"}));
            input.removal_acknowledgement = Some(crate::safety::REMOVAL_ACKNOWLEDGEMENT.into());
            let response = rule_change(input).unwrap().send(&graph).await;
            assert_eq!(
                matches!(response, ToolResponse::Success { .. }),
                should_succeed
            );
        }
        let requests = server.await.unwrap();
        assert!(requests[0].starts_with("GET /1?"));
        assert!(requests[1].starts_with("GET /1?"));
        assert!(requests[2].starts_with("DELETE /1 "));
    }

    #[test]
    fn creative_studies_validate_cell_allocation_before_any_request() {
        let mut fields = json!({"name":"Creative test","type":"SPLIT_TEST_V2","start_time":1_800_000_000,"cooldown_start_time":1_800_000_000,"end_time":1_800_086_400,"observation_end_time":1_800_086_400,"creative_test_config":{"daily_budget":1000},"cells":[{"name":"A","treatment_percentage":50,"ads":["1"]},{"name":"B","treatment_percentage":50,"ads":["2"]}]});
        assert!(
            study_change(change(
                json!({"action":"create","business_id":"42","fields":fields})
            ))
            .is_ok()
        );
        fields["cells"][0]["treatment_percentage"] = json!(70);
        assert!(
            study_change(change(
                json!({"action":"create","business_id":"42","fields":fields})
            ))
            .is_err()
        );
    }
}
