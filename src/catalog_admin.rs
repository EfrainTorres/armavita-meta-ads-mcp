// Copyright (C) 2025 ArmaVita LLC
// SPDX-License-Identifier: AGPL-3.0-only

//! Catalog contracts verified against Meta Business SDK 26.0.1.
//! The inline 26.0.0 line references identify unchanged request contracts.
//! Existing product list/upsert/batch tools retain their original contracts.

use std::collections::HashSet;

use rmcp::{
    Json,
    handler::server::wrapper::Parameters,
    schemars::{self, JsonSchema},
    tool, tool_router,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    graph_tools::{self, GraphData, Params, ReadOptions},
    safety::validate_removal_acknowledgement,
    server::MetaAdsServer,
};

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CatalogObjectKind {
    Catalog,
    ProductSet,
    Product,
    ProductGroup,
    Feed,
    FeedRule,
    FeedSchedule,
    FeedUpload,
    FeedUploadError,
    Hotel,
    HotelRoom,
    Flight,
    Destination,
    Vehicle,
    VehicleOffer,
    HomeListing,
    AutomotiveModel,
    MediaTitle,
}

impl CatalogObjectKind {
    fn defaults(self) -> &'static str {
        match self {
            Self::Catalog => "id,name,vertical,product_count,feed_count,business",
            Self::ProductSet => "id,name,product_catalog,product_count,filter,retailer_id",
            Self::Product => {
                "id,retailer_id,name,description,availability,price,currency,product_catalog"
            }
            Self::ProductGroup => "id,retailer_id,product_catalog,variants",
            Self::Feed => {
                "id,name,product_count,default_currency,deletion_enabled,schedule,update_schedule"
            }
            Self::FeedRule => "id,attribute,rule_type,params",
            Self::FeedSchedule => "id,interval,interval_count,hour,minute,day_of_week,timezone,url",
            Self::FeedUpload => {
                "id,start_time,end_time,error_count,warning_count,num_detected_items,num_persisted_items,num_invalid_items,num_deleted_items"
            }
            Self::FeedUploadError => "id,error_type,severity,summary,total_count,affected_surfaces",
            Self::Hotel => "id,hotel_id,name,currency,lowest_base_price,url",
            Self::HotelRoom => "id,room_id,name,currency,base_price,url",
            Self::Flight => "id,flight_id,origin_airport,destination_airport,price,currency",
            Self::Destination => "id,destination_id,name,price,currency,url",
            Self::Vehicle => "id,vehicle_id,title,make,model,year,price,currency,availability",
            Self::VehicleOffer => "id,vehicle_offer_id,title,offer_type,price,currency",
            Self::HomeListing => "id,home_listing_id,name,availability,price,currency,url",
            Self::AutomotiveModel => "id,automotive_model_id,title,make,model,year,price,currency",
            Self::MediaTitle => "id",
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadCatalogObjectInput {
    pub kind: CatalogObjectKind,
    pub object_id: String,
    #[serde(default)]
    pub options: ReadOptions,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CatalogCollection {
    /// Business ID.
    OwnedCatalogs,
    /// Business ID.
    ClientCatalogs,
    /// Catalog ID; optional ancestor_id, parent_id, retailer_id, has_children.
    ProductSets,
    /// Catalog ID.
    ProductGroups,
    /// Catalog, product-set, or feed ID; optional filter/error_priority/error_type.
    Products,
    /// Catalog, product-set, or feed ID; optional filter.
    Hotels,
    Flights,
    Destinations,
    Vehicles,
    VehicleOffers,
    HomeListings,
    AutomotiveModels,
    /// Catalog ID.
    Feeds,
    DataSources,
    Diagnostics,
    EventStats,
    EventSources,
    Categories,
    Agencies,
    /// Catalog ID; business is required in query.
    AssignedUsers,
    /// Feed ID.
    FeedRules,
    FeedSchedules,
    FeedUploads,
    /// Feed-upload ID; optional error_priority.
    UploadErrors,
    /// Feed-upload-error ID.
    ErrorSamples,
    SuggestedFeedRules,
    /// Product ID.
    ProductIntegrity,
    ProductOverrides,
    ProductSetsForProduct,
    ProductVideos,
    /// Catalog ID.
    CollaborativeImageBank,
    CollaborativeShareSettings,
    CreatorAssets,
    VersionConfigurations,
}

impl CatalogCollection {
    fn contract(self) -> (&'static str, &'static str, &'static [&'static str]) {
        match self {
            Self::OwnedCatalogs => (
                "owned_product_catalogs",
                "id,name,vertical,product_count,feed_count",
                &[],
            ),
            Self::ClientCatalogs => (
                "client_product_catalogs",
                "id,name,vertical,product_count,feed_count",
                &[],
            ),
            Self::ProductSets => (
                "product_sets",
                "id,name,product_count,filter,retailer_id",
                &["ancestor_id", "parent_id", "retailer_id", "has_children"],
            ),
            Self::ProductGroups => ("product_groups", "id,retailer_id,variants", &[]),
            Self::Products => (
                "products",
                CatalogObjectKind::Product.defaults(),
                &[
                    "filter",
                    "error_priority",
                    "error_type",
                    "return_only_approved_products",
                ],
            ),
            Self::Hotels => ("hotels", CatalogObjectKind::Hotel.defaults(), &["filter"]),
            Self::Flights => ("flights", CatalogObjectKind::Flight.defaults(), &["filter"]),
            Self::Destinations => (
                "destinations",
                CatalogObjectKind::Destination.defaults(),
                &["filter"],
            ),
            Self::Vehicles => (
                "vehicles",
                CatalogObjectKind::Vehicle.defaults(),
                &["filter"],
            ),
            Self::VehicleOffers => (
                "vehicle_offers",
                CatalogObjectKind::VehicleOffer.defaults(),
                &["filter"],
            ),
            Self::HomeListings => (
                "home_listings",
                CatalogObjectKind::HomeListing.defaults(),
                &["filter"],
            ),
            Self::AutomotiveModels => (
                "automotive_models",
                CatalogObjectKind::AutomotiveModel.defaults(),
                &["filter"],
            ),
            Self::Feeds => ("product_feeds", CatalogObjectKind::Feed.defaults(), &[]),
            Self::DataSources => (
                "data_sources",
                "id,name,ingestion_source_type,upload_type",
                &["ingestion_source_type"],
            ),
            Self::Diagnostics => (
                "diagnostics",
                "title,severity,type,error_code,number_of_affected_items,number_of_affected_entities",
                &[
                    "affected_channels",
                    "affected_entities",
                    "affected_features",
                    "severities",
                    "types",
                ],
            ),
            Self::EventStats => (
                "event_stats",
                "event,event_source,date_start,date_stop,total_matched_content_ids,total_unmatched_content_ids",
                &["breakdowns"],
            ),
            Self::EventSources => ("external_event_sources", "id,name", &[]),
            Self::Categories => (
                "categories",
                "name,num_items,criteria_value",
                &["categorization_criteria", "filter"],
            ),
            Self::Agencies => ("agencies", "id,name", &[]),
            Self::AssignedUsers => (
                "assigned_users",
                "id,name,user_type,business",
                &["business"],
            ),
            Self::FeedRules => ("rules", CatalogObjectKind::FeedRule.defaults(), &[]),
            Self::FeedSchedules => (
                "upload_schedules",
                CatalogObjectKind::FeedSchedule.defaults(),
                &[],
            ),
            Self::FeedUploads => ("uploads", CatalogObjectKind::FeedUpload.defaults(), &[]),
            Self::UploadErrors => (
                "errors",
                CatalogObjectKind::FeedUploadError.defaults(),
                &["error_priority"],
            ),
            Self::ErrorSamples => ("samples", "id,retailer_id,row_number", &[]),
            Self::SuggestedFeedRules => ("suggested_rules", "attribute,type,params", &[]),
            Self::ProductIntegrity => (
                "channels_to_integrity_status",
                "channels,rejection_information",
                &[],
            ),
            Self::ProductOverrides => ("override_details", "key,type,values", &["keys", "type"]),
            Self::ProductSetsForProduct => (
                "product_sets",
                CatalogObjectKind::ProductSet.defaults(),
                &[],
            ),
            Self::ProductVideos => ("videos_metadata", "id", &[]),
            Self::CollaborativeImageBank => (
                "collaborative_ads_lsb_image_bank",
                "id,ad_group_id,catalog_segment_proxy_id,agency_business_id,backup_image_urls",
                &[],
            ),
            Self::CollaborativeShareSettings => (
                "collaborative_ads_share_settings",
                "id,agency_business,product_catalog_proxy_id,utm_campaign,utm_medium,utm_source",
                &[],
            ),
            Self::CreatorAssets => (
                "creator_asset_creatives",
                "id,retailer_id,product_item_retailer_id,moderation_status,image_url,video_url,product_url",
                &["moderation_status"],
            ),
            Self::VersionConfigurations => ("version_configs", "id,name,version", &[]),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListCatalogResourcesInput {
    pub collection: CatalogCollection,
    /// ID of the parent described by collection.
    pub parent_id: String,
    #[serde(default)]
    pub options: ReadOptions,
    /// Only documented filters for the selected collection; use JSON objects, not encoded JSON strings.
    #[serde(default)]
    pub query: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CatalogWriteOperation {
    Create,
    Update,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct WriteCatalogObjectInput {
    pub kind: CatalogObjectKind,
    pub operation: CatalogWriteOperation,
    /// Create: business ID for catalog; catalog ID for sets/feeds/items/groups; feed ID for rules/schedules. Update: object ID.
    pub target_id: String,
    /// Official v26 fields. Common fields: name, filter (sets), schedule/update_schedule (feeds), attribute/rule_type/params (rules). Schedules must be objects with credential-free HTTPS URLs.
    pub fields: Map<String, Value>,
    /// Required when enabling feed deletion; otherwise omit.
    #[schemars(
        length(min = 25, max = 25),
        regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
    )]
    pub removal_acknowledgement: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeleteCatalogObjectInput {
    /// Catalog, product_set, product, product_group, feed, feed_rule, hotel, or home_listing.
    pub kind: CatalogObjectKind,
    pub object_id: String,
    /// Explicitly permit removing a catalog/product set currently used by live ads. Defaults to false.
    #[serde(default)]
    pub allow_live_product_set_deletion: bool,
    /// Exact phrase CONFIRM_META_ADS_REMOVALS.
    #[schemars(
        length(min = 25, max = 25),
        regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
    )]
    pub removal_acknowledgement: String,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CatalogConnectionAction {
    ConnectEventSources,
    DisconnectEventSources,
    AssignUser,
    RemoveUser,
    AssignAgency,
    RemoveAgency,
    ConnectStore,
    AssociateSupplementaryFeeds,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManageCatalogConnectionInput {
    pub action: CatalogConnectionAction,
    /// Catalog ID, or feed ID for associate_supplementary_feeds.
    pub parent_id: String,
    /// Event sources: external_event_sources array of IDs. Users: user plus tasks. Agencies: business plus permitted_tasks/roles. Store: page. Supplementary feeds: assoc_data.
    pub fields: Map<String, Value>,
    /// Required for disconnect/remove actions and supplementary-feed association changes.
    #[schemars(
        length(min = 25, max = 25),
        regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
    )]
    pub removal_acknowledgement: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CatalogUploadKind {
    Feed,
    HotelRooms,
    PricingVariables,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UploadCatalogFeedInput {
    pub kind: CatalogUploadKind,
    /// Feed ID for feed, catalog ID for hotel_rooms or pricing_variables.
    pub parent_id: String,
    /// Public HTTPS feed URL; credentials and signed credential query parameters are rejected.
    pub url: Option<String>,
    /// Alternatively stream a relative file below META_MEDIA_ROOT, up to 64 MiB. Choose exactly one source.
    pub local_path: Option<String>,
    /// Required with local_path: csv, tsv, xml, or json. Full feed validation runs in Meta.
    pub local_format: Option<crate::media_uploads::CatalogFeedFormat>,
    /// Update existing items only. Defaults to true; false may create/delete items according to feed settings.
    #[serde(default = "default_true")]
    pub update_only: bool,
    /// Optional provider standard for hotel_rooms/pricing_variables; omit for feed.
    pub standard: Option<String>,
    /// Required for a full replacement upload (update_only=false).
    #[schemars(
        length(min = 25, max = 25),
        regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
    )]
    pub removal_acknowledgement: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CatalogBatchKind {
    Items,
    LocalizedItems,
    GeolocatedItems,
    VersionItems,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CatalogItemOperation {
    Upsert {
        /// Include the catalog identity inside data: PRODUCT_ITEM uses id; VEHICLE uses vehicle_id; FLIGHT uses origin_airport plus destination_airport. Other fields use the feed-format names documented for items_batch.
        data: Map<String, Value>,
        /// Required only for localized_items.
        localization: Option<CatalogLocalization>,
        /// Optional documented per-record metadata for geolocated/versioned items; cannot override method or data.
        metadata: Option<Map<String, Value>>,
    },
    Delete {
        /// Only identity fields, such as {"id":"SKU-1"} or {"vehicle_id":"CAR-1"}.
        data: Map<String, Value>,
        localization: Option<CatalogLocalization>,
        metadata: Option<Map<String, Value>>,
    },
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogLocalization {
    #[serde(rename = "type")]
    pub kind: CatalogLocalizationKind,
    /// Language/country code, or language|country for LANGUAGE_AND_COUNTRY.
    pub value: String,
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum CatalogLocalizationKind {
    Language,
    Country,
    LanguageAndCountry,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BatchCatalogItemsInput {
    pub catalog_id: String,
    pub kind: CatalogBatchKind,
    /// Official item type, such as PRODUCT_ITEM, VEHICLE, HOTEL, FLIGHT or MEDIA_TITLE.
    pub item_type: String,
    #[schemars(length(min = 1, max = 100))]
    pub operations: Vec<CatalogItemOperation>,
    /// Defaults to true for ordinary/geolocated/versioned batches; omitted for localized_items unless explicitly supplied. Localized batches require an existing base item.
    pub allow_upsert: Option<bool>,
    /// Optional item subtype, only for items.
    pub item_sub_type: Option<String>,
    /// Optional catalog batch version; unavailable for geolocated_items.
    pub version: Option<u64>,
    /// Required only for version_items.
    pub item_version: Option<String>,
    /// Required when any operation deletes an item.
    #[schemars(
        length(min = 25, max = 25),
        regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
    )]
    pub removal_acknowledgement: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CatalogBatchStatusKind {
    Items,
    ProductSets,
    HotelRooms,
    PricingVariables,
    MarketplaceDeals,
    MarketplaceSellers,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadCatalogBatchStatusInput {
    pub catalog_id: String,
    pub kind: CatalogBatchStatusKind,
    /// Batch handle, or session_id returned by marketplace partner submissions.
    pub handle: String,
    /// Ask for invalid request IDs for item batches only.
    #[serde(default)]
    pub include_invalid_ids: bool,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateFeedErrorReportInput {
    pub feed_upload_id: String,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CatalogAdConfigurationAction {
    CreateCategories,
    UpdateGeneratedImages,
    SetCollaborativeImageBank,
    SubmitMarketplaceDeals,
    SubmitMarketplaceSellers,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConfigureCatalogAdvertisingInput {
    pub catalog_id: String,
    pub action: CatalogAdConfigurationAction,
    /// Categories/generated images: data array of objects. Image bank: ad_group_id, agency_business_id, backup_image_urls. Marketplace submissions: requests object following Meta's partner contract.
    pub fields: Map<String, Value>,
    /// Required for partner submissions because their batch contract can replace or remove records.
    #[schemars(
        length(min = 25, max = 25),
        regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
    )]
    pub removal_acknowledgement: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum MarketplaceEventName {
    AddToCart,
    OfferSubmitted,
    Purchase,
    PurchaseViaOffer,
    Test,
    ViewItem,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum MarketplaceConversionType {
    Attributed,
    InSession,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SendMarketplaceSignalInput {
    pub catalog_id: String,
    pub event_name: MarketplaceEventName,
    pub conversion_type: MarketplaceConversionType,
    /// Stable ID for event deduplication; reuse it when checking an uncertain submission.
    pub event_id: String,
    /// Unix seconds, within the past seven days.
    pub event_time: u64,
    pub event_source_url: Option<String>,
    /// Raw matching identifiers are normalized and hashed locally, using the same boundary as send_capi_events.
    pub user_data: crate::capi::CapiUserDataInput,
    /// Marketplace offer metadata; customer matching identifiers belong in user_data.
    pub offer_data: Option<Map<String, Value>>,
    /// Marketplace order metadata; customer matching identifiers belong in user_data.
    pub order_data: Option<Map<String, Value>>,
}

#[tool_router(router = catalog_admin_router, vis = "pub(crate)")]
impl MetaAdsServer {
    #[tool(
        name = "read_catalog_object",
        description = "Read catalog, product, set, feed, upload, rule, or vertical-item details with bounded selectable fields.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn read_catalog_object(
        &self,
        Parameters(input): Parameters<ReadCatalogObjectInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        let request: Result<(String, Params), PublicError> = (|| {
            Ok((
                graph_tools::id(&input.object_id, "object_id")?,
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
        name = "list_catalog_resources",
        description = "Browse catalog items, feeds, diagnostics, event matching, assignments, upload errors, and schedules. One bounded page; collection determines the parent ID and filters.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_catalog_resources(
        &self,
        Parameters(input): Parameters<ListCatalogResourcesInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        match build_list(input) {
            Ok((path, params)) => graph_tools::read(&self.graph, &path, params).await,
            Err(error) => ToolResponse::error(error),
        }
        .into_mcp_result()
    }

    #[tool(
        name = "write_catalog_object",
        description = "Create or update a catalog, product set, feed, feed rule, schedule, group, or supported item. Fields follow Graph v26; creation may duplicate objects. Feed deletion requires explicit acknowledgement.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn write_catalog_object(
        &self,
        Parameters(input): Parameters<WriteCatalogObjectInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        write_catalog(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "delete_catalog_object",
        description = "Delete one catalog, set, feed, rule, product/group, hotel, or home listing after CONFIRM_META_ADS_REMOVALS. Live-set deletion is disabled unless explicitly enabled.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn delete_catalog_object(
        &self,
        Parameters(input): Parameters<DeleteCatalogObjectInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        delete_catalog(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "manage_catalog_connection",
        description = "Connect catalog event sources, assign users/agencies, connect a Page store, or associate supplementary feeds. Removals require CONFIRM_META_ADS_REMOVALS.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn manage_catalog_connection(
        &self,
        Parameters(input): Parameters<ManageCatalogConnectionInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        manage_connection(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "upload_catalog_feed",
        description = "Upload a product, hotel-room, or pricing-variable feed from public HTTPS or a CSV/TSV/XML/JSON file under META_MEDIA_ROOT (64 MiB maximum). Defaults to update-only; full replacement requires CONFIRM_META_ADS_REMOVALS.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn upload_catalog_feed(
        &self,
        Parameters(input): Parameters<UploadCatalogFeedInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        upload_feed(&self.graph, self.media_root.as_deref(), input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "batch_catalog_items",
        description = "Upsert/delete bounded catalog items, including vertical, localized, geolocated, or versioned attributes. Returns an async handle; inspect read_catalog_batch_status before retrying.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn batch_catalog_items(
        &self,
        Parameters(input): Parameters<BatchCatalogItemsInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        match build_batch(input) {
            Ok((path, params)) => match graph_tools::write(&self.graph, &path, params).await {
                ToolResponse::Success { data } => graph_tools::response(catalog_batch_ack(data)),
                response => response,
            },
            Err(error) => ToolResponse::error(error),
        }
        .into_mcp_result()
    }

    #[tool(
        name = "read_catalog_batch_status",
        description = "Read completion, warnings, and errors for a catalog item, product-set, hotel-room, or pricing-variable batch handle.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn read_catalog_batch_status(
        &self,
        Parameters(input): Parameters<ReadCatalogBatchStatusInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        match build_batch_status(input) {
            Ok((path, params)) => graph_tools::read(&self.graph, &path, params).await,
            Err(error) => ToolResponse::error(error),
        }
        .into_mcp_result()
    }

    #[tool(
        name = "configure_catalog_advertising",
        description = "Configure catalog categories, generated images, collaborative-ad image banks, or marketplace partner seller/deal batches. Marketplace batches require removal acknowledgement; check their returned session with read_catalog_batch_status.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn configure_catalog_advertising(
        &self,
        Parameters(input): Parameters<ConfigureCatalogAdvertisingInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        match build_ad_configuration(input) {
            Ok((path, params)) => graph_tools::write(&self.graph, &path, params).await,
            Err(error) => ToolResponse::error(error),
        }
        .into_mcp_result()
    }

    #[tool(
        name = "send_marketplace_signal",
        description = "Send one marketplace partner conversion signal. Matching identifiers are hashed locally; use a stable event_id. Submission is single-shot and never retried automatically.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn send_marketplace_signal(
        &self,
        Parameters(input): Parameters<SendMarketplaceSignalInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        match build_marketplace_signal(
            input,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_secs()),
        ) {
            Ok((path, params)) => {
                // Return only acknowledgement fields, never provider echoes of matching data.
                match graph_tools::write(&self.graph, &path, params).await {
                    ToolResponse::Success { mut data } => {
                        let acknowledged = data.result == Value::Bool(true)
                            || data.result.get("success") == Some(&Value::Bool(true))
                            || data.result.get("events_received").and_then(Value::as_u64).is_some_and(|count| count > 0)
                            || data.result.get("id").and_then(Value::as_str).is_some_and(|id| graph_tools::id(id, "id").is_ok());
                        if !acknowledged {
                            return ToolResponse::error(crate::mutation_result::ambiguous_mutation_result("Meta did not confirm the marketplace signal", "Verify the event in Meta before retrying with the same event_id")).into_mcp_result();
                        }
                        if let Some(object) = data.result.as_object_mut() {
                            object.retain(|key, _| matches!(key.as_str(), "id" | "success" | "events_received"));
                        }
                        ToolResponse::success(data)
                    }
                    response => response,
                }
            }
            Err(error) => ToolResponse::error(error),
        }
        .into_mcp_result()
    }

    #[tool(
        name = "create_feed_error_report",
        description = "Request an error report for one feed upload; read its error_report field afterward. Repeated requests may regenerate the report.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_feed_error_report(
        &self,
        Parameters(input): Parameters<CreateFeedErrorReportInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        match graph_tools::id(&input.feed_upload_id, "feed_upload_id") {
            Ok(id) => {
                graph_tools::write(&self.graph, &format!("{id}/error_report"), Vec::new()).await
            }
            Err(error) => ToolResponse::error(error),
        }
        .into_mcp_result()
    }
}

fn invalid(message: impl Into<String>) -> PublicError {
    PublicError::invalid_input(
        message,
        "Use the documented fields for this catalog resource and operation",
    )
}

async fn write_catalog(
    graph: &GraphClient,
    input: WriteCatalogObjectInput,
) -> ToolResponse<GraphData> {
    let kind = input.kind;
    let update = matches!(input.operation, CatalogWriteOperation::Update);
    let (path, params) = match build_write(input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    if update && let Err(error) = verify_catalog_kind(graph, &path, kind).await {
        return ToolResponse::error(error);
    }
    graph_tools::write(graph, &path, params).await
}

async fn delete_catalog(
    graph: &GraphClient,
    input: DeleteCatalogObjectInput,
) -> ToolResponse<GraphData> {
    let kind = input.kind;
    let (path, params) = match build_delete(input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    if let Err(error) = verify_catalog_kind(graph, &path, kind).await {
        return ToolResponse::error(error);
    }
    graph_tools::delete(graph, &path, params).await
}

// Direct numeric-node writes must not accidentally target an ad or another object family.
async fn verify_catalog_kind(
    graph: &GraphClient,
    id: &str,
    kind: CatalogObjectKind,
) -> Result<(), PublicError> {
    let marker = match kind {
        CatalogObjectKind::Catalog => "vertical,feed_count",
        CatalogObjectKind::ProductSet => "filter,product_catalog",
        CatalogObjectKind::Product => "retailer_id,availability",
        CatalogObjectKind::ProductGroup => "variants,product_catalog",
        CatalogObjectKind::Feed => "deletion_enabled,product_count",
        CatalogObjectKind::FeedRule => "rule_type",
        CatalogObjectKind::Hotel => "hotel_id",
        CatalogObjectKind::Flight => "flight_id",
        CatalogObjectKind::Vehicle => "vehicle_id",
        CatalogObjectKind::HomeListing => "home_listing_id",
        _ => {
            return Err(invalid(
                "This catalog resource has no direct mutation contract",
            ));
        }
    };
    crate::node_identity::verify_object(
        graph,
        id,
        &format!("id,{marker}"),
        &marker.split(',').collect::<Vec<_>>(),
    )
    .await?;
    Ok(())
}

fn build_list(input: ListCatalogResourcesInput) -> Result<(String, Params), PublicError> {
    let (edge, defaults, allowed) = input.collection.contract();
    let parent = graph_tools::id(&input.parent_id, "parent_id")?;
    validate_structured_fields(&input.query)?;
    if matches!(input.collection, CatalogCollection::AssignedUsers) {
        require_id(&input.query, "business")?;
    }
    let mut params = graph_tools::read_params(&input.options, defaults)?;
    params.extend(graph_tools::form_fields(&input.query, allowed)?);
    Ok((format!("{parent}/{edge}"), params))
}

fn build_write(input: WriteCatalogObjectInput) -> Result<(String, Params), PublicError> {
    let target = graph_tools::id(&input.target_id, "target_id")?;
    let (edge, allowed) = write_contract(input.kind, input.operation)?;
    if input.fields.is_empty() {
        return Err(invalid("fields cannot be empty"));
    }
    validate_structured_fields(&input.fields)?;
    validate_removal_acknowledgement(
        input
            .fields
            .get("deletion_enabled")
            .and_then(Value::as_bool)
            == Some(true),
        input.removal_acknowledgement.as_deref(),
    )?;
    if let Some(value) = input.fields.get("deletion_enabled")
        && !value.is_boolean()
    {
        return Err(invalid("deletion_enabled must be a boolean"));
    }
    let params = graph_tools::form_fields(&input.fields, allowed)?;
    Ok((
        if edge.is_empty() {
            target
        } else {
            format!("{target}/{edge}")
        },
        params,
    ))
}

fn build_delete(input: DeleteCatalogObjectInput) -> Result<(String, Params), PublicError> {
    validate_removal_acknowledgement(true, Some(&input.removal_acknowledgement))?;
    let target = graph_tools::id(&input.object_id, "object_id")?;
    let mut params = Vec::new();
    match input.kind {
        CatalogObjectKind::Catalog => params.push((
            "allow_delete_catalog_with_live_product_set".into(),
            input.allow_live_product_set_deletion.to_string(),
        )),
        CatalogObjectKind::ProductSet => params.push((
            "allow_live_product_set_deletion".into(),
            input.allow_live_product_set_deletion.to_string(),
        )),
        CatalogObjectKind::ProductGroup if !input.allow_live_product_set_deletion => {
            params.push(("deletion_method".into(), "ONLY_IF_EMPTY".into()))
        }
        CatalogObjectKind::Product
        | CatalogObjectKind::Feed
        | CatalogObjectKind::FeedRule
        | CatalogObjectKind::Hotel
        | CatalogObjectKind::HomeListing
            if !input.allow_live_product_set_deletion => {}
        _ => {
            return Err(invalid(
                "This resource does not support this direct deletion; use the catalog item batch or feed workflow",
            ));
        }
    }
    Ok((target, params))
}

async fn manage_connection(
    graph: &GraphClient,
    input: ManageCatalogConnectionInput,
) -> ToolResponse<GraphData> {
    let kind = if matches!(
        input.action,
        CatalogConnectionAction::AssociateSupplementaryFeeds
    ) {
        CatalogObjectKind::Feed
    } else {
        CatalogObjectKind::Catalog
    };
    let (path, params, delete) = match build_connection(input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let parent = path.split('/').next().expect("validated parent path");
    if let Err(error) = verify_catalog_kind(graph, parent, kind).await {
        return ToolResponse::error(error);
    }
    if delete {
        graph_tools::delete(graph, &path, params).await
    } else {
        graph_tools::write(graph, &path, params).await
    }
}

fn build_connection(
    input: ManageCatalogConnectionInput,
) -> Result<(String, Params, bool), PublicError> {
    use CatalogConnectionAction::*;
    let (edge, allowed, remove): (&str, &[&str], bool) = match input.action {
        ConnectEventSources => ("external_event_sources", &["external_event_sources"], false),
        DisconnectEventSources => ("external_event_sources", &["external_event_sources"], true),
        AssignUser => ("assigned_users", &["user", "tasks"], false),
        RemoveUser => ("assigned_users", &["user"], true),
        AssignAgency => (
            "agencies",
            &[
                "business",
                "enabled_collab_terms",
                "permitted_roles",
                "permitted_tasks",
                "skip_defaults",
                "utm_settings",
            ],
            false,
        ),
        RemoveAgency => ("agencies", &["business"], true),
        ConnectStore => ("catalog_store", &["page"], false),
        AssociateSupplementaryFeeds => ("supplementary_feed_assocs", &["assoc_data"], false),
    };
    validate_removal_acknowledgement(
        remove || matches!(input.action, AssociateSupplementaryFeeds),
        input.removal_acknowledgement.as_deref(),
    )?;
    let parent = graph_tools::id(&input.parent_id, "parent_id")?;
    match input.action {
        AssignUser | RemoveUser => {
            require_id(&input.fields, "user")?;
            if matches!(input.action, AssignUser)
                && !input
                    .fields
                    .get("tasks")
                    .and_then(Value::as_array)
                    .is_some_and(|tasks| !tasks.is_empty() && tasks.iter().all(Value::is_string))
            {
                return Err(invalid(
                    "AssignUser requires a nonempty tasks array; use remove_user to revoke access",
                ));
            }
        }
        AssignAgency | RemoveAgency => {
            require_id(&input.fields, "business")?;
        }
        ConnectStore => {
            require_id(&input.fields, "page")?;
        }
        ConnectEventSources | DisconnectEventSources => {
            let ids = input
                .fields
                .get("external_event_sources")
                .and_then(Value::as_array)
                .filter(|v| !v.is_empty() && v.len() <= 100)
                .ok_or_else(|| {
                    invalid("external_event_sources must contain 1 through 100 numeric IDs")
                })?;
            for value in ids {
                value_id(value, "external_event_sources")?;
            }
        }
        AssociateSupplementaryFeeds => {
            if !input.fields.get("assoc_data").is_some_and(Value::is_array) {
                return Err(invalid("assoc_data must be a JSON array"));
            }
        }
    }
    validate_structured_fields(&input.fields)?;
    Ok((
        format!("{parent}/{edge}"),
        graph_tools::form_fields(&input.fields, allowed)?,
        remove,
    ))
}

async fn upload_feed(
    graph: &GraphClient,
    media_root: Option<&std::path::Path>,
    input: UploadCatalogFeedInput,
) -> ToolResponse<GraphData> {
    let (path, params) = match build_upload(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let Some(local_path) = input.local_path.as_deref() else {
        return graph_tools::write(graph, &path, params).await;
    };
    let Some(format) = input.local_format else {
        return ToolResponse::error(invalid("local_format is required with local_path"));
    };
    let opened = match crate::media_uploads::open_local_media(
        media_root,
        local_path,
        crate::media_uploads::LocalMediaKind::CatalogFeed(format),
        64 * 1024 * 1024,
    )
    .await
    {
        Ok(opened) => opened,
        Err(error) => return ToolResponse::error(error),
    };
    let result = graph.post_multipart_file_json(&path, "file", opened.file, opened.size, opened.file_name.into(), opened.mime_type, params).await
        .map_err(|error| crate::mutation_result::mutation_error_without_blind_retry(error, "Inspect the feed uploads or batch status before retrying; Meta may already have received the file"))
        .and_then(graph_tools::normalize_write);
    graph_tools::response(result)
}

fn build_upload(input: &UploadCatalogFeedInput) -> Result<(String, Params), PublicError> {
    validate_removal_acknowledgement(!input.update_only, input.removal_acknowledgement.as_deref())?;
    let parent = graph_tools::id(&input.parent_id, "parent_id")?;
    if !matches!(
        (&input.url, &input.local_path, input.local_format),
        (Some(_), None, None) | (None, Some(_), Some(_))
    ) {
        return Err(invalid(
            "Choose either url, or local_path with local_format",
        ));
    }
    let edge = match input.kind {
        CatalogUploadKind::Feed => "uploads",
        CatalogUploadKind::HotelRooms => "hotel_rooms_batch",
        CatalogUploadKind::PricingVariables => "pricing_variables_batch",
    };
    let mut params = vec![("update_only".into(), input.update_only.to_string())];
    if let Some(url) = &input.url {
        params.push(("url".into(), public_url(url)?));
    }
    if let Some(path) = &input.local_path {
        graph_tools::text(path, "local_path", 4096)?;
    }
    if let Some(standard) = &input.standard {
        if matches!(input.kind, CatalogUploadKind::Feed) {
            return Err(invalid(
                "standard is only supported for hotel-room and pricing-variable uploads",
            ));
        }
        params.push((
            "standard".into(),
            graph_tools::text(standard, "standard", 64)?,
        ));
    }
    Ok((format!("{parent}/{edge}"), params))
}

fn build_batch(input: BatchCatalogItemsInput) -> Result<(String, Params), PublicError> {
    let catalog = graph_tools::id(&input.catalog_id, "catalog_id")?;
    if input.operations.is_empty() || input.operations.len() > 100 {
        return Err(invalid("operations must contain 1 through 100 items"));
    }
    let item_type = graph_tools::text(&input.item_type, "item_type", 64)?;
    if !item_type
        .bytes()
        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        || item_type.contains("WHATSAPP")
    {
        return Err(invalid(
            "item_type must be a current uppercase Meta catalog item type",
        ));
    }
    let identity_fields: &[&str] = match item_type.as_str() {
        "DESTINATION" => &["destination_id"],
        "FLIGHT" => &["destination_airport", "origin_airport"],
        "HOME_LISTING" => &["home_listing_id"],
        "HOTEL" => &["hotel_id"],
        "HOTEL_ROOM" => &["hotel_retailer_id", "hotel_room_id"],
        "STORE_PRODUCT_ITEM" => &["retailer_item_id", "store_code"],
        "VEHICLE" => &["vehicle_id"],
        "VEHICLE_OFFER" => &["vehicle_offer_id"],
        _ => &["id"],
    };
    let remove = input
        .operations
        .iter()
        .any(|v| matches!(v, CatalogItemOperation::Delete { .. }));
    validate_removal_acknowledgement(remove, input.removal_acknowledgement.as_deref())?;
    let mut ids = HashSet::new();
    let mut requests = Vec::with_capacity(input.operations.len());
    for operation in input.operations {
        let (method, data, localization, metadata) = match operation {
            CatalogItemOperation::Upsert {
                data,
                localization,
                metadata,
            } => ("UPDATE", data, localization, metadata),
            CatalogItemOperation::Delete {
                data,
                localization,
                metadata,
            } => ("DELETE", data, localization, metadata),
        };
        graph_tools::json(&Value::Object(data.clone()), "data")?;
        let mut identity = Vec::new();
        for field in identity_fields {
            let id = data
                .get(*field)
                .and_then(Value::as_str)
                .ok_or_else(|| invalid(format!("data.{field} is required for {item_type}")))?;
            identity.push(graph_tools::text(id, field, 100)?);
        }
        if method == "DELETE"
            && data
                .keys()
                .any(|key| !identity_fields.contains(&key.as_str()))
        {
            return Err(invalid(
                "Delete data must contain only the catalog item's identity fields",
            ));
        }
        if data.keys().any(|key| {
            matches!(
                key.as_str(),
                "retailer_id" | "allow_upsert" | "requests" | "method" | "operation"
            )
        }) {
            return Err(invalid(
                "Use items_batch feed-format fields and keep request controls outside data",
            ));
        }
        let mut record = Map::from_iter([
            ("method".into(), json!(method)),
            ("data".into(), Value::Object(data.clone())),
        ]);
        match (input.kind, localization) {
            (CatalogBatchKind::LocalizedItems, Some(localization)) => {
                let kind = match localization.kind {
                    CatalogLocalizationKind::Language => "LANGUAGE",
                    CatalogLocalizationKind::Country => "COUNTRY",
                    CatalogLocalizationKind::LanguageAndCountry => "LANGUAGE_AND_COUNTRY",
                };
                let value = graph_tools::text(&localization.value, "localization.value", 64)?;
                if !value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'|'))
                    || (matches!(
                        localization.kind,
                        CatalogLocalizationKind::LanguageAndCountry
                    ) != value.contains('|'))
                {
                    return Err(invalid(
                        "Use the documented language/country code for localization.value",
                    ));
                }
                if matches!(localization.kind, CatalogLocalizationKind::Language)
                    && [
                        "price",
                        "sale_price",
                        "unit_price",
                        "base_price",
                        "status",
                        "availability",
                    ]
                    .iter()
                    .any(|field| data.contains_key(*field))
                {
                    return Err(invalid(
                        "Price, availability and status localization require country localization",
                    ));
                }
                identity.push(format!("{kind}:{value}"));
                record.insert("localization".into(), json!({"type":kind,"value":value}));
            }
            (CatalogBatchKind::LocalizedItems, None) => {
                return Err(invalid(
                    "localized_items requires localization for each operation",
                ));
            }
            (_, Some(_)) => return Err(invalid("localization is only valid for localized_items")),
            _ => (),
        }
        if let Some(metadata) = metadata {
            if !matches!(
                input.kind,
                CatalogBatchKind::GeolocatedItems | CatalogBatchKind::VersionItems
            ) || metadata.keys().any(|key| {
                matches!(key.as_str(), "method" | "data" | "localization")
                    || crate::bounded_json::request_control_key(key)
            }) {
                return Err(invalid(
                    "metadata is only for documented geolocated/versioned request fields; it cannot override method/data",
                ));
            }
            let encoded = graph_tools::json(&Value::Object(metadata.clone()), "metadata")?;
            identity.push(encoded);
            record.extend(metadata);
        }
        if !ids.insert(identity) {
            return Err(invalid(
                "Each item identity and localization must be unique within a batch",
            ));
        }
        requests.push(Value::Object(record));
    }
    let requests = graph_tools::json(&Value::Array(requests), "requests")?;
    let edge = match input.kind {
        CatalogBatchKind::Items => "items_batch",
        CatalogBatchKind::LocalizedItems => "localized_items_batch",
        CatalogBatchKind::GeolocatedItems => "geolocated_items_batch",
        CatalogBatchKind::VersionItems => "version_items_batch",
    };
    let mut params = vec![
        ("item_type".into(), item_type),
        ("requests".into(), requests),
    ];
    if let Some(allow_upsert) = input.allow_upsert {
        params.push(("allow_upsert".into(), allow_upsert.to_string()));
    } else if !matches!(input.kind, CatalogBatchKind::LocalizedItems) {
        params.push(("allow_upsert".into(), "true".into()));
    }
    if let Some(subtype) = input.item_sub_type {
        if !matches!(input.kind, CatalogBatchKind::Items) {
            return Err(invalid("item_sub_type is only valid for items"));
        }
        params.push((
            "item_sub_type".into(),
            graph_tools::text(&subtype, "item_sub_type", 64)?,
        ));
    }
    if let Some(version) = input.version {
        if matches!(input.kind, CatalogBatchKind::GeolocatedItems) {
            return Err(invalid("version is not available for geolocated_items"));
        }
        params.push(("version".into(), version.to_string()));
    }
    match (input.kind, input.item_version) {
        (CatalogBatchKind::VersionItems, Some(version)) => params.push((
            "item_version".into(),
            graph_tools::text(&version, "item_version", 128)?,
        )),
        (CatalogBatchKind::VersionItems, None) => {
            return Err(invalid("version_items requires item_version"));
        }
        (_, Some(_)) => return Err(invalid("item_version is only supported for version_items")),
        _ => (),
    }
    Ok((format!("{catalog}/{edge}"), params))
}

fn catalog_batch_ack(mut data: GraphData) -> Result<GraphData, PublicError> {
    let Some(handles) = data.result.get("handles").and_then(Value::as_array) else {
        return Err(crate::mutation_result::ambiguous_mutation_result(
            "Meta did not return catalog batch handles",
            "Read catalog batch status before retrying; the request may have been received",
        ));
    };
    if handles.len() > 100
        || handles.iter().any(|handle| {
            !handle
                .as_str()
                .is_some_and(|handle| !handle.is_empty() && handle.len() <= 2048)
        })
    {
        return Err(crate::mutation_result::ambiguous_mutation_result(
            "Meta returned invalid catalog batch handles",
            "Inspect catalog upload status before retrying",
        ));
    }
    let accepted = !handles.is_empty();
    let object = data
        .result
        .as_object_mut()
        .expect("handles belongs to an object");
    object.insert("accepted".into(), json!(accepted));
    object.insert(
        "processing_status".into(),
        json!(if accepted { "pending" } else { "not_ingested" }),
    );
    Ok(data)
}

fn build_batch_status(input: ReadCatalogBatchStatusInput) -> Result<(String, Params), PublicError> {
    let catalog = graph_tools::id(&input.catalog_id, "catalog_id")?;
    let handle = graph_tools::text(&input.handle, "handle", 2048)?;
    let edge = match input.kind {
        CatalogBatchStatusKind::Items => "check_batch_request_status",
        CatalogBatchStatusKind::ProductSets => "product_sets_batch",
        CatalogBatchStatusKind::HotelRooms => "hotel_rooms_batch",
        CatalogBatchStatusKind::PricingVariables => "pricing_variables_batch",
        CatalogBatchStatusKind::MarketplaceDeals => "check_marketplace_partner_deals_status",
        CatalogBatchStatusKind::MarketplaceSellers => "check_marketplace_partner_sellers_status",
    };
    let parameter = if matches!(
        input.kind,
        CatalogBatchStatusKind::MarketplaceDeals | CatalogBatchStatusKind::MarketplaceSellers
    ) {
        "session_id"
    } else {
        "handle"
    };
    let mut params = vec![(parameter.into(), handle)];
    if input.include_invalid_ids {
        if !matches!(input.kind, CatalogBatchStatusKind::Items) {
            return Err(invalid(
                "include_invalid_ids is supported only for item batches",
            ));
        }
        params.push(("load_ids_of_invalid_requests".into(), "true".into()));
    }
    Ok((format!("{catalog}/{edge}"), params))
}

fn build_ad_configuration(
    input: ConfigureCatalogAdvertisingInput,
) -> Result<(String, Params), PublicError> {
    use CatalogAdConfigurationAction::*;
    let catalog = graph_tools::id(&input.catalog_id, "catalog_id")?;
    let (edge, allowed): (&str, &[&str]) = match input.action {
        CreateCategories => ("categories", &["data"]),
        UpdateGeneratedImages => ("update_generated_image_config", &["data"]),
        SetCollaborativeImageBank => (
            "cpas_lsb_image_bank",
            &["ad_group_id", "agency_business_id", "backup_image_urls"],
        ),
        SubmitMarketplaceDeals => ("marketplace_partner_deals_details", &["requests"]),
        SubmitMarketplaceSellers => ("marketplace_partner_sellers_details", &["requests"]),
    };
    let partner_batch = matches!(
        input.action,
        SubmitMarketplaceDeals | SubmitMarketplaceSellers
    );
    validate_removal_acknowledgement(partner_batch, input.removal_acknowledgement.as_deref())?;
    match input.action {
        CreateCategories | UpdateGeneratedImages => {
            if !input
                .fields
                .get("data")
                .and_then(Value::as_array)
                .is_some_and(|items| {
                    !items.is_empty()
                        && items.len() <= 100
                        && items
                            .iter()
                            .all(|item| item.as_object().is_some_and(|map| !map.is_empty()))
                })
            {
                return Err(invalid(
                    "data must contain 1 through 100 nonempty JSON objects",
                ));
            }
        }
        SetCollaborativeImageBank => {
            require_id(&input.fields, "ad_group_id")?;
            require_id(&input.fields, "agency_business_id")?;
            let urls = input
                .fields
                .get("backup_image_urls")
                .and_then(Value::as_array)
                .filter(|items| !items.is_empty() && items.len() <= 100)
                .ok_or_else(|| {
                    invalid("backup_image_urls must contain 1 through 100 public HTTPS URLs")
                })?;
            for url in urls {
                public_url(
                    url.as_str()
                        .ok_or_else(|| invalid("backup_image_urls must contain strings"))?,
                )?;
            }
        }
        SubmitMarketplaceDeals | SubmitMarketplaceSellers => {
            if !input
                .fields
                .get("requests")
                .and_then(Value::as_object)
                .is_some_and(|map| !map.is_empty())
            {
                return Err(invalid("requests must be a nonempty JSON object"));
            }
        }
    }
    Ok((
        format!("{catalog}/{edge}"),
        graph_tools::form_fields(&input.fields, allowed)?,
    ))
}

fn build_marketplace_signal(
    input: SendMarketplaceSignalInput,
    now: u64,
) -> Result<(String, Params), PublicError> {
    let catalog = graph_tools::id(&input.catalog_id, "catalog_id")?;
    if input.event_time == 0
        || input.event_time < now.saturating_sub(7 * 86400)
        || input.event_time > now.saturating_add(300)
    {
        return Err(invalid(
            "event_time must be within the past seven days and not in the future",
        ));
    }
    let name = match input.event_name {
        MarketplaceEventName::AddToCart => "ADD_TO_CART",
        MarketplaceEventName::OfferSubmitted => "OFFER_SUBMITTED",
        MarketplaceEventName::Purchase => "PURCHASE",
        MarketplaceEventName::PurchaseViaOffer => "PURCHASE_VIA_OFFER",
        MarketplaceEventName::Test => "TEST",
        MarketplaceEventName::ViewItem => "VIEW_ITEM",
    };
    let conversion = match input.conversion_type {
        MarketplaceConversionType::Attributed => "ATTRIBUTED",
        MarketplaceConversionType::InSession => "IN_SESSION",
    };
    let mut params = vec![
        ("event_name".into(), name.into()),
        ("conversion_type".into(), conversion.into()),
        (
            "event_id".into(),
            graph_tools::text(&input.event_id, "event_id", 100)?,
        ),
        ("event_time".into(), input.event_time.to_string()),
        (
            "user_data".into(),
            graph_tools::json(
                &crate::capi::normalized_user_data_json(input.user_data)?,
                "user_data",
            )?,
        ),
    ];
    if let Some(url) = input.event_source_url {
        params.push(("event_source_url".into(), public_url(&url)?));
    }
    for (key, data) in [
        ("offer_data", input.offer_data),
        ("order_data", input.order_data),
    ] {
        if let Some(data) = data {
            let value = Value::Object(data);
            let encoded = graph_tools::json(&value, key)?;
            reject_matching_metadata(&value)?;
            params.push((key.into(), encoded));
        }
    }
    Ok((format!("{catalog}/marketplace_partner_signals"), params))
}

// Keep unhashed matching fields out of extensible offer/order metadata.
fn reject_matching_metadata(value: &Value) -> Result<(), PublicError> {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if matches!(
                    key.to_ascii_lowercase().as_str(),
                    "user_data"
                        | "email"
                        | "emails"
                        | "phone"
                        | "phones"
                        | "em"
                        | "ph"
                        | "first_name"
                        | "last_name"
                        | "fn"
                        | "ln"
                        | "external_id"
                        | "client_ip_address"
                        | "client_user_agent"
                        | "fbc"
                        | "fbp"
                        | "address"
                        | "billing_address"
                        | "shipping_address"
                ) {
                    return Err(invalid("Customer matching data belongs in typed user_data"));
                }
                reject_matching_metadata(value)?;
            }
        }
        Value::Array(items) => {
            for value in items {
                reject_matching_metadata(value)?;
            }
        }
        Value::String(value) if value.trim_start().starts_with(['{', '[']) => {
            return Err(invalid("Use JSON objects instead of encoded metadata"));
        }
        _ => (),
    }
    Ok(())
}

fn value_id(value: &Value, field: &str) -> Result<String, PublicError> {
    match value {
        Value::String(id) => graph_tools::id(id, field),
        Value::Number(id) if id.is_u64() => graph_tools::id(&id.to_string(), field),
        _ => Err(invalid(format!("{field} must be a numeric Meta ID"))),
    }
}

fn require_id(fields: &Map<String, Value>, field: &str) -> Result<String, PublicError> {
    value_id(
        fields
            .get(field)
            .ok_or_else(|| invalid(format!("{field} is required")))?,
        field,
    )
}

fn validate_structured_fields(fields: &Map<String, Value>) -> Result<(), PublicError> {
    for key in [
        "schedule",
        "update_schedule",
        "upload_schedule",
        "filter",
        "params",
        "utm_settings",
    ] {
        if let Some(value) = fields.get(key) {
            if !value.is_object() {
                return Err(invalid(format!(
                    "{key} must be a JSON object, not an encoded string"
                )));
            }
            graph_tools::json(value, key)?;
        }
    }
    for key in ["schedule", "update_schedule", "upload_schedule"] {
        if let Some(schedule) = fields.get(key).and_then(Value::as_object) {
            if schedule.contains_key("username") {
                return Err(invalid(
                    "Feed authentication must stay outside tool arguments; use a public HTTPS feed URL",
                ));
            }
            if let Some(url) = schedule.get("url") {
                public_url(
                    url.as_str()
                        .ok_or_else(|| invalid("schedule url must be a string"))?,
                )?;
            }
        }
    }
    Ok(())
}

fn public_url(raw: &str) -> Result<String, PublicError> {
    let text = graph_tools::text(raw, "url", 2048)?;
    let url = reqwest::Url::parse(&text).map_err(|_| invalid("url must be a public HTTPS URL"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid(
            "url must be credential-free HTTPS without a fragment",
        ));
    }
    graph_tools::json(&json!({"url":text}), "url")?;
    Ok(text)
}

fn write_contract(
    kind: CatalogObjectKind,
    operation: CatalogWriteOperation,
) -> Result<(&'static str, &'static [&'static str]), PublicError> {
    use CatalogObjectKind::*;
    use CatalogWriteOperation::*;
    let contract = match (kind, operation) {
        // SDK 26.0.0 business.py:3672 (create_owned_product_catalog).
        (Catalog, Create) => (
            "owned_product_catalogs",
            &[
                "additional_vertical_option",
                "business_metadata",
                "catalog_segment_filter",
                "catalog_segment_product_set_id",
                "da_display_settings",
                "destination_catalog_settings",
                "flight_catalog_settings",
                "name",
                "parent_catalog_id",
                "partner_integration",
                "store_catalog_settings",
                "vertical",
            ] as &[&str],
        ),
        // SDK 26.0.0 productcatalog.py:231 (api_update).
        (Catalog, Update) => (
            "",
            &[
                "additional_vertical_option",
                "da_display_settings",
                "default_image_url",
                "destination_catalog_settings",
                "fallback_image_url",
                "flight_catalog_settings",
                "name",
                "partner_integration",
                "store_catalog_settings",
            ] as &[&str],
        ),
        // SDK 26.0.0 productcatalog.py:1879 (create_product_set).
        (ProductSet, Create) => (
            "product_sets",
            &[
                "filter",
                "metadata",
                "name",
                "ordering_info",
                "publish_to_shops",
                "retailer_id",
            ] as &[&str],
        ),
        // SDK 26.0.0 productset.py:114 (api_update).
        (ProductSet, Update) => (
            "",
            &[
                "filter",
                "metadata",
                "name",
                "ordering_info",
                "publish_to_shops",
                "retailer_id",
            ] as &[&str],
        ),
        // SDK 26.0.0 productcatalog.py:1986 (create_product).
        (Product, Create) => (
            "products",
            &[
                "additional_image_urls",
                "additional_variant_attributes",
                "age_group",
                "allow_upsert",
                "android_app_name",
                "android_class",
                "android_package",
                "android_url",
                "availability",
                "brand",
                "category",
                "category_specific_fields",
                "checkout_url",
                "color",
                "commerce_tax_category",
                "condition",
                "currency",
                "custom_data",
                "custom_label_0",
                "custom_label_1",
                "custom_label_2",
                "custom_label_3",
                "custom_label_4",
                "custom_number_0",
                "custom_number_1",
                "custom_number_2",
                "custom_number_3",
                "custom_number_4",
                "description",
                "expiration_date",
                "fb_product_category",
                "gender",
                "gtin",
                "image_url",
                "importer_address",
                "importer_name",
                "inventory",
                "ios_app_name",
                "ios_app_store_id",
                "ios_url",
                "ipad_app_name",
                "ipad_app_store_id",
                "ipad_url",
                "iphone_app_name",
                "iphone_app_store_id",
                "iphone_url",
                "launch_date",
                "live_special_price",
                "manufacturer_info",
                "manufacturer_part_number",
                "marked_for_product_launch",
                "material",
                "mobile_link",
                "name",
                "ordering_index",
                "origin_country",
                "pattern",
                "price",
                "product_priority_0",
                "product_priority_1",
                "product_priority_2",
                "product_priority_3",
                "product_priority_4",
                "product_type",
                "quantity_to_sell_on_facebook",
                "retailer_id",
                "retailer_product_group_id",
                "return_policy_days",
                "rich_text_description",
                "sale_price",
                "sale_price_end_date",
                "sale_price_start_date",
                "short_description",
                "size",
                "start_date",
                "url",
                "visibility",
                "windows_phone_app_id",
                "windows_phone_app_name",
                "windows_phone_url",
            ] as &[&str],
        ),
        // SDK 26.0.0 productitem.py:957 (api_update).
        (Product, Update) => (
            "",
            &[
                "additional_image_urls",
                "additional_variant_attributes",
                "age_group",
                "android_app_name",
                "android_class",
                "android_package",
                "android_url",
                "availability",
                "brand",
                "category",
                "category_specific_fields",
                "checkout_url",
                "color",
                "commerce_tax_category",
                "condition",
                "currency",
                "custom_data",
                "custom_label_0",
                "custom_label_1",
                "custom_label_2",
                "custom_label_3",
                "custom_label_4",
                "custom_number_0",
                "custom_number_1",
                "custom_number_2",
                "custom_number_3",
                "custom_number_4",
                "description",
                "expiration_date",
                "fb_product_category",
                "gender",
                "gtin",
                "image_url",
                "importer_address",
                "importer_name",
                "inventory",
                "ios_app_name",
                "ios_app_store_id",
                "ios_url",
                "ipad_app_name",
                "ipad_app_store_id",
                "ipad_url",
                "iphone_app_name",
                "iphone_app_store_id",
                "iphone_url",
                "launch_date",
                "live_special_price",
                "manufacturer_info",
                "manufacturer_part_number",
                "marked_for_product_launch",
                "material",
                "mobile_link",
                "name",
                "ordering_index",
                "origin_country",
                "pattern",
                "price",
                "product_priority_0",
                "product_priority_1",
                "product_priority_2",
                "product_priority_3",
                "product_priority_4",
                "product_type",
                "quantity_to_sell_on_facebook",
                "retailer_id",
                "return_policy_days",
                "rich_text_description",
                "sale_price",
                "sale_price_end_date",
                "sale_price_start_date",
                "short_description",
                "size",
                "start_date",
                "url",
                "visibility",
                "windows_phone_app_id",
                "windows_phone_app_name",
                "windows_phone_url",
            ] as &[&str],
        ),
        // SDK 26.0.0 productcatalog.py:1811 (create_product_group).
        (ProductGroup, Create) => ("product_groups", &["retailer_id", "variants"] as &[&str]),
        // SDK 26.0.0 productgroup.py:110 (api_update).
        (ProductGroup, Update) => ("", &["default_product_id", "variants"] as &[&str]),
        // SDK 26.0.0 productcatalog.py:1721 (create_product_feed).
        (Feed, Create) => (
            "product_feeds",
            &[
                "country",
                "default_currency",
                "deletion_enabled",
                "delimiter",
                "encoding",
                "feed_type",
                "file_name",
                "ingestion_source_type",
                "item_sub_type",
                "migrated_from_feed_id",
                "name",
                "override_type",
                "override_value",
                "primary_feed_ids",
                "quoted_fields_mode",
                "rules",
                "schedule",
                "selected_override_fields",
                "update_schedule",
                "use_case",
            ] as &[&str],
        ),
        // SDK 26.0.0 productfeed.py:215 (api_update).
        (Feed, Update) => (
            "",
            &[
                "default_currency",
                "deletion_enabled",
                "delimiter",
                "encoding",
                "migrated_from_feed_id",
                "name",
                "quoted_fields_mode",
                "schedule",
                "update_schedule",
            ] as &[&str],
        ),
        // SDK 26.0.0 productfeed.py:490 (create_rule).
        (FeedRule, Create) => ("rules", &["attribute", "params", "rule_type"] as &[&str]),
        // SDK 26.0.0 productfeedrule.py:102 (api_update).
        (FeedRule, Update) => ("", &["params"] as &[&str]),
        // SDK 26.0.0 productfeed.py:587 (create_upload_schedule).
        (FeedSchedule, Create) => ("upload_schedules", &["upload_schedule"] as &[&str]),
        // SDK 26.0.0 productcatalog.py:1558 (create_media_title).
        (MediaTitle, Create) => (
            "media_titles",
            &[
                "additional_image_urls",
                "android_app_name",
                "android_class",
                "android_package",
                "android_url",
                "awards",
                "cast",
                "category",
                "currency",
                "description",
                "director",
                "fb_product_category",
                "genre",
                "image_url",
                "ios_app_name",
                "ios_app_store_id",
                "ios_url",
                "ipad_app_name",
                "ipad_app_store_id",
                "ipad_url",
                "iphone_app_name",
                "iphone_app_store_id",
                "iphone_url",
                "media_category",
                "name",
                "price",
                "rating",
                "release_date",
                "retailer_id",
                "url",
                "windows_phone_app_id",
                "windows_phone_app_name",
                "windows_phone_url",
            ] as &[&str],
        ),
        // SDK 26.0.0 productcatalog.py:1342 (create_hotel).
        (Hotel, Create) => (
            "hotels",
            &[
                "address",
                "applinks",
                "base_price",
                "brand",
                "currency",
                "description",
                "guest_ratings",
                "hotel_id",
                "images",
                "name",
                "phone",
                "star_rating",
                "url",
            ] as &[&str],
        ),
        // SDK 26.0.0 hotel.py:152 (api_update).
        (Hotel, Update) => (
            "",
            &[
                "address",
                "applinks",
                "base_price",
                "brand",
                "currency",
                "description",
                "guest_ratings",
                "images",
                "name",
                "phone",
                "star_rating",
                "url",
            ] as &[&str],
        ),
        // SDK 26.0.0 flight.py:107 (api_update).
        (Flight, Update) => (
            "",
            &[
                "currency",
                "description",
                "destination_airport",
                "destination_city",
                "images",
                "origin_airport",
                "origin_city",
                "price",
                "url",
            ] as &[&str],
        ),
        // SDK 26.0.0 productcatalog.py:2204 (create_vehicle).
        (Vehicle, Create) => (
            "vehicles",
            &[
                "address",
                "applinks",
                "availability",
                "body_style",
                "condition",
                "currency",
                "date_first_on_lot",
                "dealer_id",
                "dealer_name",
                "dealer_phone",
                "description",
                "drivetrain",
                "exterior_color",
                "fb_page_id",
                "fuel_type",
                "images",
                "interior_color",
                "make",
                "mileage",
                "model",
                "price",
                "state_of_vehicle",
                "title",
                "transmission",
                "trim",
                "url",
                "vehicle_id",
                "vehicle_type",
                "vin",
                "year",
            ] as &[&str],
        ),
        // SDK 26.0.0 vehicle.py:228 (api_update).
        (Vehicle, Update) => (
            "",
            &[
                "address",
                "applinks",
                "availability",
                "body_style",
                "condition",
                "currency",
                "date_first_on_lot",
                "dealer_id",
                "dealer_name",
                "dealer_phone",
                "description",
                "drivetrain",
                "exterior_color",
                "fb_page_id",
                "fuel_type",
                "images",
                "interior_color",
                "make",
                "mileage",
                "model",
                "price",
                "state_of_vehicle",
                "title",
                "transmission",
                "trim",
                "url",
                "vehicle_type",
                "vin",
                "year",
            ] as &[&str],
        ),
        // SDK 26.0.0 productcatalog.py:1193 (create_home_listing).
        (HomeListing, Create) => (
            "home_listings",
            &[
                "address",
                "availability",
                "currency",
                "description",
                "home_listing_id",
                "images",
                "listing_type",
                "name",
                "num_baths",
                "num_beds",
                "num_units",
                "price",
                "property_type",
                "url",
                "year_built",
            ] as &[&str],
        ),
        // SDK 26.0.0 homelisting.py:171 (api_update).
        (HomeListing, Update) => (
            "",
            &[
                "address",
                "availability",
                "currency",
                "description",
                "images",
                "listing_type",
                "name",
                "num_baths",
                "num_beds",
                "num_units",
                "price",
                "property_type",
                "url",
                "year_built",
            ] as &[&str],
        ),
        _ => {
            return Err(invalid(
                "This resource has no direct create/update action; use a catalog item batch, feed, or the owning feed schedule fields",
            ));
        }
    };
    Ok(contract)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::MetaConfig, safety::REMOVAL_ACKNOWLEDGEMENT};
    use std::time::Duration;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    fn write_input(
        kind: CatalogObjectKind,
        operation: CatalogWriteOperation,
        fields: Value,
    ) -> WriteCatalogObjectInput {
        WriteCatalogObjectInput {
            kind,
            operation,
            target_id: "123".into(),
            fields: fields.as_object().unwrap().clone(),
            removal_acknowledgement: None,
        }
    }

    #[test]
    fn supplemental_ad_contracts_preserve_partner_sessions_and_hash_matching_data() {
        let request: ConfigureCatalogAdvertisingInput = serde_json::from_value(json!({"catalog_id":"123","action":"set_collaborative_image_bank","fields":{"ad_group_id":"456","agency_business_id":"789","backup_image_urls":["https://example.com/image.jpg"]}})).unwrap();
        assert_eq!(
            build_ad_configuration(request).unwrap().0,
            "123/cpas_lsb_image_bank"
        );
        for (action, edge) in [
            (
                "submit_marketplace_deals",
                "marketplace_partner_deals_details",
            ),
            (
                "submit_marketplace_sellers",
                "marketplace_partner_sellers_details",
            ),
        ] {
            let mut input = json!({"catalog_id":"123","action":action,"fields":{"requests":{"DELETE":["456"]}}});
            assert!(
                build_ad_configuration(serde_json::from_value(input.clone()).unwrap()).is_err()
            );
            input["removal_acknowledgement"] = json!(REMOVAL_ACKNOWLEDGEMENT);
            assert_eq!(
                build_ad_configuration(serde_json::from_value(input).unwrap())
                    .unwrap()
                    .0,
                format!("123/{edge}")
            );
        }
        let (path, params) = build_batch_status(ReadCatalogBatchStatusInput {
            catalog_id: "123".into(),
            kind: CatalogBatchStatusKind::MarketplaceDeals,
            handle: "session-456".into(),
            include_invalid_ids: false,
        })
        .unwrap();
        assert_eq!(path, "123/check_marketplace_partner_deals_status");
        assert_eq!(params, vec![("session_id".into(), "session-456".into())]);
        let signal = json!({"catalog_id":"123","event_name":"PURCHASE","conversion_type":"ATTRIBUTED","event_id":"order-1","event_time":1700000000_u64,"user_data":{"emails":["Person@Example.com"]},"order_data":{"order_id":"order-1"}});
        let (path, params) =
            build_marketplace_signal(serde_json::from_value(signal.clone()).unwrap(), 1700000000)
                .unwrap();
        assert_eq!(path, "123/marketplace_partner_signals");
        let user_data: Value =
            serde_json::from_str(&params.iter().find(|(key, _)| key == "user_data").unwrap().1)
                .unwrap();
        let hash = user_data["em"][0].as_str().unwrap();
        assert_eq!(hash.len(), 64);
        assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(!format!("{params:?}").contains("Person@Example.com"));
        let mut unsafe_signal = signal.clone();
        unsafe_signal["order_data"]["email"] = json!("person@example.com");
        assert!(
            build_marketplace_signal(serde_json::from_value(unsafe_signal).unwrap(), 1700000000)
                .is_err()
        );
        assert!(
            build_marketplace_signal(
                serde_json::from_value(signal).unwrap(),
                1700000000 + 8 * 86400
            )
            .is_err()
        );
        let local =
            json!({"kind":"feed","parent_id":"123","local_path":"feed.csv","local_format":"csv"});
        let (_, params) = build_upload(&serde_json::from_value(local.clone()).unwrap()).unwrap();
        assert_eq!(params, vec![("update_only".into(), "true".into())]);
        let mut both = local;
        both["url"] = json!("https://example.com/feed.csv");
        assert!(build_upload(&serde_json::from_value(both).unwrap()).is_err());
    }

    #[test]
    fn catalog_contracts_use_business_and_feed_edges_without_guessing_node_actions() {
        let (path, params) = build_write(write_input(
            CatalogObjectKind::Catalog,
            CatalogWriteOperation::Create,
            json!({"name":"Store","vertical":"commerce"}),
        ))
        .unwrap();
        assert_eq!(path, "123/owned_product_catalogs");
        assert!(params.contains(&("vertical".into(), "commerce".into())));
        let (path,params)=build_write(write_input(CatalogObjectKind::Feed,CatalogWriteOperation::Create,json!({"name":"Inventory","schedule":{"interval":"DAILY","hour":3,"url":"https://example.com/feed.csv"}}))).unwrap();
        assert_eq!(path, "123/product_feeds");
        let schedule = params.iter().find(|(key, _)| key == "schedule").unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&schedule.1).unwrap()["hour"],
            3
        );
        assert!(
            write_contract(
                CatalogObjectKind::FeedSchedule,
                CatalogWriteOperation::Update
            )
            .is_err()
        );
        assert!(
            write_contract(
                CatalogObjectKind::Destination,
                CatalogWriteOperation::Create
            )
            .is_err()
        );
        let (path, params) = build_batch_status(ReadCatalogBatchStatusInput {
            catalog_id: "123".into(),
            kind: CatalogBatchStatusKind::Items,
            handle: "batch-42".into(),
            include_invalid_ids: true,
        })
        .unwrap();
        assert_eq!(path, "123/check_batch_request_status");
        assert!(params.contains(&("load_ids_of_invalid_requests".into(), "true".into())));
    }

    #[test]
    fn credentials_and_removal_paths_fail_before_graph_requests() {
        for fields in [
            json!({"schedule":"{\"password\":\"hidden\"}"}),
            json!({"schedule":{"url":"https://user:pass@example.com/feed"}}),
            json!({"schedule":{"url":"https://example.com/feed?access_token=hidden"}}),
            json!({"schedule":{"username":"private"}}),
            json!({"schedule":{"password":"private"}}),
            json!({"method":"DELETE"}),
        ] {
            assert!(
                build_write(write_input(
                    CatalogObjectKind::Feed,
                    CatalogWriteOperation::Update,
                    fields
                ))
                .is_err()
            );
        }
        let mut removal = write_input(
            CatalogObjectKind::Feed,
            CatalogWriteOperation::Update,
            json!({"deletion_enabled":true}),
        );
        assert!(
            build_write(write_input(
                CatalogObjectKind::Feed,
                CatalogWriteOperation::Update,
                json!({"deletion_enabled":true})
            ))
            .is_err()
        );
        removal.removal_acknowledgement = Some(REMOVAL_ACKNOWLEDGEMENT.into());
        assert!(build_write(removal).is_ok());
        for id in ["1/products", "act_1", "https://example.com"] {
            assert!(
                build_list(ListCatalogResourcesInput {
                    collection: CatalogCollection::Products,
                    parent_id: id.into(),
                    options: ReadOptions::default(),
                    query: Map::new()
                })
                .is_err()
            );
        }
        let input:UploadCatalogFeedInput=serde_json::from_value(json!({"kind":"feed","parent_id":"123","url":"https://example.com/feed","update_only":false})).unwrap();
        assert!(build_upload(&input).is_err());
    }

    #[test]
    fn item_and_localized_batches_follow_reference_records_and_acknowledgements() {
        let batch = json!({"catalog_id":"123","kind":"items","item_type":"PRODUCT_ITEM","operations":[{"operation":"delete","data":{"id":"SKU-1"}}],"removal_acknowledgement":REMOVAL_ACKNOWLEDGEMENT});
        let (_, params) = build_batch(serde_json::from_value(batch).unwrap()).unwrap();
        let records: Value =
            serde_json::from_str(&params.iter().find(|(key, _)| key == "requests").unwrap().1)
                .unwrap();
        assert_eq!(records, json!([{"method":"DELETE","data":{"id":"SKU-1"}}]));
        let mut localized = json!({"catalog_id":"123","kind":"localized_items","item_type":"PRODUCT_ITEM","operations":[{"operation":"upsert","data":{"id":"SKU-1","title":"Produkt"},"localization":{"type":"LANGUAGE","value":"pl_PL"}}]});
        let (path, params) =
            build_batch(serde_json::from_value(localized.clone()).unwrap()).unwrap();
        assert_eq!(path, "123/localized_items_batch");
        assert!(!params.iter().any(|(key, _)| key == "allow_upsert"));
        let records: Value =
            serde_json::from_str(&params.iter().find(|(key, _)| key == "requests").unwrap().1)
                .unwrap();
        assert_eq!(
            records,
            json!([{"method":"UPDATE","data":{"id":"SKU-1","title":"Produkt"},"localization":{"type":"LANGUAGE","value":"pl_PL"}}])
        );
        localized["operations"][0]["data"]["price"] = json!("10 PLN");
        assert!(build_batch(serde_json::from_value(localized).unwrap()).is_err());
        let empty=catalog_batch_ack(GraphData{result:json!({"handles":[],"validation_status":[{"retailer_id":"SKU-1","errors":[{"message":"invalid price"}]}]}),next_cursor:None}).unwrap();
        assert_eq!(empty.result["accepted"], false);
        assert_eq!(empty.result["processing_status"], "not_ingested");
        assert_eq!(
            empty.result["validation_status"][0]["errors"][0]["message"],
            "invalid price"
        );
        let pending = catalog_batch_ack(GraphData {
            result: json!({"handles":["handle-1"],"validation_status":[]}),
            next_cursor: None,
        })
        .unwrap();
        assert_eq!(pending.result["accepted"], true);
        assert_eq!(pending.result["processing_status"], "pending");
        assert!(
            catalog_batch_ack(GraphData {
                result: json!({"success":true}),
                next_cursor: None
            })
            .is_err()
        );
    }

    #[test]
    fn vertical_batches_are_bounded_and_preserve_explicit_operation_semantics() {
        let valid = json!({"catalog_id":"123","kind":"items","item_type":"VEHICLE","operations":[{"operation":"upsert","data":{"vehicle_id":"CAR-42","make":"Volvo","model":"XC60","mileage":{"value":120,"unit":"KM"}}}]});
        let (path, params) = build_batch(serde_json::from_value(valid.clone()).unwrap()).unwrap();
        assert_eq!(path, "123/items_batch");
        let requests: Value =
            serde_json::from_str(&params.iter().find(|(key, _)| key == "requests").unwrap().1)
                .unwrap();
        assert_eq!(requests[0]["method"], "UPDATE");
        assert_eq!(requests[0]["data"]["vehicle_id"], "CAR-42");
        assert!(requests[0].get("retailer_id").is_none());
        assert_eq!(requests[0]["data"]["mileage"]["unit"], "KM");
        let mut duplicate = valid.clone();
        duplicate["operations"]
            .as_array_mut()
            .unwrap()
            .push(valid["operations"][0].clone());
        assert!(build_batch(serde_json::from_value(duplicate).unwrap()).is_err());
        for bad_data in [
            json!({"access_token":"secret"}),
            json!({"image_url":"https://example.com/image?api_key=secret"}),
        ] {
            let mut bad = valid.clone();
            bad["operations"][0]["data"] = bad_data;
            assert!(build_batch(serde_json::from_value(bad).unwrap()).is_err());
        }
        let mut delete = valid;
        delete["operations"] = json!([{"operation":"delete","data":{"vehicle_id":"CAR-42"}}]);
        assert!(build_batch(serde_json::from_value(delete.clone()).unwrap()).is_err());
        delete["removal_acknowledgement"] = json!(REMOVAL_ACKNOWLEDGEMENT);
        assert!(build_batch(serde_json::from_value(delete).unwrap()).is_ok());
    }

    #[tokio::test]
    async fn catalog_agency_changes_reject_business_ids_before_mutation() {
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
            assert!(
                String::from_utf8_lossy(&bytes)
                    .starts_with("GET /123?fields=id%2Cvertical%2Cfeed_count")
            );
            let body = r#"{"id":"123","vertical":"OTHER","verification_status":"verified"}"#;
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
        let input=serde_json::from_value(json!({"action":"remove_agency","parent_id":"123","fields":{"business":"456"},"removal_acknowledgement":REMOVAL_ACKNOWLEDGEMENT})).unwrap();
        assert!(matches!(
            manage_connection(&graph, input).await,
            ToolResponse::Error { .. }
        ));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn direct_deletion_checks_resource_kind_then_sends_one_request() {
        for (identity, expected_requests) in [
            (r#"{"id":"123","name":"Ad"}"#, 1),
            (r#"{"id":"123","vertical":"OTHER"}"#, 1),
            (r#"{"id":"123","vertical":"commerce","feed_count":0}"#, 2),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let mut requests = Vec::new();
                for index in 0..expected_requests {
                    let (mut socket, _) =
                        tokio::time::timeout(Duration::from_secs(5), listener.accept())
                            .await
                            .unwrap()
                            .unwrap();
                    let mut bytes = Vec::new();
                    let mut chunk = [0_u8; 2048];
                    while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                        let n = socket.read(&mut chunk).await.unwrap();
                        assert!(n > 0);
                        bytes.extend_from_slice(&chunk[..n]);
                    }
                    requests.push(String::from_utf8(bytes).unwrap());
                    let body = if index == 0 {
                        identity
                    } else {
                        r#"{"success":true}"#
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    socket.write_all(response.as_bytes()).await.unwrap();
                }
                assert!(
                    tokio::time::timeout(Duration::from_millis(100), listener.accept())
                        .await
                        .is_err()
                );
                requests
            });
            let graph = GraphClient::new(&MetaConfig::for_test(
                format!("http://{address}"),
                Some("test-access-token-1234567890"),
            ))
            .unwrap();
            let result = delete_catalog(
                &graph,
                DeleteCatalogObjectInput {
                    kind: CatalogObjectKind::Catalog,
                    object_id: "123".into(),
                    allow_live_product_set_deletion: false,
                    removal_acknowledgement: REMOVAL_ACKNOWLEDGEMENT.into(),
                },
            )
            .await;
            assert_eq!(
                matches!(result, ToolResponse::Error { .. }),
                expected_requests == 1
            );
            let requests = server.await.unwrap();
            assert!(requests[0].starts_with("GET /123?fields=id%2Cvertical%2Cfeed_count HTTP/1.1"));
            if expected_requests == 2 {
                assert!(requests[1].starts_with(
                    "DELETE /123?allow_delete_catalog_with_live_product_set=false HTTP/1.1"
                ));
            }
        }
    }
}
