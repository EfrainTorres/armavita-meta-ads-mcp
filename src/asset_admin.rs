// Copyright (C) 2025 ArmaVita LLC
// SPDX-License-Identifier: AGPL-3.0-only

//! Business, account, measurement-source and publisher-blocklist contracts verified against SDK 26.0.1.
//! The inline 26.0.0 line references identify unchanged request contracts.
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
    node_identity::verify_object,
    safety::validate_removal_acknowledgement,
    server::MetaAdsServer,
};

const BUSINESS_FIELDS: &str =
    "id,name,verification_status,primary_page,timezone_id,two_factor_type";
const ACCOUNT_FIELDS: &str =
    "id,name,account_status,currency,timezone_name,spend_cap,amount_spent,balance";
const PIXEL_FIELDS: &str = "id,name,creation_time,last_fired_time,owner_business,owner_ad_account,enable_automatic_matching,first_party_cookie_status,data_use_setting";
const DATASET_FIELDS: &str = "id,name,description,is_crm,is_unavailable,owner_business,owner_ad_account,last_fired_time,last_upload_time";
const USER_FIELDS: &str = "id,name,business,role,tasks";
const SYSTEM_USER_FIELDS: &str = "id,name,role,created_time";
const BLOCKLIST_FIELDS: &str =
    "id,name,owner_ad_account_id,business_owner_id,is_auto_blocking_on,last_update_time";

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BusinessAssetKind {
    Business,
    AdAccount,
    Pixel,
    BusinessUser,
    SystemUser,
    PublisherBlockList,
}

impl BusinessAssetKind {
    fn defaults(self) -> &'static str {
        match self {
            Self::Business => BUSINESS_FIELDS,
            Self::AdAccount => ACCOUNT_FIELDS,
            Self::Pixel => PIXEL_FIELDS,
            Self::BusinessUser => USER_FIELDS,
            Self::SystemUser => SYSTEM_USER_FIELDS,
            Self::PublisherBlockList => BLOCKLIST_FIELDS,
        }
    }
    fn proof(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::Business => (
                "id,verification_status,two_factor_type",
                &["verification_status", "two_factor_type"],
            ),
            Self::AdAccount => (
                "id,account_id,account_status",
                &["account_id", "account_status"],
            ),
            Self::Pixel => (
                "id,first_party_cookie_status",
                &["first_party_cookie_status"],
            ),
            Self::BusinessUser => ("id,business,role", &["business", "role"]),
            Self::SystemUser => ("id,role,created_time", &["role", "created_time"]),
            Self::PublisherBlockList => (
                "id,web_publishers,app_publishers",
                &["web_publishers", "app_publishers"],
            ),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadBusinessAssetInput {
    pub kind: BusinessAssetKind,
    pub object_id: String,
    #[serde(default)]
    pub options: ReadOptions,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BusinessAssetCollection {
    /// User ID, or me.
    Businesses,
    /// Business ID.
    OwnedAdAccounts,
    ClientAdAccounts,
    PendingOwnedAdAccounts,
    PendingClientAdAccounts,
    BusinessUsers,
    SystemUsers,
    BusinessAgencies,
    BusinessPixels,
    OwnedPixels,
    ClientPixels,
    Datasets,
    /// Ad account ID, with or without act_.
    AccountPixels,
    AccountActivities,
    AccountUsers,
    AccountAssignedUsers,
    AccountAgencies,
    PublisherBlockLists,
    /// Pixel ID; assigned users/accounts/shared accounts require query.business.
    PixelAccounts,
    PixelAssignedUsers,
    PixelAgencies,
    PixelSharedAccounts,
    PixelSharedAgencies,
    PixelStats,
    PixelCatalogChecks,
    PixelOfflineUploads,
    PixelOpenBridgeConfigurations,
    /// Business-user or system-user ID.
    UserAdAccounts,
    UserPages,
    UserCatalogs,
    /// Publisher blocklist ID.
    BlockedPublishers,
}

impl BusinessAssetCollection {
    fn account_parent(self) -> bool {
        matches!(
            self,
            Self::AccountPixels
                | Self::AccountActivities
                | Self::AccountUsers
                | Self::AccountAssignedUsers
                | Self::AccountAgencies
                | Self::PublisherBlockLists
        )
    }
    fn contract(self) -> (&'static str, &'static str, &'static [&'static str]) {
        match self {
            Self::Businesses => ("businesses", BUSINESS_FIELDS, &[]),
            Self::OwnedAdAccounts => (
                "owned_ad_accounts",
                ACCOUNT_FIELDS,
                &["include_shared_ad_accounts", "search_query"],
            ),
            Self::ClientAdAccounts => ("client_ad_accounts", ACCOUNT_FIELDS, &["search_query"]),
            Self::PendingOwnedAdAccounts => ("pending_owned_ad_accounts", ACCOUNT_FIELDS, &[]),
            Self::PendingClientAdAccounts => ("pending_client_ad_accounts", ACCOUNT_FIELDS, &[]),
            Self::BusinessUsers => ("business_users", USER_FIELDS, &[]),
            Self::SystemUsers => ("system_users", SYSTEM_USER_FIELDS, &[]),
            Self::BusinessAgencies => ("agencies", BUSINESS_FIELDS, &[]),
            Self::BusinessPixels => (
                "adspixels",
                PIXEL_FIELDS,
                &["id_filter", "name_filter", "sort_by"],
            ),
            Self::OwnedPixels => ("owned_pixels", PIXEL_FIELDS, &[]),
            Self::ClientPixels => ("client_pixels", PIXEL_FIELDS, &[]),
            Self::Datasets => (
                "ads_dataset",
                DATASET_FIELDS,
                &["id_filter", "name_filter", "sort_by"],
            ),
            Self::AccountPixels => ("adspixels", PIXEL_FIELDS, &["sort_by"]),
            Self::AccountActivities => (
                "activities",
                "event_time,event_type,actor_id,actor_name,object_id,object_name,object_type",
                &[
                    "add_children",
                    "business_id",
                    "category",
                    "data_source",
                    "extra_oids",
                    "oid",
                    "since",
                    "uid",
                    "until",
                ],
            ),
            Self::AccountUsers => ("users", "id,name,tasks", &[]),
            Self::AccountAssignedUsers | Self::PixelAssignedUsers => (
                "assigned_users",
                "id,name,user_type,business",
                &["business"],
            ),
            Self::AccountAgencies | Self::PixelAgencies => ("agencies", "id,name", &[]),
            Self::PublisherBlockLists => ("publisher_block_lists", BLOCKLIST_FIELDS, &[]),
            Self::PixelAccounts => ("adaccounts", ACCOUNT_FIELDS, &["business"]),
            Self::PixelSharedAccounts => ("shared_accounts", ACCOUNT_FIELDS, &["business"]),
            Self::PixelSharedAgencies => ("shared_agencies", "id,name", &[]),
            Self::PixelStats => (
                "stats",
                "aggregation,data,start_time",
                &[
                    "agent",
                    "aggregation",
                    "end_time",
                    "event",
                    "event_source",
                    "start_time",
                ],
            ),
            Self::PixelCatalogChecks => (
                "da_checks",
                "key,result,title,description,user_message",
                &["checks", "connection_method"],
            ),
            Self::PixelOfflineUploads => (
                "offline_event_uploads",
                "id,creation_time,last_upload_time,upload_tag,valid_entries,matched_entries,duplicate_entries,match_rate_approx",
                &["end_time", "order", "sort_by", "start_time", "upload_tag"],
            ),
            Self::PixelOpenBridgeConfigurations => (
                "openbridge_configurations",
                "id,active,pixel_id,capi_publishing_state,partner_name",
                &[],
            ),
            Self::UserAdAccounts => ("assigned_ad_accounts", ACCOUNT_FIELDS, &[]),
            Self::UserPages => ("assigned_pages", "id,name", &["pages"]),
            Self::UserCatalogs => (
                "assigned_product_catalogs",
                "id,name,vertical,product_count",
                &[],
            ),
            Self::BlockedPublishers => (
                "paged_web_publishers",
                "id,domain_url,publisher_name",
                &["draft_id"],
            ),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListBusinessAssetsInput {
    pub collection: BusinessAssetCollection,
    /// Parent described by collection. Only businesses accepts me.
    pub parent_id: String,
    #[serde(default)]
    pub options: ReadOptions,
    /// Official filters for this collection, e.g. business, id_filter, since/until, aggregation.
    #[serde(default)]
    pub query: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CreateBusinessAssetKind {
    BusinessPixel,
    AccountPixel,
    Dataset,
    BusinessUser,
    SystemUser,
    AdAccount,
    PublisherBlockList,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateBusinessAssetInput {
    pub kind: CreateBusinessAssetKind,
    /// Business ID, except account_pixel and publisher_block_list use ad account ID.
    pub parent_id: String,
    /// Official v26 creation fields. Most require name; business users require email/role and may send an invitation. No token generation.
    pub fields: Map<String, Value>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateBusinessAssetInput {
    /// Business, ad_account, pixel, business_user, or publisher_block_list; system-user node updates are unavailable.
    pub kind: BusinessAssetKind,
    pub object_id: String,
    /// Official v26 writable fields. Pixel: matching/cookie/data-use settings; account: spend_cap, timezone_id, name; blocklist: structured spec.
    pub fields: Map<String, Value>,
    /// Required for replacing blocklist specs/user access, removing CAPI business access, or clearing/resetting spending limits.
    #[schemars(
        length(min = 25, max = 25),
        regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
    )]
    pub removal_acknowledgement: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AssetAccessAction {
    AssignAccountUser,
    RemoveAccountUser,
    AssignAccountAgency,
    RemoveAccountAgency,
    AssignPixelUser,
    AssignPixelAgency,
    RemovePixelAgency,
    SharePixelAccount,
    UnsharePixelAccount,
    ClaimBusinessAdAccount,
    RemoveBusinessAdAccount,
    RemoveBusinessAgency,
    AppendBlockedPublishers,
    ConfigurePixelAppLinks,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManageBusinessAssetAccessInput {
    pub action: AssetAccessAction,
    /// Account, pixel, business or blocklist ID as selected by action.
    pub parent_id: String,
    /// User actions: user/tasks. Agency actions: business/permitted_tasks. Pixel sharing: account_id/business. Claim/remove account: adaccount_id. Blocklist append: publisher_urls array. Pixel app links: applink_autosetup boolean.
    pub fields: Map<String, Value>,
    #[schemars(
        length(min = 25, max = 25),
        regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
    )]
    pub removal_acknowledgement: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DeletableBusinessAsset {
    BusinessUser,
    PublisherBlockList,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeleteBusinessAssetInput {
    pub kind: DeletableBusinessAsset,
    pub object_id: String,
    #[schemars(
        length(min = 25, max = 25),
        regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
    )]
    pub removal_acknowledgement: String,
}

#[tool_router(router=asset_admin_router,vis="pub(crate)")]
impl MetaAdsServer {
    #[tool(
        name = "read_business_asset",
        description = "Read business, account, pixel settings, user, or publisher blocklist details. Choose fields for a bounded result; dataset details use list_business_assets with datasets/id_filter.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn read_business_asset(
        &self,
        Parameters(input): Parameters<ReadBusinessAssetInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        let request: Result<(String, Params), PublicError> = (|| {
            Ok((
                asset_id(input.kind, &input.object_id)?,
                graph_tools::read_params(&input.options, input.kind.defaults())?,
            ))
        })();
        match request {
            Ok((path, params)) => graph_tools::read(&self.graph, &path, params).await,
            Err(error) => ToolResponse::error(error),
        }
        .into_mcp_result()
    }
    #[tool(
        name = "list_business_assets",
        description = "Discover businesses, account/user access, pixels/datasets, activity logs, pixel event statistics and diagnostics, and publisher blocklists. Collection defines the parent and accepted filters.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_business_assets(
        &self,
        Parameters(input): Parameters<ListBusinessAssetsInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        match build_list(input) {
            Ok((path, params)) => graph_tools::read(&self.graph, &path, params).await,
            Err(error) => ToolResponse::error(error),
        }
        .into_mcp_result()
    }
    #[tool(
        name = "create_business_asset",
        description = "Create a pixel, dataset, user, ad account or publisher blocklist. Business-user creation may invite the supplied email. Repeated calls may create duplicates; credentials are never tool inputs.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_business_asset(
        &self,
        Parameters(input): Parameters<CreateBusinessAssetInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        match build_create(input) {
            Ok((path, params)) => graph_tools::write(&self.graph, &path, params).await,
            Err(error) => ToolResponse::error(error),
        }
        .into_mcp_result()
    }
    #[tool(
        name = "update_business_asset",
        description = "Update verified business/account settings, pixel matching/cookie settings, user access or blocklist spec. Access reductions and spending-limit resets require CONFIRM_META_ADS_REMOVALS.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn update_business_asset(
        &self,
        Parameters(input): Parameters<UpdateBusinessAssetInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        update_asset(&self.graph, input).await.into_mcp_result()
    }
    #[tool(
        name = "manage_business_asset_access",
        description = "Assign users/agencies, share pixels with ad accounts, claim/remove business accounts, or append blocked publishers. Removal actions require CONFIRM_META_ADS_REMOVALS.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn manage_business_asset_access(
        &self,
        Parameters(input): Parameters<ManageBusinessAssetAccessInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        manage_access(&self.graph, input).await.into_mcp_result()
    }
    #[tool(
        name = "delete_business_asset",
        description = "Remove a business user or delete a publisher blocklist after verifying its kind. Requires CONFIRM_META_ADS_REMOVALS.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn delete_business_asset(
        &self,
        Parameters(input): Parameters<DeleteBusinessAssetInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        let result: Result<(String, BusinessAssetKind), PublicError> = (|| {
            validate_removal_acknowledgement(true, Some(&input.removal_acknowledgement))?;
            Ok((
                graph_tools::id(&input.object_id, "object_id")?,
                match input.kind {
                    DeletableBusinessAsset::BusinessUser => BusinessAssetKind::BusinessUser,
                    DeletableBusinessAsset::PublisherBlockList => {
                        BusinessAssetKind::PublisherBlockList
                    }
                },
            ))
        })();
        match result {
            Ok((id, kind)) => match verify_kind(&self.graph, &id, kind).await {
                Ok(()) => graph_tools::delete(&self.graph, &id, Vec::new()).await,
                Err(error) => ToolResponse::error(error),
            },
            Err(error) => ToolResponse::error(error),
        }
        .into_mcp_result()
    }
}

fn invalid(message: impl Into<String>) -> PublicError {
    PublicError::invalid_input(
        message,
        "Use the documented v26 fields for this asset; keep credentials outside tools",
    )
}
fn asset_id(kind: BusinessAssetKind, raw: &str) -> Result<String, PublicError> {
    if matches!(kind, BusinessAssetKind::AdAccount) {
        graph_tools::account(raw)
    } else {
        graph_tools::id(raw, "object_id")
    }
}
async fn verify_kind(
    graph: &GraphClient,
    id: &str,
    kind: BusinessAssetKind,
) -> Result<(), PublicError> {
    if matches!(kind, BusinessAssetKind::AdAccount) {
        let expected = graph_tools::account(id)?;
        let payload = graph
            .get_json(
                &expected,
                &[("fields".into(), "id,account_id,account_status".into())],
            )
            .await
            .map_err(PublicError::from)?;
        if payload.get("id").and_then(Value::as_str) != Some(expected.as_str())
            || payload.get("account_id").and_then(Value::as_str) != expected.strip_prefix("act_")
            || payload.get("account_status").is_none_or(Value::is_null)
        {
            return Err(invalid(
                "Meta did not confirm the requested ad account; no change was sent",
            ));
        }
    } else {
        let (fields, proof) = kind.proof();
        verify_object(graph, id, fields, proof).await?;
    }
    Ok(())
}

fn build_list(input: ListBusinessAssetsInput) -> Result<(String, Params), PublicError> {
    let parent = if matches!(input.collection, BusinessAssetCollection::Businesses)
        && input.parent_id == "me"
    {
        "me".into()
    } else if input.collection.account_parent() {
        graph_tools::account(&input.parent_id)?
    } else {
        graph_tools::id(&input.parent_id, "parent_id")?
    };
    let (edge, defaults, allowed) = input.collection.contract();
    if matches!(
        input.collection,
        BusinessAssetCollection::AccountAssignedUsers
            | BusinessAssetCollection::PixelAssignedUsers
            | BusinessAssetCollection::PixelAccounts
            | BusinessAssetCollection::PixelSharedAccounts
    ) {
        required_id(&input.query, "business", false)?;
    }
    let mut params = graph_tools::read_params(&input.options, defaults)?;
    params.extend(graph_tools::form_fields(&input.query, allowed)?);
    Ok((format!("{parent}/{edge}"), params))
}

fn build_create(input: CreateBusinessAssetInput) -> Result<(String, Params), PublicError> {
    let account = matches!(
        input.kind,
        CreateBusinessAssetKind::AccountPixel | CreateBusinessAssetKind::PublisherBlockList
    );
    let parent = if account {
        graph_tools::account(&input.parent_id)?
    } else {
        graph_tools::id(&input.parent_id, "parent_id")?
    };
    let (edge, allowed) = create_contract(input.kind);
    require_nonempty(&input.fields)?;
    validate_fields(&input.fields)?;
    if matches!(input.kind, CreateBusinessAssetKind::BusinessUser) {
        required_text(&input.fields, "email", 254)?;
        required_text(&input.fields, "role", 64)?;
    } else {
        required_text(&input.fields, "name", 256)?;
    }
    Ok((
        format!("{parent}/{edge}"),
        graph_tools::form_fields(&input.fields, allowed)?,
    ))
}

fn build_update(input: &UpdateBusinessAssetInput) -> Result<(String, Params), PublicError> {
    let id = asset_id(input.kind, &input.object_id)?;
    require_nonempty(&input.fields)?;
    validate_fields(&input.fields)?;
    let destructive = matches!(
        input.kind,
        BusinessAssetKind::PublisherBlockList | BusinessAssetKind::BusinessUser
    ) || input
        .fields
        .contains_key("server_events_business_ids_remove")
        || input.fields.contains_key("server_events_business_ids")
        || input.fields.contains_key("spend_cap_action")
        || input.fields.get("spend_cap").and_then(Value::as_f64) == Some(0.0);
    validate_removal_acknowledgement(destructive, input.removal_acknowledgement.as_deref())?;
    let allowed = update_contract(input.kind)?;
    Ok((id, graph_tools::form_fields(&input.fields, allowed)?))
}

async fn update_asset(
    graph: &GraphClient,
    input: UpdateBusinessAssetInput,
) -> ToolResponse<GraphData> {
    let (path, params) = match build_update(&input) {
        Ok(value) => value,
        Err(error) => return ToolResponse::error(error),
    };
    if let Err(error) = verify_kind(graph, &path, input.kind).await {
        return ToolResponse::error(error);
    }
    graph_tools::write(graph, &path, params).await
}

async fn manage_access(
    graph: &GraphClient,
    input: ManageBusinessAssetAccessInput,
) -> ToolResponse<GraphData> {
    use AssetAccessAction::*;
    let kind = match input.action {
        AssignAccountUser | RemoveAccountUser | AssignAccountAgency | RemoveAccountAgency => {
            BusinessAssetKind::AdAccount
        }
        AssignPixelUser
        | AssignPixelAgency
        | RemovePixelAgency
        | SharePixelAccount
        | UnsharePixelAccount
        | ConfigurePixelAppLinks => BusinessAssetKind::Pixel,
        ClaimBusinessAdAccount | RemoveBusinessAdAccount | RemoveBusinessAgency => {
            BusinessAssetKind::Business
        }
        AppendBlockedPublishers => BusinessAssetKind::PublisherBlockList,
    };
    let (path, params, delete) = match build_access(input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let parent = path.split('/').next().expect("validated parent path");
    if let Err(error) = verify_kind(graph, parent, kind).await {
        return ToolResponse::error(error);
    }
    if delete {
        graph_tools::delete(graph, &path, params).await
    } else {
        graph_tools::write(graph, &path, params).await
    }
}

fn build_access(
    input: ManageBusinessAssetAccessInput,
) -> Result<(String, Params, bool), PublicError> {
    use AssetAccessAction::*;
    let account = matches!(
        input.action,
        AssignAccountUser | RemoveAccountUser | AssignAccountAgency | RemoveAccountAgency
    );
    let parent = if account {
        graph_tools::account(&input.parent_id)?
    } else {
        graph_tools::id(&input.parent_id, "parent_id")?
    };
    let (edge, allowed, delete): (&str, &[&str], bool) = match input.action {
        AssignAccountUser | AssignPixelUser => ("assigned_users", &["user", "tasks"], false),
        RemoveAccountUser => ("assigned_users", &["user"], true),
        AssignAccountAgency | AssignPixelAgency => {
            ("agencies", &["business", "permitted_tasks"], false)
        }
        RemoveAccountAgency | RemovePixelAgency | RemoveBusinessAgency => {
            ("agencies", &["business"], true)
        }
        SharePixelAccount => ("shared_accounts", &["account_id", "business"], false),
        UnsharePixelAccount => ("shared_accounts", &["account_id", "business"], true),
        ClaimBusinessAdAccount => ("owned_ad_accounts", &["adaccount_id"], false),
        RemoveBusinessAdAccount => ("ad_accounts", &["adaccount_id"], true),
        AppendBlockedPublishers => ("append_publisher_urls", &["publisher_urls"], false),
        ConfigurePixelAppLinks => ("ahp_configs", &["applink_autosetup"], false),
    };
    validate_removal_acknowledgement(delete, input.removal_acknowledgement.as_deref())?;
    validate_fields(&input.fields)?;
    match input.action {
        AssignAccountUser | RemoveAccountUser | AssignPixelUser => {
            required_id(&input.fields, "user", false)?;
        }
        AssignAccountAgency | RemoveAccountAgency | AssignPixelAgency | RemovePixelAgency
        | RemoveBusinessAgency => {
            required_id(&input.fields, "business", false)?;
        }
        SharePixelAccount | UnsharePixelAccount => {
            required_id(&input.fields, "business", false)?;
            required_id(&input.fields, "account_id", true)?;
        }
        ClaimBusinessAdAccount | RemoveBusinessAdAccount => {
            required_id(&input.fields, "adaccount_id", true)?;
        }
        ConfigurePixelAppLinks => {
            if !input
                .fields
                .get("applink_autosetup")
                .is_some_and(Value::is_boolean)
            {
                return Err(invalid("applink_autosetup must be a boolean"));
            }
        }
        AppendBlockedPublishers => {
            let urls = input
                .fields
                .get("publisher_urls")
                .and_then(Value::as_array)
                .filter(|v| !v.is_empty() && v.len() <= 100)
                .ok_or_else(|| invalid("publisher_urls must contain 1 through 100 strings"))?;
            for value in urls {
                graph_tools::text(
                    value
                        .as_str()
                        .ok_or_else(|| invalid("publisher_urls must be strings"))?,
                    "publisher_urls",
                    2048,
                )?;
            }
        }
    }
    if matches!(input.action, AssignAccountUser | AssignPixelUser)
        && !input
            .fields
            .get("tasks")
            .and_then(Value::as_array)
            .is_some_and(|tasks| !tasks.is_empty())
    {
        return Err(invalid("Assignment requires a nonempty tasks array"));
    }
    if matches!(input.action, AssignAccountAgency | AssignPixelAgency)
        && !input
            .fields
            .get("permitted_tasks")
            .and_then(Value::as_array)
            .is_some_and(|tasks| !tasks.is_empty())
    {
        return Err(invalid(
            "Agency assignment requires a nonempty permitted_tasks array",
        ));
    }
    Ok((
        format!("{parent}/{edge}"),
        graph_tools::form_fields(&input.fields, allowed)?,
        delete,
    ))
}

fn require_nonempty(fields: &Map<String, Value>) -> Result<(), PublicError> {
    if fields.is_empty() {
        Err(invalid("fields cannot be empty"))
    } else {
        Ok(())
    }
}
fn required_text(
    fields: &Map<String, Value>,
    key: &str,
    max: usize,
) -> Result<String, PublicError> {
    graph_tools::text(
        fields
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| invalid(format!("{key} must be a string")))?,
        key,
        max,
    )
}
fn required_id(
    fields: &Map<String, Value>,
    key: &str,
    account: bool,
) -> Result<String, PublicError> {
    let value = fields
        .get(key)
        .ok_or_else(|| invalid(format!("{key} is required")))?;
    let raw = match value {
        Value::String(text) => text.clone(),
        Value::Number(number) if number.is_u64() => number.to_string(),
        _ => return Err(invalid(format!("{key} must be a numeric ID"))),
    };
    if account {
        graph_tools::account(&raw)
    } else {
        graph_tools::id(&raw, key)
    }
}
fn validate_fields(fields: &Map<String, Value>) -> Result<(), PublicError> {
    for key in [
        "spec",
        "agency_client_declaration",
        "business_info",
        "custom_audience_info",
        "tos_accepted",
    ] {
        if fields.get(key).is_some_and(|value| !value.is_object()) {
            return Err(invalid(format!(
                "{key} must be a JSON object, not encoded text"
            )));
        }
    }
    if let Some(value) = fields.get("spend_cap")
        && !value
            .as_f64()
            .is_some_and(|value| value.is_finite() && value >= 0.0)
    {
        return Err(invalid(
            "spend_cap must be a nonnegative number in Meta's account units",
        ));
    }
    Ok(())
}

fn create_contract(kind: CreateBusinessAssetKind) -> (&'static str, &'static [&'static str]) {
    use CreateBusinessAssetKind::*;
    match kind {
        // SDK 26.0.0 business.py:1511.
        BusinessPixel => ("adspixels", &["is_crm", "name"]),
        // SDK 26.0.0 adaccount.py:1601.
        AccountPixel => ("adspixels", &["name"]),
        // SDK 26.0.0 business.py:1379.
        Dataset => (
            "ads_dataset",
            &["ad_account_id", "app_id", "is_crm", "name"],
        ),
        // SDK 26.0.0 business.py:1799.
        BusinessUser => (
            "business_users",
            &["email", "invited_user_type", "role", "tasks"],
        ),
        // SDK 26.0.0 business.py:4461.
        SystemUser => ("system_users", &["name", "role", "system_user_id"]),
        // SDK 26.0.0 business.py:1114.
        AdAccount => (
            "adaccount",
            &[
                "ad_account_created_from_bm_flag",
                "currency",
                "end_advertiser",
                "funding_id",
                "invoice",
                "invoice_group_id",
                "invoicing_emails",
                "io",
                "media_agency",
                "name",
                "partner",
                "po_number",
                "timezone_id",
            ],
        ),
        // SDK 26.0.0 adaccount.py:3757.
        PublisherBlockList => ("publisher_block_lists", &["name"]),
    }
}
fn update_contract(kind: BusinessAssetKind) -> Result<&'static [&'static str], PublicError> {
    use BusinessAssetKind::*;
    Ok(match kind {
        // SDK 26.0.0 business.py:836.
        Business => &[
            "entry_point",
            "name",
            "primary_page",
            "timezone_id",
            "two_factor_type",
            "vertical",
        ],
        // SDK 26.0.0 adaccount.py:283.
        AdAccount => &[
            "agency_client_declaration",
            "attribution_spec",
            "business_info",
            "currency",
            "custom_audience_info",
            "default_dsa_beneficiary",
            "default_dsa_payor",
            "end_advertiser",
            "existing_customers",
            "is_ba_skip_delayed_eligible",
            "is_notifications_enabled",
            "media_agency",
            "name",
            "partner",
            "spend_cap",
            "spend_cap_action",
            "timezone_id",
            "tos_accepted",
        ],
        // SDK 26.0.0 adspixel.py:145.
        Pixel => &[
            "automatic_matching_fields",
            "data_use_setting",
            "enable_automatic_matching",
            "first_party_cookie_status",
            "name",
            "server_events_business_ids",
            "server_events_business_ids_add",
            "server_events_business_ids_remove",
        ],
        // SDK 26.0.0 businessuser.py:155.
        BusinessUser => &[
            "clear_pending_email",
            "email",
            "first_name",
            "last_name",
            "pending_email",
            "role",
            "skip_verification_email",
            "tasks",
            "title",
        ],
        // SDK 26.0.0 publisherblocklist.py:114.
        PublisherBlockList => &["spec"],
        SystemUser => {
            return Err(invalid(
                "The v26 SDK has no system-user node update contract",
            ));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::MetaConfig, safety::REMOVAL_ACKNOWLEDGEMENT};
    use serde_json::json;

    #[test]
    fn dataset_and_blocklist_contracts_match_v26_not_competitor_wrappers() {
        let request=build_create(serde_json::from_value(json!({"kind":"dataset","parent_id":"123","fields":{"name":"CRM","is_crm":true,"ad_account_id":"456"}})).unwrap()).unwrap();
        assert_eq!(request.0, "123/ads_dataset");
        assert!(request.1.contains(&("is_crm".into(), "true".into())));
        let request=build_access(serde_json::from_value(json!({"action":"append_blocked_publishers","parent_id":"123","fields":{"publisher_urls":["example.com"]}})).unwrap()).unwrap();
        assert_eq!(request.0, "123/append_publisher_urls");
        assert_eq!(
            request.1,
            vec![("publisher_urls".into(), "[\"example.com\"]".into())]
        );
        assert!(!request.2);
        let request=build_list(serde_json::from_value(json!({"collection":"pixel_stats","parent_id":"123","query":{"aggregation":"event","event":"Purchase","start_time":"2026-09-01"}})).unwrap()).unwrap();
        assert_eq!(request.0, "123/stats");
        let request=build_list(serde_json::from_value(json!({"collection":"account_activities","parent_id":"456","query":{"since":"2026-09-01","add_children":true}})).unwrap()).unwrap();
        assert_eq!(request.0, "act_456/activities");
        let request = build_list(
            serde_json::from_value(json!({"collection":"businesses","parent_id":"me"})).unwrap(),
        )
        .unwrap();
        assert_eq!(request.0, "me/businesses");
    }

    #[test]
    fn asset_mutations_reject_credentials_unsupported_fields_and_unacknowledged_removals() {
        for fields in [
            json!({"name":"Test","access_token":"secret"}),
            json!({"name":"Test","method":"DELETE"}),
            json!({"name":"Test","credentials":{"key":"secret"}}),
        ] {
            assert!(
                build_create(
                    serde_json::from_value(
                        json!({"kind":"business_pixel","parent_id":"123","fields":fields})
                    )
                    .unwrap()
                )
                .is_err()
            );
        }
        let remove = json!({"action":"unshare_pixel_account","parent_id":"123","fields":{"account_id":"456","business":"789"}});
        assert!(build_access(serde_json::from_value(remove.clone()).unwrap()).is_err());
        let mut confirmed = remove;
        confirmed["removal_acknowledgement"] = json!(REMOVAL_ACKNOWLEDGEMENT);
        let request = build_access(serde_json::from_value(confirmed).unwrap()).unwrap();
        assert_eq!(request.0, "123/shared_accounts");
        assert!(request.2);
        let remove = json!({
            "action":"remove_business_ad_account", "parent_id":"123",
            "fields":{"adaccount_id":"456"}
        });
        assert!(build_access(serde_json::from_value(remove.clone()).unwrap()).is_err());
        let mut confirmed = remove;
        confirmed["removal_acknowledgement"] = json!(REMOVAL_ACKNOWLEDGEMENT);
        let request = build_access(serde_json::from_value(confirmed).unwrap()).unwrap();
        assert_eq!(request.0, "123/ad_accounts");
        assert_eq!(request.1, vec![("adaccount_id".into(), "456".into())]);
        assert!(request.2);
        for fields in [
            json!({"spend_cap":0}),
            json!({"spend_cap_action":"reset"}),
            json!({"spend_cap":"0"}),
            json!({"timezone_name":"America/Chicago"}),
        ] {
            assert!(
                build_update(
                    &serde_json::from_value(
                        json!({"kind":"ad_account","object_id":"123","fields":fields})
                    )
                    .unwrap()
                )
                .is_err()
            );
        }
        assert!(build_update(&serde_json::from_value(json!({"kind":"publisher_block_list","object_id":"123","fields":{"spec":"{\"password\":\"hidden\"}"},"removal_acknowledgement":REMOVAL_ACKNOWLEDGEMENT})).unwrap()).is_err());
        assert!(
            build_list(
                serde_json::from_value(
                    json!({"collection":"pixel_assigned_users","parent_id":"123"})
                )
                .unwrap()
            )
            .is_err()
        );
        assert!(build_access(serde_json::from_value(json!({"action":"assign_pixel_user","parent_id":"123","fields":{"user":"456","tasks":[]}})).unwrap()).is_err());
    }

    #[tokio::test]
    async fn agency_actions_reject_wrong_parent_kinds_before_mutation() {
        use std::time::Duration;
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        for (action, identity) in [
            (
                "remove_pixel_agency",
                r#"{"id":"123","verification_status":"verified","two_factor_type":"none"}"#,
            ),
            (
                "remove_business_agency",
                r#"{"id":"123","first_party_cookie_status":"FIRST_PARTY_COOKIE_ENABLED"}"#,
            ),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    let mut chunk = [0_u8; 2048];
                    let count = socket.read(&mut chunk).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&chunk[..count]);
                }
                assert!(String::from_utf8_lossy(&bytes).starts_with("GET /123?fields="));
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{identity}",identity.len()).as_bytes()).await.unwrap();
                assert!(
                    tokio::time::timeout(Duration::from_millis(100), listener.accept())
                        .await
                        .is_err()
                );
            });
            let graph = GraphClient::new(&MetaConfig::for_test(
                format!("http://{address}"),
                Some("test-access-token-1234567890"),
            ))
            .unwrap();
            let input=serde_json::from_value(json!({"action":action,"parent_id":"123","fields":{"business":"456"},"removal_acknowledgement":REMOVAL_ACKNOWLEDGEMENT})).unwrap();
            assert!(matches!(
                manage_access(&graph, input).await,
                ToolResponse::Error { .. }
            ));
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn wrong_account_identity_prevents_any_settings_write() {
        use std::time::Duration;
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = vec![0_u8; 4096];
            let count = socket.read(&mut bytes).await.unwrap();
            let request = String::from_utf8_lossy(&bytes[..count]).into_owned();
            assert!(
                request
                    .starts_with("GET /act_123?fields=id%2Caccount_id%2Caccount_status HTTP/1.1")
            );
            let body = r#"{"id":"act_999","account_id":"999","account_status":1}"#;
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err()
            );
        });
        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        ))
        .unwrap();
        let result = update_asset(
            &graph,
            serde_json::from_value(
                json!({"kind":"ad_account","object_id":"123","fields":{"name":"Renamed"}}),
            )
            .unwrap(),
        )
        .await;
        assert!(matches!(result, ToolResponse::Error { .. }));
        server.await.unwrap();
    }
}
