// Copyright (C) 2025 ArmaVita LLC
// SPDX-License-Identifier: AGPL-3.0-only

use rmcp::{
    Json,
    handler::server::wrapper::Parameters,
    schemars::{self, JsonSchema},
    tool, tool_router,
};
use serde::Deserialize;

use crate::{
    ad_automation::{AutomationChange, AutomationRead, acknowledge},
    campaign_mutations::CampaignBudgetScheduleValue,
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    graph_tools::{self, GraphData, Params},
    node_identity::{MetaNodeKind, verify_meta_node, verify_object},
    safety::validate_removal_acknowledgement,
    server::MetaAdsServer,
};

// SDK 26.0.1 Campaign/AdSet budget_schedules and HighDemandPeriod node methods.
const FIELDS: &str = "id,ad_object_id,budget_value,budget_value_type,time_start,time_end";

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum BudgetScheduleRead {
    List {
        /// Campaign or ad-set ID.
        ad_object_id: String,
        time_start: Option<u32>,
        /// Read-filter spelling is time_stop; writes use time_end.
        time_stop: Option<u32>,
    },
    Read {
        schedule_id: String,
    },
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum BudgetScheduleChange {
    /// Campaign creation remains available in create_campaign_budget_schedule.
    CreateAdSet {
        ad_account_id: String,
        ad_set_id: String,
        budget: CampaignBudgetScheduleValue,
        time_start: u32,
        time_end: u32,
    },
    Update {
        schedule_id: String,
        budget: Option<CampaignBudgetScheduleValue>,
        /// When changing dates, provide both start and end.
        time_start: Option<u32>,
        time_end: Option<u32>,
    },
    Delete {
        schedule_id: String,
    },
}

fn read_request(
    input: AutomationRead<BudgetScheduleRead>,
) -> Result<(String, Params), PublicError> {
    let mut params = graph_tools::read_params(&input.options, FIELDS)?;
    let endpoint = match input.operation {
        BudgetScheduleRead::List {
            ad_object_id,
            time_start,
            time_stop,
        } => {
            if let (Some(start), Some(stop)) = (time_start, time_stop)
                && stop <= start
            {
                return Err(invalid("time_stop must be after time_start"));
            }
            for (key, time) in [("time_start", time_start), ("time_stop", time_stop)] {
                if let Some(time) = time {
                    if time == 0 {
                        return Err(invalid("Time filters must be positive Unix timestamps"));
                    }
                    params.push((key.into(), time.to_string()));
                }
            }
            format!(
                "{}/budget_schedules",
                graph_tools::id(&ad_object_id, "ad_object_id")?
            )
        }
        BudgetScheduleRead::Read { schedule_id } => graph_tools::id(&schedule_id, "schedule_id")?,
    };
    Ok((endpoint, params))
}

struct ScheduleWrite {
    endpoint: String,
    params: Params,
    owner_account: Option<String>,
    delete: bool,
}

fn write_request(
    input: AutomationChange<BudgetScheduleChange>,
) -> Result<ScheduleWrite, PublicError> {
    acknowledge(true, input.apply_acknowledgement.as_deref())?;
    validate_removal_acknowledgement(
        matches!(input.operation, BudgetScheduleChange::Delete { .. }),
        input.removal_acknowledgement.as_deref(),
    )?;
    match input.operation {
        BudgetScheduleChange::CreateAdSet {
            ad_account_id,
            ad_set_id,
            budget,
            time_start,
            time_end,
        } => Ok(ScheduleWrite {
            endpoint: format!(
                "{}/budget_schedules",
                graph_tools::id(&ad_set_id, "ad_set_id")?
            ),
            params: schedule_fields(Some(budget), Some(time_start), Some(time_end))?,
            owner_account: Some(graph_tools::account(&ad_account_id)?),
            delete: false,
        }),
        BudgetScheduleChange::Update {
            schedule_id,
            budget,
            time_start,
            time_end,
        } => Ok(ScheduleWrite {
            endpoint: graph_tools::id(&schedule_id, "schedule_id")?,
            params: schedule_fields(budget, time_start, time_end)?,
            owner_account: None,
            delete: false,
        }),
        BudgetScheduleChange::Delete { schedule_id } => Ok(ScheduleWrite {
            endpoint: graph_tools::id(&schedule_id, "schedule_id")?,
            params: Vec::new(),
            owner_account: None,
            delete: true,
        }),
    }
}

fn schedule_fields(
    budget: Option<CampaignBudgetScheduleValue>,
    start: Option<u32>,
    end: Option<u32>,
) -> Result<Params, PublicError> {
    let mut params = Vec::new();
    if let Some(budget) = budget {
        let (kind, value) = match budget {
            CampaignBudgetScheduleValue::Absolute { value } => ("ABSOLUTE", value),
            CampaignBudgetScheduleValue::Multiplier { value } => ("MULTIPLIER", value),
        };
        if value == 0 {
            return Err(invalid("budget value must be positive"));
        }
        params.extend([
            ("budget_value".into(), value.to_string()),
            ("budget_value_type".into(), kind.into()),
        ]);
    }
    match (start, end) {
        (None, None) => {}
        (Some(start), Some(end)) if start > 0 && end > start => params.extend([
            ("time_start".into(), start.to_string()),
            ("time_end".into(), end.to_string()),
        ]),
        _ => {
            return Err(invalid(
                "Provide both positive time_start and time_end, with end after start",
            ));
        }
    }
    if params.is_empty() {
        return Err(invalid("Set a budget adjustment or a time interval"));
    }
    Ok(params)
}

async fn write_schedule(graph: &GraphClient, request: ScheduleWrite) -> ToolResponse<GraphData> {
    let proof = match &request.owner_account {
        Some(account) => {
            let ad_set = request
                .endpoint
                .strip_suffix("/budget_schedules")
                .expect("validated endpoint");
            verify_meta_node(graph, ad_set, MetaNodeKind::AdSet, Some(account))
                .await
                .map(|_| ())
        }
        None => verify_object(
            graph,
            &request.endpoint,
            "budget_value_type,time_start,time_end",
            &["budget_value_type", "time_start", "time_end"],
        )
        .await
        .map(|_| ()),
    };
    if let Err(error) = proof {
        return ToolResponse::error(error);
    }
    if request.delete {
        graph_tools::delete(graph, &request.endpoint, request.params).await
    } else {
        graph_tools::write(graph, &request.endpoint, request.params).await
    }
}

fn invalid(message: &str) -> PublicError {
    PublicError::invalid_input(
        message,
        "Use a positive budget adjustment and an ordered Unix time interval",
    )
}

#[tool_router(router = budget_schedules_router, vis = "pub(crate)")]
impl MetaAdsServer {
    #[tool(
        name = "read_budget_schedules",
        description = "List campaign/ad-set budget schedules or read a high-demand period.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn read_budget_schedules(
        &self,
        Parameters(input): Parameters<AutomationRead<BudgetScheduleRead>>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        match read_request(input) {
            Ok((endpoint, params)) => graph_tools::read(&self.graph, &endpoint, params).await,
            Err(error) => ToolResponse::error(error),
        }
        .into_mcp_result()
    }

    #[tool(
        name = "manage_budget_schedules",
        description = "Create an ad-set budget schedule or update/delete a campaign or ad-set schedule. Changes can affect live spending.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn manage_budget_schedules(
        &self,
        Parameters(input): Parameters<AutomationChange<BudgetScheduleChange>>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        match write_request(input) {
            Ok(request) => write_schedule(&self.graph, request).await,
            Err(error) => ToolResponse::error(error),
        }
        .into_mcp_result()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn budget_schedule_contract_uses_correct_read_and_write_time_names() {
        let input = serde_json::from_value(json!({"operation":{"action":"list","ad_object_id":"12","time_start":10,"time_stop":20}})).unwrap();
        let (path, params) = read_request(input).unwrap();
        assert_eq!(path, "12/budget_schedules");
        assert!(params.contains(&("time_stop".into(), "20".into())));
        let input = serde_json::from_value(json!({"apply_acknowledgement":"APPLY_LIVE_META_ADS_CHANGES","operation":{"action":"create_ad_set","ad_account_id":"1","ad_set_id":"12","budget":{"kind":"multiplier","value":150},"time_start":10,"time_end":20}})).unwrap();
        let request = write_request(input).unwrap();
        assert_eq!(request.endpoint, "12/budget_schedules");
        assert_eq!(request.owner_account.as_deref(), Some("act_1"));
        assert!(request.params.contains(&("time_end".into(), "20".into())));
        assert!(schedule_fields(None, Some(10), None).is_err());
        assert!(
            schedule_fields(
                Some(CampaignBudgetScheduleValue::Absolute { value: 0 }),
                None,
                None
            )
            .is_err()
        );
        let deletion = serde_json::from_value(json!({"apply_acknowledgement":"APPLY_LIVE_META_ADS_CHANGES","operation":{"action":"delete","schedule_id":"1"}})).unwrap();
        assert!(write_request(deletion).is_err());
    }
}
