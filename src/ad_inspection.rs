// Copyright (C) 2026 ArmaVita LLC
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
    graph_tools::{self, GraphData, Params, ReadOptions},
    server::MetaAdsServer,
};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct InspectAdObjectInput {
    pub kind: AdObjectKind,
    pub object_id: String,
    /// Opt into large specs, review feedback or diagnostics by selecting simple v26 field names.
    #[schemars(length(min = 1, max = 30), inner(length(min = 1, max = 64)))]
    pub fields: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AdObjectKind {
    Account,
    Campaign,
    AdSet,
    Ad,
    Creative,
    CustomAudience,
    SavedAudience,
    Video,
    Label,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadAdInventoryInput {
    pub target: AdInventoryTarget,
    pub options: Option<ReadOptions>,
    /// Only the documented parameters for the selected collection are accepted.
    pub filters: Option<Map<String, Value>>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AdInventoryTarget {
    Account {
        ad_account_id: String,
        collection: AccountCollection,
    },
    Audience {
        custom_audience_id: String,
        collection: AudienceCollection,
    },
    Label {
        label_id: String,
        collection: LabelCollection,
    },
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AccountCollection {
    Creatives,
    SavedAudiences,
    Labels,
    CreativesByLabels,
    AdsByLabels,
    AdSetsByLabels,
    CampaignsByLabels,
    AudienceFunnel,
    AdVolume,
    /// Includes the signature needed to apply a recommendation.
    Recommendations,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AudienceCollection {
    Health,
    Sessions,
    SharedAccounts,
    SharedAccountInfo,
    Ads,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LabelCollection {
    Creatives,
    Ads,
    AdSets,
    Campaigns,
}

fn inventory_request(input: &ReadAdInventoryInput) -> Result<(String, Params), PublicError> {
    let (parent, edge, defaults, allowed): (String, &str, &str, &[&str]) = match &input.target {
        AdInventoryTarget::Account {
            ad_account_id,
            collection,
        } => {
            let parent = graph_tools::account(ad_account_id)?;
            let (edge, defaults, allowed): (&str, &str, &[&str]) = match collection {
                AccountCollection::Creatives => ("adcreatives", "id,name,status,object_type", &[]),
                AccountCollection::SavedAudiences => (
                    "saved_audiences",
                    "id,name,targeting",
                    &["business_id", "filtering"],
                ),
                AccountCollection::Labels => ("adlabels", "id,name", &[]),
                AccountCollection::CreativesByLabels => (
                    "adcreativesbylabels",
                    "id,name",
                    &["ad_label_ids", "operator"],
                ),
                AccountCollection::AdsByLabels => (
                    "adsbylabels",
                    "id,name,status",
                    &["ad_label_ids", "operator"],
                ),
                AccountCollection::AdSetsByLabels => (
                    "adsetsbylabels",
                    "id,name,status",
                    &["ad_label_ids", "operator"],
                ),
                AccountCollection::CampaignsByLabels => (
                    "campaignsbylabels",
                    "id,name,status",
                    &["ad_label_ids", "operator"],
                ),
                AccountCollection::AudienceFunnel => ("audience_funnel", "", &[]),
                AccountCollection::AdVolume => (
                    "ads_volume",
                    "",
                    &["page_id", "recommendation_type", "show_breakdown_by_actor"],
                ),
                AccountCollection::Recommendations => (
                    "recommendations",
                    "recommendations",
                    &["recommendation_names", "recommendation_stages"],
                ),
            };
            (parent, edge, defaults, allowed)
        }
        AdInventoryTarget::Audience {
            custom_audience_id,
            collection,
        } => {
            let parent = graph_tools::id(custom_audience_id, "custom_audience_id")?;
            let (edge, defaults, allowed): (&str, &str, &[&str]) = match collection {
                AudienceCollection::Health => (
                    "health",
                    "",
                    &[
                        "calculated_date",
                        "processed_date",
                        "value_aggregation_duration",
                        "value_country",
                        "value_currency",
                        "value_version",
                    ],
                ),
                AudienceCollection::Sessions => ("sessions", "", &["session_id"]),
                AudienceCollection::SharedAccounts => ("adaccounts", "id,name", &["permissions"]),
                AudienceCollection::SharedAccountInfo => ("shared_account_info", "", &[]),
                AudienceCollection::Ads => ("ads", "id,name,status", &["effective_status"]),
            };
            (parent, edge, defaults, allowed)
        }
        AdInventoryTarget::Label {
            label_id,
            collection,
        } => (
            graph_tools::id(label_id, "label_id")?,
            match collection {
                LabelCollection::Creatives => "adcreatives",
                LabelCollection::Ads => "ads",
                LabelCollection::AdSets => "adsets",
                LabelCollection::Campaigns => "campaigns",
            },
            "id,name",
            &[],
        ),
    };
    let mut params = graph_tools::read_params(
        input.options.as_ref().unwrap_or(&ReadOptions::default()),
        defaults,
    )?;
    if let Some(filters) = &input.filters {
        params.extend(graph_tools::form_fields(filters, allowed)?);
    }
    Ok((format!("{parent}/{edge}"), params))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct QueryTargetingInput {
    pub ad_account_id: String,
    pub operation: TargetingOperation,
    pub parameters: Map<String, Value>,
    pub options: Option<ReadOptions>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TargetingOperation {
    Browse,
    Search,
    Suggestions,
    Validate,
    Describe,
    DeliveryEstimate,
    MinimumBudgets,
    BroadCategories,
}

fn targeting_request(input: &QueryTargetingInput) -> Result<(String, Params), PublicError> {
    let parent = graph_tools::account(&input.ad_account_id)?;
    let (edge, allowed): (&str, &[&str]) = match input.operation {
        TargetingOperation::Browse => (
            "targetingbrowse",
            &[
                "excluded_category",
                "include_nodes",
                "is_exclusion",
                "is_reserved",
                "limit_type",
                "optimization_goal",
                "regulated_categories",
                "regulated_countries",
                "whitelisted_types",
            ],
        ),
        TargetingOperation::Search => (
            "targetingsearch",
            &[
                "allow_only_fat_head_interests",
                "app_store",
                "countries",
                "is_account_level_brand_safety_exclusion",
                "is_account_level_employer_exclusion",
                "is_exclusion",
                "is_reserved",
                "limit_type",
                "objective",
                "optimization_goal",
                "promoted_object",
                "q",
                "regulated_categories",
                "regulated_countries",
                "session_id",
                "targeting_list",
                "whitelisted_types",
            ],
        ),
        TargetingOperation::Suggestions => (
            "targetingsuggestions",
            &[
                "app_store",
                "countries",
                "limit_type",
                "mode",
                "objective",
                "objects",
                "regulated_categories",
                "regulated_countries",
                "session_id",
                "targeting_list",
                "whitelisted_types",
            ],
        ),
        TargetingOperation::Validate => (
            "targetingvalidation",
            &["id_list", "is_exclusion", "name_list", "targeting_list"],
        ),
        TargetingOperation::Describe => (
            "targetingsentencelines",
            &[
                "discard_ages",
                "discard_placements",
                "hide_targeting_spec_from_return",
                "targeting_spec",
            ],
        ),
        TargetingOperation::DeliveryEstimate => (
            "delivery_estimate",
            &["optimization_goal", "promoted_object", "targeting_spec"],
        ),
        TargetingOperation::MinimumBudgets => ("minimum_budgets", &["bid_amount"]),
        TargetingOperation::BroadCategories => {
            ("broadtargetingcategories", &["custom_categories_only"])
        }
    };
    let mut params = graph_tools::read_params(
        input.options.as_ref().unwrap_or(&ReadOptions::default()),
        "",
    )?;
    params.extend(graph_tools::form_fields(&input.parameters, allowed)?);
    Ok((format!("{parent}/{edge}"), params))
}

#[tool_router(router = ad_inspection_router, vis = "pub(crate)")]
impl MetaAdsServer {
    #[tool(
        name = "inspect_ad_object",
        description = "Read selected v26 fields from an ad object. Use for full creative/targeting specs, review feedback, delivery issues, attribution settings, media status or audience rules; request only what is needed.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn inspect_ad_object(
        &self,
        Parameters(input): Parameters<InspectAdObjectInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        let request = (|| {
            let id = if matches!(input.kind, AdObjectKind::Account) {
                graph_tools::account(&input.object_id)?
            } else {
                graph_tools::id(&input.object_id, "object_id")?
            };
            let mut params = graph_tools::read_params(
                &ReadOptions {
                    fields: Some(input.fields),
                    ..Default::default()
                },
                "",
            )?;
            params.retain(|(key, _)| key != "limit");
            Ok::<_, PublicError>((id, params))
        })();
        match request {
            Ok((path, params)) => graph_tools::read(&self.graph, &path, params).await,
            Err(error) => ToolResponse::error(error),
        }
        .into_mcp_result()
    }

    #[tool(
        name = "read_ad_inventory",
        description = "Read account creatives, saved audiences, labels, ad volume and recommendation signatures, or audience health, upload sessions and sharing. Use list_business_assets for change history.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn read_ad_inventory(
        &self,
        Parameters(input): Parameters<ReadAdInventoryInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        match inventory_request(&input) {
            Ok((path, params)) => graph_tools::read(&self.graph, &path, params).await,
            Err(error) => ToolResponse::error(error),
        }
        .into_mcp_result()
    }

    #[tool(
        name = "query_targeting",
        description = "Browse, search, suggest, validate or describe account targeting; estimate delivery and read minimum budgets. Accepts bounded documented v26 parameters for the selected operation.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn query_targeting(
        &self,
        Parameters(input): Parameters<QueryTargetingInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        match targeting_request(&input) {
            Ok((path, params)) => graph_tools::read(&self.graph, &path, params).await,
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
    fn inventory_and_targeting_only_reach_verified_bounded_edges() {
        let input: ReadAdInventoryInput = serde_json::from_value(
            json!({"target":{"kind":"account","ad_account_id":"123","collection":"creatives"}}),
        )
        .unwrap();
        let (path, params) = inventory_request(&input).unwrap();
        assert_eq!(path, "act_123/adcreatives");
        assert!(params.contains(&("limit".into(), "25".into())));
        let mut input: QueryTargetingInput = serde_json::from_value(json!({"ad_account_id":"123","operation":"delivery_estimate","parameters":{"optimization_goal":"LINK_CLICKS","targeting_spec":{"geo_locations":{"countries":["US"]}}}})).unwrap();
        assert_eq!(
            targeting_request(&input).unwrap().0,
            "act_123/delivery_estimate"
        );
        input.parameters.insert("method".into(), json!("DELETE"));
        assert!(targeting_request(&input).is_err());
        for operation in ["browse", "search"] {
            let input: QueryTargetingInput = serde_json::from_value(json!({
                "ad_account_id":"123", "operation":operation,
                "parameters":{"is_reserved":false,"optimization_goal":"LINK_CLICKS"}
            }))
            .unwrap();
            let (_, params) = targeting_request(&input).unwrap();
            assert!(params.contains(&("is_reserved".into(), "false".into())));
        }
        let input: ReadAdInventoryInput = serde_json::from_value(json!({
            "target":{"kind":"account","ad_account_id":"123","collection":"recommendations"},
            "filters":{"recommendation_stages":["ACTIVE"]}
        }))
        .unwrap();
        assert_eq!(
            inventory_request(&input).unwrap().0,
            "act_123/recommendations"
        );
        let result = graph_tools::normalize_write(json!({"data":[{"recommendations":[{
            "recommendation_signature":"opaque-recommendation-reference"
        }]}]}))
        .unwrap();
        assert_eq!(
            result.result[0]["recommendations"][0]["recommendation_signature"],
            "opaque-recommendation-reference"
        );
    }
}
