// Copyright (C) 2026 ArmaVita LLC
// SPDX-License-Identifier: AGPL-3.0-only

use rmcp::{
    Json,
    handler::server::wrapper::Parameters,
    schemars::{self, JsonSchema},
    tool, tool_router,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    graph_tools::{self, GraphData, Params},
    meta_ids,
    node_identity::{MetaNodeKind, verify_meta_node, verify_object},
    safety::validate_removal_acknowledgement,
    server::MetaAdsServer,
};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManageAdLabelInput {
    pub ad_account_id: String,
    pub action: AdLabelAction,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AdLabelAction {
    Create {
        #[schemars(length(min = 1, max = 255))]
        name: String,
    },
    Rename {
        label_id: String,
        #[schemars(length(min = 1, max = 255))]
        name: String,
    },
    Delete {
        label_id: String,
        #[schemars(
            length(min = 25, max = 25),
            regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
        )]
        removal_acknowledgement: String,
    },
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LabelObjectKind {
    Campaign,
    AdSet,
    Ad,
    Creative,
}

impl LabelObjectKind {
    fn node(self) -> MetaNodeKind {
        match self {
            Self::Campaign => MetaNodeKind::Campaign,
            Self::AdSet => MetaNodeKind::AdSet,
            Self::Ad => MetaNodeKind::Ad,
            Self::Creative => MetaNodeKind::AdCreative,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChangeAdLabelsInput {
    pub ad_account_id: String,
    pub object_id: String,
    pub object_kind: LabelObjectKind,
    #[schemars(length(min = 1, max = 50), inner(length(min = 1, max = 64)))]
    pub label_ids: Vec<String>,
    pub action: LabelAssignmentAction,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum LabelAssignmentAction {
    Add,
    Remove {
        #[schemars(
            length(min = 25, max = 25),
            regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
        )]
        removal_acknowledgement: String,
    },
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeleteAdMediaInput {
    pub ad_account_id: String,
    pub media: AdLibraryMedia,
    #[schemars(
        length(min = 25, max = 25),
        regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
    )]
    pub removal_acknowledgement: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AdLibraryMedia {
    ImageHash {
        #[schemars(length(min = 1, max = 256))]
        image_hash: String,
    },
    ImageId {
        #[schemars(length(min = 1, max = 256))]
        image_id: String,
    },
    Video {
        video_id: String,
    },
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShareCustomAudienceInput {
    pub ad_account_id: String,
    pub custom_audience_id: String,
    #[schemars(length(min = 1, max = 25), inner(length(min = 1, max = 68)))]
    pub recipient_ad_account_ids: Vec<String>,
    pub action: AudienceSharingAction,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AudienceSharingAction {
    /// Grant access without removing existing recipients. Default permission is targeting only.
    Add {
        permissions: Option<AudienceSharingPermission>,
        #[schemars(length(min = 1, max = 25), inner(length(min = 1, max = 64)))]
        relationship_type: Option<Vec<String>>,
    },
    /// Removing access can stop the recipient's ads that use this audience.
    Remove {
        #[schemars(
            length(min = 25, max = 25),
            regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
        )]
        removal_acknowledgement: String,
    },
}

#[derive(Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AudienceSharingPermission {
    #[default]
    Targeting,
    TargetingAndInsights,
}

fn invalid(message: &str) -> PublicError {
    PublicError::invalid_input(
        message,
        "Use IDs from the specified ad account and acknowledge removals",
    )
}

fn label_ids(raw: &[String]) -> Result<Vec<String>, PublicError> {
    if raw.is_empty() || raw.len() > 50 {
        return Err(invalid("label_ids must contain 1 through 50 IDs"));
    }
    let ids = raw
        .iter()
        .map(|id| graph_tools::id(id, "label_id"))
        .collect::<Result<Vec<_>, _>>()?;
    let unique = ids.iter().collect::<std::collections::HashSet<_>>();
    if unique.len() != ids.len() {
        return Err(invalid("label_ids must not contain duplicates"));
    }
    Ok(ids)
}

fn labels_param(ids: &[String]) -> Result<Params, PublicError> {
    Ok(vec![(
        "adlabels".into(),
        graph_tools::json(
            &json!(ids.iter().map(|id| json!({"id":id})).collect::<Vec<_>>()),
            "adlabels",
        )?,
    )])
}

async fn verify_label(graph: &GraphClient, id: &str, account: &str) -> Result<(), PublicError> {
    let payload = verify_object(graph, id, "id,name,account", &["account"]).await?;
    let owner = payload.pointer("/account/id");
    let owner = owner
        .and_then(Value::as_str)
        .and_then(meta_ids::ad_account)
        .or_else(|| {
            owner
                .and_then(meta_ids::numeric_value)
                .and_then(|id| meta_ids::ad_account(&id))
        });
    if owner.as_deref() != Some(account) {
        return Err(invalid("The label does not belong to ad_account_id"));
    }
    Ok(())
}

async fn manage_label(graph: &GraphClient, input: ManageAdLabelInput) -> ToolResponse<GraphData> {
    let prepared = (|| {
        let account = graph_tools::account(&input.ad_account_id)?;
        let (label, params, delete) = match &input.action {
            AdLabelAction::Create { name } => (
                None,
                vec![("name".into(), graph_tools::text(name, "name", 255)?)],
                false,
            ),
            AdLabelAction::Rename { label_id, name } => (
                Some(graph_tools::id(label_id, "label_id")?),
                vec![("name".into(), graph_tools::text(name, "name", 255)?)],
                false,
            ),
            AdLabelAction::Delete {
                label_id,
                removal_acknowledgement,
            } => {
                validate_removal_acknowledgement(true, Some(removal_acknowledgement))?;
                (
                    Some(graph_tools::id(label_id, "label_id")?),
                    Vec::new(),
                    true,
                )
            }
        };
        Ok::<_, PublicError>((account, label, params, delete))
    })();
    let (account, label, params, delete) = match prepared {
        Ok(v) => v,
        Err(e) => return ToolResponse::error(e),
    };
    let endpoint = match label {
        Some(id) => {
            if let Err(e) = verify_label(graph, &id, &account).await {
                return ToolResponse::error(e);
            }
            id
        }
        None => format!("{account}/adlabels"),
    };
    if delete {
        graph_tools::delete(graph, &endpoint, params).await
    } else {
        graph_tools::write(graph, &endpoint, params).await
    }
}

fn remaining_creative_labels(
    payload: &Value,
    object_id: &str,
    removed: &[String],
) -> Result<Vec<String>, PublicError> {
    if payload
        .get("id")
        .and_then(meta_ids::numeric_value)
        .as_deref()
        != Some(object_id)
    {
        return Err(invalid("Meta did not confirm the creative identity"));
    }
    // A complete list is required: never replace a paginated/partial response and lose other labels.
    let existing = payload
        .get("adlabels")
        .and_then(Value::as_array)
        .filter(|labels| labels.len() <= 50)
        .ok_or_else(|| invalid("Meta did not return a complete bounded creative label list"))?;
    let mut remaining = Vec::new();
    for label in existing {
        let id = label
            .get("id")
            .and_then(meta_ids::numeric_value)
            .ok_or_else(|| invalid("Meta returned an invalid creative label"))?;
        if !removed.contains(&id) {
            remaining.push(id);
        }
    }
    Ok(remaining)
}

async fn change_labels(graph: &GraphClient, input: ChangeAdLabelsInput) -> ToolResponse<GraphData> {
    let prepared = (|| {
        let account = graph_tools::account(&input.ad_account_id)?;
        let object = graph_tools::id(&input.object_id, "object_id")?;
        let labels = label_ids(&input.label_ids)?;
        if let LabelAssignmentAction::Remove {
            removal_acknowledgement,
        } = &input.action
        {
            validate_removal_acknowledgement(true, Some(removal_acknowledgement))?;
        }
        Ok::<_, PublicError>((account, object, labels))
    })();
    let (account, object, labels) = match prepared {
        Ok(v) => v,
        Err(e) => return ToolResponse::error(e),
    };
    if let Err(e) = verify_meta_node(graph, &object, input.object_kind.node(), Some(&account)).await
    {
        return ToolResponse::error(e);
    }
    for label in &labels {
        if let Err(e) = verify_label(graph, label, &account).await {
            return ToolResponse::error(e);
        }
    }
    let remove = matches!(input.action, LabelAssignmentAction::Remove { .. });
    if remove && matches!(input.object_kind, LabelObjectKind::Creative) {
        let remaining = match graph
            .get_json(&object, &[("fields".into(), "id,adlabels".into())])
            .await
        {
            Ok(payload) => remaining_creative_labels(&payload, &object, &labels),
            Err(e) => Err(e.into()),
        };
        return match remaining.and_then(|ids| labels_param(&ids)) {
            Ok(params) => graph_tools::write(graph, &object, params).await,
            Err(e) => ToolResponse::error(e),
        };
    }
    let params = match labels_param(&labels) {
        Ok(v) => v,
        Err(e) => return ToolResponse::error(e),
    };
    let endpoint = format!("{object}/adlabels");
    if remove {
        graph_tools::delete(graph, &endpoint, params).await
    } else {
        graph_tools::write(graph, &endpoint, params).await
    }
}

fn media_delete_request(input: &DeleteAdMediaInput) -> Result<(String, Params), PublicError> {
    let account = graph_tools::account(&input.ad_account_id)?;
    validate_removal_acknowledgement(true, Some(&input.removal_acknowledgement))?;
    let (edge, key, value) = match &input.media {
        AdLibraryMedia::ImageHash { image_hash } => (
            "adimages",
            "hash",
            graph_tools::text(image_hash, "image_hash", 256)?,
        ),
        AdLibraryMedia::ImageId { image_id } => (
            "adimages",
            "image_id",
            graph_tools::text(image_id, "image_id", 256)?,
        ),
        AdLibraryMedia::Video { video_id } => (
            "advideos",
            "video_id",
            graph_tools::id(video_id, "video_id")?,
        ),
    };
    if !value
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b':'))
    {
        return Err(invalid("Media identifiers contain invalid characters"));
    }
    Ok((format!("{account}/{edge}"), vec![(key.into(), value)]))
}

fn sharing_request(
    input: &ShareCustomAudienceInput,
) -> Result<(String, String, Params, bool), PublicError> {
    let account = graph_tools::account(&input.ad_account_id)?;
    let audience = graph_tools::id(&input.custom_audience_id, "custom_audience_id")?;
    if input.recipient_ad_account_ids.is_empty() || input.recipient_ad_account_ids.len() > 25 {
        return Err(invalid("Provide 1 through 25 recipient ad account IDs"));
    }
    let recipients = input
        .recipient_ad_account_ids
        .iter()
        .map(|raw| {
            meta_ids::ad_account_digits(raw)
                .map(str::to_owned)
                .ok_or_else(|| invalid("Recipient accounts must be numeric Meta ad account IDs"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if recipients.iter().any(|id| format!("act_{id}") == account)
        || recipients
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != recipients.len()
    {
        return Err(invalid(
            "Recipient accounts must be unique and exclude the owning account",
        ));
    }
    let mut params = vec![(
        "adaccounts".into(),
        graph_tools::json(&json!(recipients), "adaccounts")?,
    )];
    let remove = match &input.action {
        AudienceSharingAction::Add {
            permissions,
            relationship_type,
        } => {
            let permission = match permissions.unwrap_or_default() {
                AudienceSharingPermission::Targeting => "targeting",
                AudienceSharingPermission::TargetingAndInsights => "targeting_and_insights",
            };
            params.push(("permissions".into(), permission.into()));
            params.push(("replace".into(), "false".into()));
            if let Some(types) = relationship_type {
                if types.is_empty() || types.len() > 25 {
                    return Err(invalid(
                        "relationship_type must contain 1 through 25 strings",
                    ));
                }
                let types = types
                    .iter()
                    .map(|value| graph_tools::text(value, "relationship_type", 64))
                    .collect::<Result<Vec<_>, _>>()?;
                params.push((
                    "relationship_type".into(),
                    graph_tools::json(&json!(types), "relationship_type")?,
                ));
            }
            false
        }
        AudienceSharingAction::Remove {
            removal_acknowledgement,
        } => {
            validate_removal_acknowledgement(true, Some(removal_acknowledgement))?;
            true
        }
    };
    Ok((account, audience, params, remove))
}

async fn share_audience(
    graph: &GraphClient,
    input: ShareCustomAudienceInput,
) -> ToolResponse<GraphData> {
    let (account, audience, params, remove) = match sharing_request(&input) {
        Ok(v) => v,
        Err(e) => return ToolResponse::error(e),
    };
    if let Err(e) = verify_meta_node(
        graph,
        &audience,
        MetaNodeKind::CustomAudience,
        Some(&account),
    )
    .await
    {
        return ToolResponse::error(e);
    }
    // Versioned Business SDK 26.0.1 generates this spelling for both writes; no fallback retry.
    let endpoint = format!("{audience}/adaccounts");
    if remove {
        graph_tools::delete(graph, &endpoint, params).await
    } else {
        graph_tools::write(graph, &endpoint, params).await
    }
}

#[tool_router(router = ad_library_mutations_router, vis = "pub(crate)")]
impl MetaAdsServer {
    #[tool(
        name = "manage_ad_label",
        description = "Create, rename or delete an ad-account label. Existing labels are checked against the account; deletion requires removal acknowledgement.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn manage_ad_label(
        &self,
        Parameters(input): Parameters<ManageAdLabelInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        manage_label(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "change_ad_labels",
        description = "Add or remove labels on one verified campaign, ad set, ad or creative. Other labels are preserved; removals require acknowledgement. At most 50 labels per call.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn change_ad_labels(
        &self,
        Parameters(input): Parameters<ChangeAdLabelsInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        change_labels(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "delete_ad_media",
        description = "Remove an image or video through its ad-account library edge. Requires CONFIRM_META_ADS_REMOVALS; check existing creative usage before deletion.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn delete_ad_media(
        &self,
        Parameters(input): Parameters<DeleteAdMediaInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        match media_delete_request(&input) {
            Ok((endpoint, params)) => graph_tools::delete(&self.graph, &endpoint, params).await,
            Err(e) => ToolResponse::error(e),
        }
        .into_mcp_result()
    }

    #[tool(
        name = "share_custom_audience",
        description = "Grant or revoke ad-account access to an owned Custom Audience. Grants preserve other recipients and default to targeting only. Revocation can stop recipient ads and requires removal acknowledgement.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn share_custom_audience(
        &self,
        Parameters(input): Parameters<ShareCustomAudienceInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        share_audience(&self.graph, input).await.into_mcp_result()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MetaConfig, safety::REMOVAL_ACKNOWLEDGEMENT};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn bounds_sharing_and_preserves_unselected_creative_labels() {
        let input:ShareCustomAudienceInput=serde_json::from_value(json!({"ad_account_id":"act_10","custom_audience_id":"20","recipient_ad_account_ids":["act_30","40"],"action":{"kind":"add"}})).unwrap();
        let (_, _, params, remove) = sharing_request(&input).unwrap();
        assert!(!remove);
        assert!(params.contains(&("adaccounts".into(), "[\"30\",\"40\"]".into())));
        assert!(params.contains(&("permissions".into(), "targeting".into())));
        assert!(params.contains(&("replace".into(), "false".into())));
        for recipients in [vec![], vec!["10"], vec!["30", "act_30"], vec!["30/other"]] {
            let mut input:ShareCustomAudienceInput=serde_json::from_value(json!({"ad_account_id":"10","custom_audience_id":"20","recipient_ad_account_ids":["30"],"action":{"kind":"add"}})).unwrap();
            input.recipient_ad_account_ids = recipients.into_iter().map(str::to_owned).collect();
            assert!(sharing_request(&input).is_err());
        }
        assert!(serde_json::from_value::<ShareCustomAudienceInput>(json!({"ad_account_id":"10","custom_audience_id":"20","recipient_ad_account_ids":["30"],"action":{"kind":"add","replace":true}})).is_err());
        let payload = json!({"id":"20","adlabels":[{"id":"50"},{"id":"60"}]});
        let remaining = remaining_creative_labels(&payload, "20", &["50".into()]).unwrap();
        assert_eq!(remaining, vec!["60"]);
        assert_eq!(
            labels_param(&[]).unwrap(),
            vec![("adlabels".into(), "[]".into())]
        );
        for payload in [
            json!({"id":"20","adlabels":{"data":[{"id":"60"}],"paging":{"next":"more"}}}),
            json!({"id":"20"}),
            json!({"id":"20","adlabels":vec![json!({"id":"60"});51]}),
        ] {
            assert!(remaining_creative_labels(&payload, "20", &["50".into()]).is_err());
        }
        assert!(label_ids(&["50".into(), "50".into()]).is_err());
        assert!(label_ids(&vec!["50".into(); 51]).is_err());
        let input = DeleteAdMediaInput {
            ad_account_id: "10".into(),
            media: AdLibraryMedia::Video {
                video_id: "20".into(),
            },
            removal_acknowledgement: REMOVAL_ACKNOWLEDGEMENT.into(),
        };
        assert_eq!(
            media_delete_request(&input).unwrap(),
            (
                "act_10/advideos".into(),
                vec![("video_id".into(), "20".into())]
            )
        );
        let input = DeleteAdMediaInput {
            ad_account_id: "10".into(),
            media: AdLibraryMedia::ImageHash {
                image_hash: "hash_123".into(),
            },
            removal_acknowledgement: "yes".into(),
        };
        assert!(media_delete_request(&input).is_err());
    }

    #[tokio::test]
    async fn verifies_owners_and_uses_exact_safe_mutation_routes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{}", listener.local_addr().unwrap()),
            Some("test-token"),
        ))
        .unwrap();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for reply in [
                json!({"id":"50","name":"Label","account":{"id":"act_99"}}),
                json!({"id":"50","name":"Label","account":{"id":"act_10"}}),
                json!({"success":true}),
                json!({"id":"20","account_id":"99","subtype":"CUSTOM"}),
                json!({"id":"20","account_id":"10","subtype":"CUSTOM"}),
                json!({"success":true}),
                json!({"id":"70","account_id":"10","object_type":"SHARE"}),
                json!({"id":"50","account":{"id":"act_10"}}),
                json!({"id":"70","adlabels":[{"id":"50"},{"id":"60"}]}),
                json!({"success":true}),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0_u8; 1024];
                    let count = socket.read(&mut chunk).await.unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&chunk[..count]);
                    if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8(request).unwrap());
                let body = reply.to_string();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
            }
            requests
        });
        for success in [false, true] {
            let response = manage_label(
                &graph,
                ManageAdLabelInput {
                    ad_account_id: "10".into(),
                    action: AdLabelAction::Rename {
                        label_id: "50".into(),
                        name: "Renamed".into(),
                    },
                },
            )
            .await;
            assert_eq!(matches!(response, ToolResponse::Success { .. }), success);
        }
        for success in [false, true] {
            let response = share_audience(
                &graph,
                ShareCustomAudienceInput {
                    ad_account_id: "10".into(),
                    custom_audience_id: "20".into(),
                    recipient_ad_account_ids: vec!["30".into()],
                    action: AudienceSharingAction::Remove {
                        removal_acknowledgement: REMOVAL_ACKNOWLEDGEMENT.into(),
                    },
                },
            )
            .await;
            assert_eq!(matches!(response, ToolResponse::Success { .. }), success);
        }
        let response = change_labels(
            &graph,
            ChangeAdLabelsInput {
                ad_account_id: "10".into(),
                object_id: "70".into(),
                object_kind: LabelObjectKind::Creative,
                label_ids: vec!["50".into()],
                action: LabelAssignmentAction::Remove {
                    removal_acknowledgement: REMOVAL_ACKNOWLEDGEMENT.into(),
                },
            },
        )
        .await;
        assert!(matches!(response, ToolResponse::Success { .. }));
        let requests = server.await.unwrap();
        assert!(requests[2].starts_with("POST /50 "));
        assert!(requests[2].ends_with("name=Renamed"));
        assert!(requests[5].starts_with("DELETE /20/adaccounts?adaccounts=%5B%2230%22%5D "));
        assert!(requests[9].starts_with("POST /70 "));
        assert!(requests[9].ends_with("adlabels=%5B%7B%22id%22%3A%2260%22%7D%5D"));
        assert_eq!(
            requests
                .iter()
                .filter(|request| !request.starts_with("GET "))
                .count(),
            3
        );
    }
}
