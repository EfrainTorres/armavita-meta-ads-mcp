// Copyright (C) 2026 ArmaVita LLC
// SPDX-License-Identifier: AGPL-3.0-only

use rmcp::{
    Json,
    handler::server::wrapper::Parameters,
    schemars::{self, JsonSchema},
    tool, tool_router,
};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    graph_tools::{self, GraphData, Params, ReadOptions},
    meta_ids,
    safety::validate_removal_acknowledgement,
    server::MetaAdsServer,
};

const FB_COMMENTS: &str = "id,message,created_time,is_hidden,like_count,comment_count";
const IG_COMMENTS: &str = "id,text,timestamp,hidden,like_count";
const THREADS_REPLIES: &str = "id,caption,timestamp,hide_status,like_count,reply_count";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadAdAssetsInput {
    pub target: AdAssetTarget,
    pub options: Option<ReadOptions>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AdAssetTarget {
    PageInstagramAccounts {
        page_id: String,
    },
    AdAccountInstagramAccounts {
        ad_account_id: String,
    },
    PageBackedInstagramAccounts {
        page_id: String,
    },
    /// Existing published Page posts for reuse in an ad creative; no post publishing.
    PagePosts {
        page_id: String,
    },
    /// Existing Page photo IDs for advertising and Instant Experience elements.
    PagePhotos {
        page_id: String,
    },
    /// Existing Page video IDs for advertising and Instant Experience elements.
    PageVideos {
        page_id: String,
    },
    InstagramMedia {
        instagram_user_id: String,
    },
    /// Authorized partner content eligible for partnership ads.
    PartnershipMedia {
        instagram_user_id: String,
    },
    InstagramMediaDetail {
        media_id: String,
    },
    /// Inspect ad media returned by Meta for a Threads ad; organic publishing is separate.
    ThreadsAdMedia {
        media_id: String,
    },
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreatePageBackedInstagramAccountInput {
    pub page_id: String,
}

#[derive(Clone, Deserialize, JsonSchema)]
#[serde(tag = "platform", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AdCommentTarget {
    /// Resolve the current creative's effective Page story before reading or writing comments.
    Facebook { ad_id: String, page_id: String },
    /// Resolve the current creative's effective Instagram media before reading or writing comments.
    Instagram { ad_id: String },
    /// Marketing API ad-media ID. Backed-account, catalog and boosted ads are unsupported by Meta.
    Threads { media_id: String },
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadAdCommentsInput {
    pub target: AdCommentTarget,
    /// Optional verified parent comment for Facebook/Instagram replies. Threads supports direct ad replies only.
    pub parent_comment_id: Option<String>,
    pub options: Option<ReadOptions>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManageAdCommentInput {
    pub target: AdCommentTarget,
    pub comment_id: String,
    pub action: AdCommentAction,
    /// Threads only: cursor from read_ad_comments containing this reply, for bounded membership verification.
    pub comment_page_cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AdCommentAction {
    Reply {
        #[schemars(length(min = 1, max = 2000))]
        text: String,
    },
    SetHidden {
        hidden: bool,
    },
    /// Available for Facebook/Instagram comments; Meta exposes no Threads ad-reply deletion API.
    Delete {
        #[schemars(
            length(min = 25, max = 25),
            regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
        )]
        removal_acknowledgement: String,
    },
}

fn invalid(message: &str) -> PublicError {
    PublicError::invalid_input(
        message,
        "Use the exact ad, media and comment IDs returned by Meta",
    )
}

fn facebook_id(raw: &str) -> Result<String, PublicError> {
    let parts = raw.split('_').collect::<Vec<_>>();
    if parts.is_empty()
        || parts.len() > 2
        || parts
            .iter()
            .any(|part| meta_ids::numeric(part) != Some(*part))
    {
        return Err(invalid(
            "Facebook object IDs must be numeric or page_id_post_id",
        ));
    }
    Ok(raw.to_owned())
}

fn comment_id(target: &AdCommentTarget, raw: &str) -> Result<String, PublicError> {
    if matches!(target, AdCommentTarget::Facebook { .. }) {
        facebook_id(raw)
    } else {
        graph_tools::id(raw, "comment_id")
    }
}

fn assets_request(input: &ReadAdAssetsInput) -> Result<(String, Params), PublicError> {
    let (endpoint, fields, single) = match &input.target {
        AdAssetTarget::PageInstagramAccounts { page_id } => (
            format!(
                "{}/instagram_accounts",
                graph_tools::id(page_id, "page_id")?
            ),
            "id,username",
            false,
        ),
        AdAssetTarget::AdAccountInstagramAccounts { ad_account_id } => (
            format!(
                "{}/instagram_accounts",
                graph_tools::account(ad_account_id)?
            ),
            "id,username",
            false,
        ),
        AdAssetTarget::PageBackedInstagramAccounts { page_id } => (
            format!(
                "{}/page_backed_instagram_accounts",
                graph_tools::id(page_id, "page_id")?
            ),
            "id,username",
            false,
        ),
        AdAssetTarget::PagePosts { page_id } => (
            format!("{}/posts", graph_tools::id(page_id, "page_id")?),
            "id,message,created_time,permalink_url,is_published",
            false,
        ),
        AdAssetTarget::PagePhotos { page_id } => (
            format!("{}/photos", graph_tools::id(page_id, "page_id")?),
            "id,name,created_time,link",
            false,
        ),
        AdAssetTarget::PageVideos { page_id } => (
            format!("{}/videos", graph_tools::id(page_id, "page_id")?),
            "id,title,description,created_time,length",
            false,
        ),
        AdAssetTarget::InstagramMedia { instagram_user_id } => (
            format!(
                "{}/media",
                graph_tools::id(instagram_user_id, "instagram_user_id")?
            ),
            "id,caption,media_type,media_product_type,permalink,timestamp",
            false,
        ),
        AdAssetTarget::PartnershipMedia { instagram_user_id } => (
            format!(
                "{}/branded_content_advertisable_medias",
                graph_tools::id(instagram_user_id, "instagram_user_id")?
            ),
            "id,owner_id,permalink,has_permission_for_partnership_ad,is_creator_allowlisted,eligibility_errors",
            false,
        ),
        AdAssetTarget::InstagramMediaDetail { media_id } => (
            graph_tools::id(media_id, "media_id")?,
            "id,caption,media_type,media_product_type,permalink,timestamp,owner,is_comment_enabled",
            true,
        ),
        AdAssetTarget::ThreadsAdMedia { media_id } => (
            graph_tools::id(media_id, "media_id")?,
            "id,caption,media_type,timestamp,username,reply_count,like_count",
            true,
        ),
    };
    let defaults = ReadOptions::default();
    let options = input.options.as_ref().unwrap_or(&defaults);
    if single && (options.page_size.is_some() || options.page_cursor.is_some()) {
        return Err(invalid("Pagination applies only to list operations"));
    }
    let mut params = graph_tools::read_params(options, fields)?;
    if single {
        params.retain(|(key, _)| key != "limit");
    }
    if matches!(
        input.target,
        AdAssetTarget::PagePhotos { .. } | AdAssetTarget::PageVideos { .. }
    ) {
        params.push(("type".into(), "uploaded".into()));
    }
    Ok((endpoint, params))
}

async fn resolve_media(
    graph: &GraphClient,
    target: &AdCommentTarget,
) -> Result<String, PublicError> {
    let (ad_id, key) = match target {
        AdCommentTarget::Threads { media_id } => return graph_tools::id(media_id, "media_id"),
        AdCommentTarget::Facebook { ad_id, page_id } => {
            graph_tools::id(page_id, "page_id")?;
            (ad_id, "effective_object_story_id")
        }
        AdCommentTarget::Instagram { ad_id } => (ad_id, "effective_instagram_media_id"),
    };
    let ad_id = graph_tools::id(ad_id, "ad_id")?;
    let payload = graph
        .get_json(
            &ad_id,
            &[("fields".into(), format!("id,creative{{id,{key}}}"))],
        )
        .await?;
    if payload
        .get("id")
        .and_then(meta_ids::numeric_value)
        .as_deref()
        != Some(ad_id.as_str())
    {
        return Err(invalid("Meta did not confirm the requested ad identity"));
    }
    let value = payload
        .get("creative")
        .and_then(|creative| creative.get(key));
    let media_id = value
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| value.and_then(meta_ids::numeric_value))
        .ok_or_else(|| invalid("The ad creative has no effective media for this platform"))?;
    match target {
        AdCommentTarget::Facebook { page_id, .. } => {
            facebook_id(&media_id)?;
            if media_id
                .split_once('_')
                .is_none_or(|(page, _)| page != page_id.trim())
            {
                return Err(invalid(
                    "The ad's current story belongs to a different Page",
                ));
            }
        }
        _ => {
            graph_tools::id(&media_id, "media_id")?;
        }
    }
    Ok(media_id)
}

async fn verify_comment(
    graph: &GraphClient,
    target: &AdCommentTarget,
    media_id: &str,
    comment_id: &str,
    cursor: Option<&str>,
) -> Result<(), PublicError> {
    match target {
        AdCommentTarget::Facebook { .. } => {
            facebook_id(comment_id)?;
            let payload = graph
                .get_json(comment_id, &[("fields".into(), "id,object".into())])
                .await?;
            if payload.get("id").and_then(Value::as_str) != Some(comment_id)
                || payload.pointer("/object/id").and_then(Value::as_str) != Some(media_id)
            {
                return Err(invalid(
                    "This comment does not belong to the ad's current Page story",
                ));
            }
        }
        AdCommentTarget::Instagram { .. } => {
            graph_tools::id(comment_id, "comment_id")?;
            let payload = graph
                .get_json(comment_id, &[("fields".into(), "id,media".into())])
                .await?;
            if payload
                .get("id")
                .and_then(meta_ids::numeric_value)
                .as_deref()
                != Some(comment_id)
                || payload
                    .pointer("/media/id")
                    .and_then(meta_ids::numeric_value)
                    .as_deref()
                    != Some(media_id)
            {
                return Err(invalid(
                    "This comment does not belong to the ad's current Instagram media",
                ));
            }
        }
        AdCommentTarget::Threads { .. } => {
            graph_tools::id(comment_id, "comment_id")?;
            let options = ReadOptions {
                fields: Some(vec!["id".into()]),
                page_size: Some(100),
                page_cursor: cursor.map(str::to_owned),
            };
            let params = graph_tools::read_params(&options, "id")?;
            let payload = graph
                .get_json(&format!("{media_id}/replies"), &params)
                .await?;
            let replies = payload
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    PublicError::invalid_upstream("Meta returned an invalid Threads reply page")
                })?;
            if replies.len() > 100
                || !replies.iter().any(|reply| {
                    reply.get("id").and_then(meta_ids::numeric_value).as_deref() == Some(comment_id)
                })
            {
                return Err(invalid(
                    "The reply is not on the selected ad-media reply page; use its next_cursor if needed",
                ));
            }
        }
    }
    Ok(())
}

async fn comments_read(
    graph: &GraphClient,
    mut input: ReadAdCommentsInput,
) -> ToolResponse<GraphData> {
    let defaults = ReadOptions::default();
    let options = input.options.as_ref().unwrap_or(&defaults);
    let fields = match input.target {
        AdCommentTarget::Facebook { .. } => FB_COMMENTS,
        AdCommentTarget::Instagram { .. } => IG_COMMENTS,
        AdCommentTarget::Threads { .. } => THREADS_REPLIES,
    };
    let params = match graph_tools::read_params(options, fields) {
        Ok(value) => value,
        Err(error) => return ToolResponse::error(error),
    };
    if input.parent_comment_id.is_some() && matches!(input.target, AdCommentTarget::Threads { .. })
    {
        return ToolResponse::error(invalid(
            "Threads ad replies support only direct replies to the ad media",
        ));
    }
    if let Some(parent) = &input.parent_comment_id {
        match comment_id(&input.target, parent) {
            Ok(id) => input.parent_comment_id = Some(id),
            Err(error) => return ToolResponse::error(error),
        }
    }
    let media_id = match resolve_media(graph, &input.target).await {
        Ok(value) => value,
        Err(error) => return ToolResponse::error(error),
    };
    let scoped_graph = match &input.target {
        AdCommentTarget::Facebook { page_id, .. } => match graph.for_page(page_id.trim()).await {
            Ok(graph) => graph,
            Err(error) => return ToolResponse::error(error),
        },
        _ => graph.clone(),
    };
    let graph = &scoped_graph;
    let has_parent = input.parent_comment_id.is_some();
    let object_id = if let Some(comment_id) = input.parent_comment_id {
        if let Err(error) = verify_comment(graph, &input.target, &media_id, &comment_id, None).await
        {
            return ToolResponse::error(error);
        }
        comment_id
    } else {
        media_id
    };
    let edge = match input.target {
        AdCommentTarget::Facebook { .. } => "comments",
        AdCommentTarget::Instagram { .. } if has_parent => "replies",
        AdCommentTarget::Instagram { .. } => "comments",
        AdCommentTarget::Threads { .. } => "replies",
    };
    graph_tools::read(graph, &format!("{object_id}/{edge}"), params).await
}

fn mutation_request(
    target: &AdCommentTarget,
    comment_id: &str,
    action: &AdCommentAction,
) -> Result<(String, Params, bool), PublicError> {
    self::comment_id(target, comment_id)?;
    match action {
        AdCommentAction::Reply { text } => {
            let text = graph_tools::text(text, "text", 2000)?;
            let (edge, key) = match target {
                AdCommentTarget::Facebook { .. } => ("comments", "message"),
                AdCommentTarget::Instagram { .. } => ("replies", "message"),
                AdCommentTarget::Threads { .. } => ("add_reply", "text"),
            };
            Ok((
                format!("{comment_id}/{edge}"),
                vec![(key.into(), text)],
                false,
            ))
        }
        AdCommentAction::SetHidden { hidden } => {
            let (endpoint, key) = match target {
                AdCommentTarget::Facebook { .. } => (comment_id.to_owned(), "is_hidden"),
                AdCommentTarget::Instagram { .. } => (comment_id.to_owned(), "hide"),
                AdCommentTarget::Threads { .. } => (format!("{comment_id}/manage_reply"), "hide"),
            };
            Ok((endpoint, vec![(key.into(), hidden.to_string())], false))
        }
        AdCommentAction::Delete {
            removal_acknowledgement,
        } => {
            if matches!(target, AdCommentTarget::Threads { .. }) {
                return Err(invalid(
                    "Meta does not expose deletion of Threads ad replies; hide the reply instead",
                ));
            }
            validate_removal_acknowledgement(true, Some(removal_acknowledgement))?;
            Ok((comment_id.to_owned(), Vec::new(), true))
        }
    }
}

async fn comment_write(
    graph: &GraphClient,
    mut input: ManageAdCommentInput,
) -> ToolResponse<GraphData> {
    input.comment_id = match comment_id(&input.target, &input.comment_id) {
        Ok(id) => id,
        Err(error) => return ToolResponse::error(error),
    };
    let (endpoint, params, delete) =
        match mutation_request(&input.target, &input.comment_id, &input.action) {
            Ok(value) => value,
            Err(error) => return ToolResponse::error(error),
        };
    if input.comment_page_cursor.is_some()
        && !matches!(input.target, AdCommentTarget::Threads { .. })
    {
        return ToolResponse::error(invalid(
            "comment_page_cursor applies only to Threads reply verification",
        ));
    }
    if let Some(cursor) = &input.comment_page_cursor {
        let options = ReadOptions {
            page_cursor: Some(cursor.clone()),
            ..ReadOptions::default()
        };
        if let Err(error) = graph_tools::read_params(&options, "id") {
            return ToolResponse::error(error);
        }
    }
    let media_id = match resolve_media(graph, &input.target).await {
        Ok(value) => value,
        Err(error) => return ToolResponse::error(error),
    };
    let scoped_graph = match &input.target {
        AdCommentTarget::Facebook { page_id, .. } => match graph.for_page(page_id.trim()).await {
            Ok(graph) => graph,
            Err(error) => return ToolResponse::error(error),
        },
        _ => graph.clone(),
    };
    let graph = &scoped_graph;
    if let Err(error) = verify_comment(
        graph,
        &input.target,
        &media_id,
        &input.comment_id,
        input.comment_page_cursor.as_deref(),
    )
    .await
    {
        return ToolResponse::error(error);
    }
    if delete {
        graph_tools::delete(graph, &endpoint, params).await
    } else {
        graph_tools::write(graph, &endpoint, params).await
    }
}

#[tool_router(router = ad_engagement_router, vis = "pub(crate)")]
impl MetaAdsServer {
    #[tool(
        name = "read_ad_assets",
        description = "Discover eligible Instagram identities, existing Page posts, Instagram/partner media, and Threads ad media for advertising. One bounded page; request extra fields only when needed.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn read_ad_assets(
        &self,
        Parameters(input): Parameters<ReadAdAssetsInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        let response = match assets_request(&input) {
            Ok((endpoint, params)) => {
                let graph = match &input.target {
                    AdAssetTarget::PageInstagramAccounts { page_id }
                    | AdAssetTarget::PageBackedInstagramAccounts { page_id }
                    | AdAssetTarget::PagePosts { page_id }
                    | AdAssetTarget::PagePhotos { page_id }
                    | AdAssetTarget::PageVideos { page_id } => {
                        match self.graph.for_page(page_id.trim()).await {
                            Ok(graph) => graph,
                            Err(error) => {
                                return ToolResponse::<GraphData>::error(error).into_mcp_result();
                            }
                        }
                    }
                    _ => self.graph.clone(),
                };
                graph_tools::read(&graph, &endpoint, params).await
            }
            Err(error) => ToolResponse::error(error),
        };
        response.into_mcp_result()
    }
    #[tool(
        name = "create_page_backed_instagram_account",
        description = "Create the Page-backed Instagram advertising identity for a Page without an Instagram account. Read its existing identities first; requires Page advertising access.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_page_backed_instagram_account(
        &self,
        Parameters(input): Parameters<CreatePageBackedInstagramAccountInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        let response = match graph_tools::id(&input.page_id, "page_id") {
            Ok(id) => match self.graph.for_page(&id).await {
                Ok(graph) => {
                    graph_tools::write(
                        &graph,
                        &format!("{id}/page_backed_instagram_accounts"),
                        Vec::new(),
                    )
                    .await
                }
                Err(error) => ToolResponse::error(error),
            },
            Err(error) => ToolResponse::error(error),
        };
        response.into_mcp_result()
    }
    #[tool(
        name = "read_ad_comments",
        description = "Read comments for a Facebook/Instagram ad's current creative, or direct replies for Threads ad media. FB/IG can read a verified comment's replies; Threads nested replies are unavailable.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn read_ad_comments(
        &self,
        Parameters(input): Parameters<ReadAdCommentsInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        comments_read(&self.graph, input).await.into_mcp_result()
    }
    #[tool(
        name = "manage_ad_comment",
        description = "Reply to or hide/unhide a verified Facebook, Instagram or Threads ad comment. FB/IG deletion requires CONFIRM_META_ADS_REMOVALS. Threads supports text replies and hiding only; writes are one-shot.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn manage_ad_comment(
        &self,
        Parameters(input): Parameters<ManageAdCommentInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        comment_write(&self.graph, input).await.into_mcp_result()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MetaConfig, safety::REMOVAL_ACKNOWLEDGEMENT};
    use serde_json::json;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn platform_routes_and_destructive_boundaries() {
        let facebook = AdCommentTarget::Facebook {
            ad_id: "10".into(),
            page_id: "20".into(),
        };
        let instagram = AdCommentTarget::Instagram { ad_id: "10".into() };
        let threads = AdCommentTarget::Threads {
            media_id: "30".into(),
        };
        for (target, reply_path, reply_key, hide_path, hide_key) in [
            (&facebook, "40/comments", "message", "40", "is_hidden"),
            (&instagram, "40/replies", "message", "40", "hide"),
            (&threads, "40/add_reply", "text", "40/manage_reply", "hide"),
        ] {
            let (path, params, delete) = mutation_request(
                target,
                "40",
                &AdCommentAction::Reply {
                    text: "Thank you".into(),
                },
            )
            .unwrap();
            assert_eq!(path, reply_path);
            assert_eq!(params, vec![(reply_key.into(), "Thank you".into())]);
            assert!(!delete);
            let (path, params, _) =
                mutation_request(target, "40", &AdCommentAction::SetHidden { hidden: true })
                    .unwrap();
            assert_eq!(path, hide_path);
            assert_eq!(params, vec![(hide_key.into(), "true".into())]);
            assert!(
                mutation_request(
                    target,
                    "40/other",
                    &AdCommentAction::SetHidden { hidden: false }
                )
                .is_err()
            );
        }
        let delete = AdCommentAction::Delete {
            removal_acknowledgement: REMOVAL_ACKNOWLEDGEMENT.into(),
        };
        assert!(mutation_request(&threads, "40", &delete).is_err());
        assert!(mutation_request(&instagram, "40", &delete).unwrap().2);
        assert!(
            mutation_request(
                &facebook,
                "40",
                &AdCommentAction::Delete {
                    removal_acknowledgement: "yes".into()
                }
            )
            .is_err()
        );
        assert!(serde_json::from_value::<ManageAdCommentInput>(json!({"target":{"platform":"threads","media_id":"30","access_token":"secret"},"comment_id":"40","action":{"kind":"set_hidden","hidden":true}})).is_err());
        let assets: ReadAdAssetsInput = serde_json::from_value(
            json!({"target":{"kind":"ad_account_instagram_accounts","ad_account_id":"10"}}),
        )
        .unwrap();
        let (path, params) = assets_request(&assets).unwrap();
        assert_eq!(path, "act_10/instagram_accounts");
        assert!(params.contains(&("limit".into(), "25".into())));
        let partner: ReadAdAssetsInput = serde_json::from_value(
            json!({"target":{"kind":"partnership_media","instagram_user_id":"30"}}),
        )
        .unwrap();
        let (_, partner_params) = assets_request(&partner).unwrap();
        let fields = &partner_params
            .iter()
            .find(|(key, _)| key == "fields")
            .unwrap()
            .1;
        assert!(fields.contains("has_permission_for_partnership_ad"));
        assert!(!fields.contains("caption"));
        for (kind, edge) in [("page_photos", "photos"), ("page_videos", "videos")] {
            let assets: ReadAdAssetsInput =
                serde_json::from_value(json!({"target":{"kind":kind,"page_id":"20"}})).unwrap();
            let (path, params) = assets_request(&assets).unwrap();
            assert_eq!(path, format!("20/{edge}"));
            assert!(params.contains(&("type".into(), "uploaded".into())));
        }
        let assets: ReadAdAssetsInput = serde_json::from_value(json!({"target":{"kind":"instagram_media_detail","media_id":"30"},"options":{"page_size":25}})).unwrap();
        assert!(assets_request(&assets).is_err());
    }

    #[tokio::test]
    async fn verifies_ad_media_and_comment_membership_before_one_shot_writes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{}", listener.local_addr().unwrap()),
            Some("test-token"),
        ))
        .unwrap();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for reply in [
                json!({"id":"10","creative":{"id":"11","effective_instagram_media_id":"30"}}),
                json!({"id":"40","media":{"id":"99"}}),
                json!({"id":"10","creative":{"id":"11","effective_instagram_media_id":"30"}}),
                json!({"id":"40","media":{"id":"30"}}),
                json!({"success":true}),
                json!({"data":[{"id":"99"}]}),
                json!({"data":[{"id":"40"}]}),
                json!({"id":"41"}),
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
            let response = comment_write(
                &graph,
                ManageAdCommentInput {
                    target: AdCommentTarget::Instagram { ad_id: "10".into() },
                    comment_id: "40".into(),
                    action: AdCommentAction::SetHidden { hidden: true },
                    comment_page_cursor: None,
                },
            )
            .await;
            assert_eq!(matches!(response, ToolResponse::Success { .. }), success);
        }
        for success in [false, true] {
            let response = comment_write(
                &graph,
                ManageAdCommentInput {
                    target: AdCommentTarget::Threads {
                        media_id: "30".into(),
                    },
                    comment_id: "40".into(),
                    action: AdCommentAction::Reply {
                        text: "Thank you".into(),
                    },
                    comment_page_cursor: Some("next-page".into()),
                },
            )
            .await;
            assert_eq!(matches!(response, ToolResponse::Success { .. }), success);
        }
        let requests = server.await.unwrap();
        assert!(requests[0].starts_with("GET /10?"));
        assert!(requests[1].starts_with("GET /40?"));
        assert!(requests[4].starts_with("POST /40 "));
        assert!(requests[4].ends_with("hide=true"));
        assert!(requests[5].starts_with("GET /30/replies?"));
        assert!(requests[5].contains("after=next-page"));
        assert!(requests[7].starts_with("POST /40/add_reply "));
        assert!(requests[7].ends_with("text=Thank+you"));
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.starts_with("POST "))
                .count(),
            2
        );
    }
}
