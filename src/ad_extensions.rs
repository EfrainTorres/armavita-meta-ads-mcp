// Copyright (C) 2025 ArmaVita LLC
// SPDX-License-Identifier: AGPL-3.0-only

use std::path::Path;

use rmcp::{
    Json,
    handler::server::wrapper::Parameters,
    schemars::{self, JsonSchema},
    tool, tool_router,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    ad_automation::{AutomationChange, AutomationRead, acknowledge},
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    graph_tools::{self, GraphData, Params},
    media_uploads::{LocalMediaKind, normalize_remote_video_url, open_local_media},
    mutation_result::mutation_error_without_blind_retry,
    server::MetaAdsServer,
};

// Account and object contracts verified against the official Business SDK 26.0.1.
const PLACE_FIELDS: &str =
    "id,name,account_id,parent_page,location_types,pages_count,targeted_area_type";
const PLAYABLE_FIELDS: &str = "id,name,owner";
const CLOUD_FIELDS: &str =
    "id,name,playable_ad_status,playable_ad_reject_reason,playable_ad_file_size";
const ACCEPT_TERMS: &str = "ACCEPT_META_CUSTOM_AUDIENCE_TERMS";
const MAX_PLAYABLE_BYTES: u64 = 5 * 1024 * 1024;
const RECOMMENDATION_FIELDS: &[&str] = &[
    "asc_fragmentation_parameters",
    "autoflow_parameters",
    "extra_data",
    "fragmentation_parameters",
    "music_parameters",
    "recommendation_signature",
    "scale_good_campaign_parameters",
];

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AdExtensionRead {
    CreativeInsights {
        creative_id: String,
    },
    Playables {
        ad_account_id: String,
    },
    Playable {
        playable_id: String,
    },
    CloudPlayables {
        ad_account_id: String,
    },
    CloudPlayable {
        playable_id: String,
    },
    PlacePageSets {
        ad_account_id: String,
    },
    /// Read the set returned by synchronous or asynchronous creation.
    PlacePageSet {
        place_page_set_id: String,
    },
    TrackingDefaults {
        ad_account_id: String,
    },
    CustomAudienceTerms {
        ad_account_id: String,
    },
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AdExtensionChange {
    /// Apply an existing Meta recommendation; this can alter live delivery and budgets.
    ApplyRecommendation {
        ad_account_id: String,
        /// recommendation_signature and optional SDK recommendation parameter maps.
        fields: Map<String, Value>,
    },
    CreatePlayable {
        ad_account_id: String,
        name: String,
        app_id: Option<String>,
        session_id: Option<String>,
        source: PlayableSource,
    },
    CreatePlacePageSet {
        ad_account_id: String,
        name: String,
        parent_page_id: String,
        #[schemars(length(min = 1, max = 2))]
        location_types: Vec<PlaceLocation>,
        targeted_area_type: TargetedArea,
        /// Use Meta's asynchronous creation edge. Read the returned set to inspect it.
        #[serde(default)]
        asynchronous: bool,
    },
    SetTrackingDefaults {
        ad_account_id: String,
        /// Meta tracking_specs object or array to add at account level.
        tracking_specs: Value,
    },
    /// Accept legal terms only after the user has reviewed and authorized those terms.
    AcceptCustomAudienceTerms {
        ad_account_id: String,
        business_id: Option<String>,
        tos_id: String,
        #[schemars(
            length(min = 33, max = 33),
            regex(pattern = "^ACCEPT_META_CUSTOM_AUDIENCE_TERMS$")
        )]
        terms_acknowledgement: String,
    },
    TranslateValueRuleSet {
        ad_account_id: String,
        /// Meta's source object for value-rule translation.
        source: Map<String, Value>,
    },
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum PlayableSource {
    Url {
        url: String,
    },
    /// ZIP under META_MEDIA_ROOT, at most 5 MiB. The server streams it without extraction.
    LocalZip {
        relative_path: String,
    },
}

#[derive(Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PlaceLocation {
    Home,
    Recent,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum TargetedArea {
    CustomRadius,
    MarketingArea,
    None,
}

fn read_request(input: AutomationRead<AdExtensionRead>) -> Result<(String, Params), PublicError> {
    let (endpoint, defaults) = match input.operation {
        AdExtensionRead::CreativeInsights { creative_id } => (
            format!(
                "{}/creative_insights",
                graph_tools::id(&creative_id, "creative_id")?
            ),
            "aesthetics",
        ),
        AdExtensionRead::Playables { ad_account_id } => (
            account_edge(&ad_account_id, "adplayables")?,
            PLAYABLE_FIELDS,
        ),
        AdExtensionRead::Playable { playable_id } => (
            graph_tools::id(&playable_id, "playable_id")?,
            PLAYABLE_FIELDS,
        ),
        AdExtensionRead::CloudPlayables { ad_account_id } => (
            account_edge(&ad_account_id, "adcloudplayables")?,
            CLOUD_FIELDS,
        ),
        AdExtensionRead::CloudPlayable { playable_id } => {
            (graph_tools::id(&playable_id, "playable_id")?, CLOUD_FIELDS)
        }
        AdExtensionRead::PlacePageSets { ad_account_id } => (
            account_edge(&ad_account_id, "ad_place_page_sets")?,
            PLACE_FIELDS,
        ),
        AdExtensionRead::PlacePageSet { place_page_set_id } => (
            graph_tools::id(&place_page_set_id, "place_page_set_id")?,
            PLACE_FIELDS,
        ),
        AdExtensionRead::TrackingDefaults { ad_account_id } => {
            (account_edge(&ad_account_id, "tracking")?, "tracking_specs")
        }
        AdExtensionRead::CustomAudienceTerms { ad_account_id } => (
            account_edge(&ad_account_id, "customaudiencestos")?,
            "id,type,content",
        ),
    };
    Ok((
        endpoint,
        graph_tools::read_params(&input.options, defaults)?,
    ))
}

struct ExtensionWrite {
    endpoint: String,
    params: Params,
    zip_path: Option<String>,
}

fn write_request(
    input: AutomationChange<AdExtensionChange>,
) -> Result<ExtensionWrite, PublicError> {
    acknowledge(true, input.apply_acknowledgement.as_deref())?;
    let mut zip_path = None;
    let (endpoint, params) = match input.operation {
        AdExtensionChange::ApplyRecommendation {
            ad_account_id,
            fields,
        } => {
            let signature = fields
                .get("recommendation_signature")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("A recommendation_signature from Meta is required"))?;
            graph_tools::text(signature, "recommendation_signature", 4096)?;
            if fields
                .iter()
                .any(|(key, value)| key != "recommendation_signature" && !value.is_object())
            {
                return Err(invalid("Recommendation parameter fields must be objects"));
            }
            (
                account_edge(&ad_account_id, "recommendations")?,
                graph_tools::form_fields(&fields, RECOMMENDATION_FIELDS)?,
            )
        }
        AdExtensionChange::CreatePlayable {
            ad_account_id,
            name,
            app_id,
            session_id,
            source,
        } => {
            let mut params = vec![("name".into(), graph_tools::text(&name, "name", 255)?)];
            if let Some(app_id) = app_id {
                params.push(("app_id".into(), graph_tools::id(&app_id, "app_id")?));
            }
            if let Some(session_id) = session_id {
                params.push((
                    "session_id".into(),
                    graph_tools::text(&session_id, "session_id", 256)?,
                ));
            }
            match source {
                PlayableSource::Url { url } => {
                    graph_tools::text(&url, "url", 4096)?;
                    params.push(("source_url".into(), normalize_remote_video_url(&url)?));
                }
                PlayableSource::LocalZip { relative_path } => {
                    zip_path = Some(graph_tools::text(&relative_path, "relative_path", 1024)?);
                }
            }
            (account_edge(&ad_account_id, "adplayables")?, params)
        }
        AdExtensionChange::CreatePlacePageSet {
            ad_account_id,
            name,
            parent_page_id,
            location_types,
            targeted_area_type,
            asynchronous,
        } => {
            if location_types.is_empty()
                || location_types.len() > 2
                || (location_types.len() == 2 && location_types[0] == location_types[1])
            {
                return Err(invalid(
                    "Choose one or both distinct location types: home, recent",
                ));
            }
            let edge = if asynchronous {
                "ad_place_page_sets_async"
            } else {
                "ad_place_page_sets"
            };
            let area = match targeted_area_type {
                TargetedArea::CustomRadius => "CUSTOM_RADIUS",
                TargetedArea::MarketingArea => "MARKETING_AREA",
                TargetedArea::None => "NONE",
            };
            (
                account_edge(&ad_account_id, edge)?,
                vec![
                    ("name".into(), graph_tools::text(&name, "name", 255)?),
                    (
                        "parent_page".into(),
                        graph_tools::id(&parent_page_id, "parent_page_id")?,
                    ),
                    (
                        "location_types".into(),
                        graph_tools::json(
                            &serde_json::to_value(location_types).expect("enum serialization"),
                            "location_types",
                        )?,
                    ),
                    ("targeted_area_type".into(), area.into()),
                ],
            )
        }
        AdExtensionChange::SetTrackingDefaults {
            ad_account_id,
            tracking_specs,
        } => {
            if !tracking_specs.is_object() && !tracking_specs.is_array() {
                return Err(invalid(
                    "tracking_specs must be a Meta tracking object or array",
                ));
            }
            (
                account_edge(&ad_account_id, "tracking")?,
                vec![(
                    "tracking_specs".into(),
                    graph_tools::json(&tracking_specs, "tracking_specs")?,
                )],
            )
        }
        AdExtensionChange::AcceptCustomAudienceTerms {
            ad_account_id,
            business_id,
            tos_id,
            terms_acknowledgement,
        } => {
            if terms_acknowledgement != ACCEPT_TERMS {
                return Err(invalid(
                    "Explicit ACCEPT_META_CUSTOM_AUDIENCE_TERMS acknowledgement is required",
                ));
            }
            let mut params = vec![("tos_id".into(), graph_tools::text(&tos_id, "tos_id", 256)?)];
            if let Some(business_id) = business_id {
                params.push((
                    "business_id".into(),
                    graph_tools::id(&business_id, "business_id")?,
                ));
            }
            (account_edge(&ad_account_id, "customaudiencestos")?, params)
        }
        AdExtensionChange::TranslateValueRuleSet {
            ad_account_id,
            source,
        } => {
            if source.is_empty() {
                return Err(invalid("source must be a nonempty Meta translation object"));
            }
            (
                account_edge(&ad_account_id, "value_rule_set_translation")?,
                vec![(
                    "source".into(),
                    graph_tools::json(&Value::Object(source), "source")?,
                )],
            )
        }
    };
    Ok(ExtensionWrite {
        endpoint,
        params,
        zip_path,
    })
}

fn account_edge(account: &str, edge: &str) -> Result<String, PublicError> {
    Ok(format!("{}/{edge}", graph_tools::account(account)?))
}

async fn write_extension(
    graph: &GraphClient,
    media_root: Option<&Path>,
    request: ExtensionWrite,
) -> ToolResponse<GraphData> {
    let Some(path) = request.zip_path else {
        return graph_tools::write(graph, &request.endpoint, request.params).await;
    };
    let media = match open_local_media(
        media_root,
        &path,
        LocalMediaKind::PlayableArchive,
        MAX_PLAYABLE_BYTES,
    )
    .await
    {
        Ok(media) => media,
        Err(error) => return ToolResponse::error(error),
    };
    graph_tools::response(
        graph
            .post_multipart_file_json(
                &request.endpoint,
                "source_zip",
                media.file,
                media.size,
                media.file_name.into(),
                media.mime_type,
                request.params,
            )
            .await
            .map_err(|error| {
                mutation_error_without_blind_retry(
                    error,
                    "List account playables before attempting another upload",
                )
            })
            .and_then(graph_tools::normalize_write),
    )
}

fn invalid(message: &str) -> PublicError {
    PublicError::invalid_input(
        message,
        "Use the documented operation fields and explicit acknowledgements",
    )
}

#[tool_router(router = extensions_router, vis = "pub(crate)")]
impl MetaAdsServer {
    #[tool(
        name = "read_ad_extensions",
        description = "Inspect creative insights, playable assets, store-location Page sets, account tracking defaults or custom-audience terms.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn read_ad_extensions(
        &self,
        Parameters(input): Parameters<AutomationRead<AdExtensionRead>>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        match read_request(input) {
            Ok((endpoint, params)) => graph_tools::read(&self.graph, &endpoint, params).await,
            Err(error) => ToolResponse::error(error),
        }
        .into_mcp_result()
    }

    #[tool(
        name = "manage_ad_extensions",
        description = "Apply a recommendation, create playable assets or store-location Page sets, set tracking defaults, translate value rules, or explicitly accept audience terms. Writes require approval and are sent once.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn manage_ad_extensions(
        &self,
        Parameters(input): Parameters<AutomationChange<AdExtensionChange>>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        match write_request(input) {
            Ok(request) => write_extension(&self.graph, self.media_root.as_deref(), request).await,
            Err(error) => ToolResponse::error(error),
        }
        .into_mcp_result()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn change(operation: Value) -> Result<ExtensionWrite, PublicError> {
        write_request(
            serde_json::from_value(json!({
                "operation": operation,
                "apply_acknowledgement": "APPLY_LIVE_META_ADS_CHANGES"
            }))
            .unwrap(),
        )
    }

    #[test]
    fn utility_contracts_keep_account_paths_and_explicit_terms() {
        let mut operation = json!({"action":"accept_custom_audience_terms", "ad_account_id":"42", "business_id":"43", "tos_id":"44", "terms_acknowledgement":"yes"});
        assert!(change(operation.clone()).is_err());
        operation["terms_acknowledgement"] = ACCEPT_TERMS.into();
        let request = change(operation).unwrap();
        assert_eq!(request.endpoint, "act_42/customaudiencestos");
        assert_eq!(
            request.params,
            [
                ("tos_id".into(), "44".into()),
                ("business_id".into(), "43".into())
            ]
        );
        let place = change(json!({"action":"create_place_page_set", "ad_account_id":"42", "name":"Stores", "parent_page_id":"43", "location_types":["home","recent"], "targeted_area_type":"CUSTOM_RADIUS", "asynchronous":true})).unwrap();
        assert_eq!(place.endpoint, "act_42/ad_place_page_sets_async");
        assert!(
            place
                .params
                .contains(&("location_types".into(), "[\"home\",\"recent\"]".into()))
        );
        let insights = read_request(
            serde_json::from_value(
                json!({"operation":{"action":"creative_insights","creative_id":"77"}}),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(insights.0, "77/creative_insights");
        assert!(insights.1.contains(&("fields".into(), "aesthetics".into())));
        assert!(change(json!({"action":"apply_recommendation", "ad_account_id":"42", "fields":{"recommendation_signature":"abc","access_token":"secret"}})).is_err());
        assert!(change(json!({"action":"set_tracking_defaults", "ad_account_id":"42", "tracking_specs":{"token":"secret"}})).is_err());
        assert!(write_request(serde_json::from_value(json!({"operation":{"action":"set_tracking_defaults","ad_account_id":"42","tracking_specs":[]}})).unwrap()).is_err());
    }

    #[test]
    fn playable_sources_are_exclusive_and_guarded() {
        let base = json!({"action":"create_playable","ad_account_id":"42","name":"Demo","source":{"type":"url","url":"https://cdn.example.com/demo.html"}});
        let request = change(base.clone()).unwrap();
        assert_eq!(request.endpoint, "act_42/adplayables");
        assert!(request.params.contains(&(
            "source_url".into(),
            "https://cdn.example.com/demo.html".into()
        )));
        for url in [
            "file:///tmp/demo.html",
            "https://user:secret@example.com/demo.html",
            "https://127.0.0.1/demo.html",
            "https://example.com/demo.html?access_token=secret",
        ] {
            let mut bad = base.clone();
            bad["source"]["url"] = url.into();
            assert!(change(bad).is_err());
        }
        let mut zip = base;
        zip["source"] = json!({"type":"local_zip","relative_path":"demo.zip"});
        let request = change(zip).unwrap();
        assert_eq!(request.zip_path.as_deref(), Some("demo.zip"));
        assert!(!request.params.iter().any(|(key, _)| key == "source_url"));
    }
}
