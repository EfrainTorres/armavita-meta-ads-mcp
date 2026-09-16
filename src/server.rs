use rmcp::{
    Json, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Implementation, ServerCapabilities, ServerInfo, Tool},
    tool, tool_handler, tool_router,
};

use crate::{
    MetaConfig,
    account_controls::{
        AccountControlsList, AccountControlsUpdate, GetAccountControlsInput,
        UpdateAccountControlsInput, get_account_controls, update_account_controls,
    },
    accounts::{
        AdAccount, AdAccountList, ListAdAccountsInput, ReadAdAccountInput, list_ad_accounts,
        read_ad_account,
    },
    ad_mutations::{
        CloneAdInput, ClonedAd, CreateAdCreativeInput, CreateAdInput, CreatedAd, CreatedAdCreative,
        UpdateAdCreativeInput, UpdateAdInput, UpdatedAd, UpdatedAdCreative, clone_ad, create_ad,
        create_ad_creative, update_ad, update_ad_creative,
    },
    ads::{Ad, AdList, ListAdsInput, ReadAdInput, list_ads, read_ad},
    ads_archive::{ArchivedAdList, SearchAdsArchiveInput, search_ads_archive},
    adset_mutations::{
        CloneAdSetInput, ClonedAdSet, CreateAdSetInput, CreatedAdSet, UpdateAdSetInput,
        UpdatedAdSet, clone_ad_set, create_ad_set, update_ad_set,
    },
    adsets::{AdSet, AdSetList, ListAdSetsInput, ReadAdSetInput, list_ad_sets, read_ad_set},
    audience_estimate::{AudienceSizeEstimate, EstimateAudienceSizeInput, estimate_audience_size},
    audience_mutations::{
        CreateCustomAudienceInput, CreateLookalikeAudienceInput, CreatedCustomAudience,
        CustomAudienceMutationAck, DeleteCustomAudienceInput, ManageCustomAudienceUsersInput,
        ManagedAudienceUsers, UpdateCustomAudienceInput, create_custom_audience,
        create_lookalike_audience, delete_custom_audience, manage_custom_audience_users,
        update_custom_audience,
    },
    audiences::{
        CustomAudience, CustomAudienceList, ListCustomAudiencesInput, ReadCustomAudienceInput,
        list_custom_audiences, read_custom_audience,
    },
    auxiliary_mutations::{
        CreateReachFrequencyPredictionInput, CreateThreadsAccountInput,
        CreatedReachFrequencyPrediction, CreatedThreadsAccount,
        GrantBrandedContentAdPermissionInput, GrantedBrandedContentAdPermission,
        RevokeBrandedContentAdPermissionInput, RevokedBrandedContentAdPermission,
        create_reach_frequency_prediction, create_threads_account,
        grant_branded_content_ad_permission, revoke_branded_content_ad_permission,
    },
    auxiliary_reads::{
        BrandedContentPermissionPage, DerivedMetricPage, GetThreadsAccountInput,
        ListAdCustomDerivedMetricsInput, ListBrandedContentAdPermissionsInput,
        ListReachFrequencyPredictionsInput, ListRecommendationsInput, ReachFrequencyPrediction,
        ReachFrequencyPredictionPage, ReadReachFrequencyPredictionInput, RecommendationPage,
        ThreadsAccount, get_threads_account, list_ad_custom_derived_metrics,
        list_branded_content_ad_permissions, list_reach_frequency_predictions,
        list_recommendations, read_reach_frequency_prediction,
    },
    campaign_mutations::{
        CloneCampaignInput, ClonedCampaign, CreateCampaignBudgetScheduleInput, CreateCampaignInput,
        CreatedCampaign, CreatedCampaignBudgetSchedule, UpdateCampaignInput, UpdatedCampaign,
        clone_campaign, create_campaign, create_campaign_budget_schedule, update_campaign,
    },
    campaigns::{
        Campaign, CampaignList, ListCampaignsInput, ReadCampaignInput, list_campaigns,
        read_campaign,
    },
    capi::{SendCapiEventsInput, SentCapiEvents, send_capi_events},
    commerce_mutations::{
        AcceptedProductBatch, BatchProductsInput, UpsertProductInput, UpsertedProduct,
        batch_products, upsert_product,
    },
    commerce_reads::{
        BusinessDatasetList, CustomConversion, CustomConversionList, ListBusinessDatasetsInput,
        ListCustomConversionsInput, ListProductCatalogsInput, ListProductSetsInput,
        ListProductsInput, ProductCatalogList, ProductList, ProductSetList,
        ReadCustomConversionInput, list_business_datasets, list_custom_conversions,
        list_product_catalogs, list_product_sets, list_products, read_custom_conversion,
    },
    creatives::{
        AdCreative, AdCreativeList, AdImageList, AdVideoList, ListAdCreativesInput,
        ListAdImagesInput, ListAdVideosInput, ReadAdCreativeInput, list_ad_creatives,
        list_ad_images, list_ad_videos, read_ad_creative,
    },
    custom_conversion_mutations::{
        CreateCustomConversionInput, CreatedCustomConversion, DeleteCustomConversionInput,
        DeletedCustomConversion, UpdateCustomConversionInput, UpdatedCustomConversion,
        create_custom_conversion, delete_custom_conversion, update_custom_conversion,
    },
    dataset_quality::{DatasetQuality, ReadDatasetQualityInput, read_dataset_quality},
    error::{StartupError, ToolResponse},
    graph::GraphClient,
    insights::{
        CreateInsightsJobInput, CreatedInsightsJob, InsightPage, InsightsJob, ListInsightsInput,
        ReadInsightsJobInput, ReadInsightsJobResultsInput, create_insights_job, list_insights,
        read_insights_job, read_insights_job_results,
    },
    media_read::{ReadAdImageInput, read_ad_image},
    media_uploads::{
        UploadAdImageAssetInput, UploadAdVideoAssetInput, UploadedAdImageAsset,
        UploadedAdVideoAsset, upload_ad_image_asset, upload_ad_video_asset,
    },
    mutation_plan::{
        AppliedMutationPlan, ApplyMutationPlanInput, BuildMutationPlanInput,
        DiscardMutationPlanInput, DiscardedMutationPlan, GetMutationPlanInput, MutationPlanReceipt,
        MutationPlanStore, MutationPlanView, apply_mutation_plan, build_mutation_plan,
        discard_mutation_plan, get_mutation_plan,
    },
    pages::{ListPagesInput, PageList, list_pages},
    previews::{AdPreviewList, ListAdPreviewsInput, list_ad_previews},
    reports::{CreateReportInput, PerformanceReport, create_report},
    targeting_search::{
        GeoLocationPage, InterestPage, SearchBehaviorsInput, SearchDemographicsInput,
        SearchGeoLocationsInput, SearchInterestsInput, SuggestInterestsInput,
        TargetingCategoryPage, search_behaviors, search_demographics, search_geo_locations,
        search_interests, suggest_interests,
    },
};

const SERVER_INSTRUCTIONS: &str = "Use this server for Meta Ads delivery, audiences, reports, media, catalogs, CAPI, partnerships, reach/frequency, and Threads. Keep credentials outside tools; never place access tokens in prompts. Prefer typed tools. For uncommon v26 writes, build and review one frozen request; plans are not atomic. Apply needs APPLY_LIVE_META_ADS_CHANGES; removals need CONFIRM_META_ADS_REMOVALS. Writes are one-shot: reconcile unknown outcomes before retrying. Read current state first; page with next_cursor only as needed.";

#[derive(Debug, Clone)]
pub struct MetaAdsServer {
    tool_router: ToolRouter<Self>,
    graph: GraphClient,
    media_root: Option<std::path::PathBuf>,
    plans: MutationPlanStore,
}

impl MetaAdsServer {
    pub fn new(config: MetaConfig) -> Result<Self, StartupError> {
        let graph = GraphClient::new(&config)?;
        Ok(Self {
            tool_router: Self::tool_router(),
            graph,
            media_root: config.media_root,
            plans: MutationPlanStore::new(),
        })
    }

    pub fn tool_definitions(&self) -> Vec<Tool> {
        self.tool_router.list_all()
    }
}

#[tool_router]
impl MetaAdsServer {
    #[tool(
        name = "get_account_controls",
        description = "Read account-wide audience, placement, age, and campaign-error controls.",
        annotations(
            title = "Read Meta account controls",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn get_account_controls(
        &self,
        Parameters(input): Parameters<GetAccountControlsInput>,
    ) -> Result<Json<ToolResponse<AccountControlsList>>, Json<ToolResponse<AccountControlsList>>>
    {
        get_account_controls(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "update_account_controls",
        description = "Set one or both bounded account-wide audience or placement control objects.",
        annotations(
            title = "Update Meta account controls",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn update_account_controls(
        &self,
        Parameters(input): Parameters<UpdateAccountControlsInput>,
    ) -> Result<Json<ToolResponse<AccountControlsUpdate>>, Json<ToolResponse<AccountControlsUpdate>>>
    {
        update_account_controls(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "list_ad_accounts",
        description = "List Meta ad accounts visible to the configured user, with bounded cursor pagination.",
        annotations(
            title = "List Meta ad accounts",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_ad_accounts(
        &self,
        Parameters(input): Parameters<ListAdAccountsInput>,
    ) -> Result<Json<ToolResponse<AdAccountList>>, Json<ToolResponse<AdAccountList>>> {
        list_ad_accounts(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "read_ad_account",
        description = "Read metadata for one Meta ad account by numeric ID.",
        annotations(
            title = "Read Meta ad account",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn read_ad_account(
        &self,
        Parameters(input): Parameters<ReadAdAccountInput>,
    ) -> Result<Json<ToolResponse<AdAccount>>, Json<ToolResponse<AdAccount>>> {
        read_ad_account(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "list_campaigns",
        description = "List campaigns for one Meta ad account with bounded v26 filters and cursor pagination.",
        annotations(
            title = "List Meta campaigns",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_campaigns(
        &self,
        Parameters(input): Parameters<ListCampaignsInput>,
    ) -> Result<Json<ToolResponse<CampaignList>>, Json<ToolResponse<CampaignList>>> {
        list_campaigns(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "read_campaign",
        description = "Read compact metadata for one Meta campaign by numeric ID.",
        annotations(
            title = "Read Meta campaign",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn read_campaign(
        &self,
        Parameters(input): Parameters<ReadCampaignInput>,
    ) -> Result<Json<ToolResponse<Campaign>>, Json<ToolResponse<Campaign>>> {
        read_campaign(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "create_campaign",
        description = "Create one typed Meta campaign; delivery defaults to PAUSED and repeated calls can duplicate it.",
        annotations(
            title = "Create Meta campaign",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_campaign(
        &self,
        Parameters(input): Parameters<CreateCampaignInput>,
    ) -> Result<Json<ToolResponse<CreatedCampaign>>, Json<ToolResponse<CreatedCampaign>>> {
        create_campaign(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "update_campaign",
        description = "Update explicitly supplied fields on one Meta campaign; ACTIVE may begin delivery.",
        annotations(
            title = "Update Meta campaign",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn update_campaign(
        &self,
        Parameters(input): Parameters<UpdateCampaignInput>,
    ) -> Result<Json<ToolResponse<UpdatedCampaign>>, Json<ToolResponse<UpdatedCampaign>>> {
        update_campaign(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "clone_campaign",
        description = "Copy one Meta campaign through the v26 copies edge; the copy defaults to PAUSED.",
        annotations(
            title = "Clone Meta campaign",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn clone_campaign(
        &self,
        Parameters(input): Parameters<CloneCampaignInput>,
    ) -> Result<Json<ToolResponse<ClonedCampaign>>, Json<ToolResponse<ClonedCampaign>>> {
        clone_campaign(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "create_campaign_budget_schedule",
        description = "Create one bounded high-demand budget schedule; repeated calls can duplicate the schedule.",
        annotations(
            title = "Create Meta campaign budget schedule",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_campaign_budget_schedule(
        &self,
        Parameters(input): Parameters<CreateCampaignBudgetScheduleInput>,
    ) -> Result<
        Json<ToolResponse<CreatedCampaignBudgetSchedule>>,
        Json<ToolResponse<CreatedCampaignBudgetSchedule>>,
    > {
        create_campaign_budget_schedule(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "create_ad_set",
        description = "Create one typed Meta ad set; delivery defaults to PAUSED and repeated calls can duplicate it.",
        annotations(
            title = "Create Meta ad set",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_ad_set(
        &self,
        Parameters(input): Parameters<CreateAdSetInput>,
    ) -> Result<Json<ToolResponse<CreatedAdSet>>, Json<ToolResponse<CreatedAdSet>>> {
        create_ad_set(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "update_ad_set",
        description = "Update explicitly supplied fields on one Meta ad set; ACTIVE may begin delivery.",
        annotations(
            title = "Update Meta ad set",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn update_ad_set(
        &self,
        Parameters(input): Parameters<UpdateAdSetInput>,
    ) -> Result<Json<ToolResponse<UpdatedAdSet>>, Json<ToolResponse<UpdatedAdSet>>> {
        update_ad_set(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "clone_ad_set",
        description = "Copy one Meta ad set through the v26 copies edge; defaults to PAUSED and can optionally copy child ads.",
        annotations(
            title = "Clone Meta ad set",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn clone_ad_set(
        &self,
        Parameters(input): Parameters<CloneAdSetInput>,
    ) -> Result<Json<ToolResponse<ClonedAdSet>>, Json<ToolResponse<ClonedAdSet>>> {
        clone_ad_set(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "list_ad_sets",
        description = "List Meta ad sets for an account or campaign with bounded cursor pagination.",
        annotations(
            title = "List Meta ad sets",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_ad_sets(
        &self,
        Parameters(input): Parameters<ListAdSetsInput>,
    ) -> Result<Json<ToolResponse<AdSetList>>, Json<ToolResponse<AdSetList>>> {
        list_ad_sets(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "read_ad_set",
        description = "Read details for one Meta ad set by numeric ID.",
        annotations(
            title = "Read Meta ad set",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn read_ad_set(
        &self,
        Parameters(input): Parameters<ReadAdSetInput>,
    ) -> Result<Json<ToolResponse<AdSet>>, Json<ToolResponse<AdSet>>> {
        read_ad_set(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "list_ads",
        description = "List compact Meta ad summaries for an account, campaign, or ad set with cursor pagination.",
        annotations(title = "List Meta ads", read_only_hint = true, open_world_hint = true)
    )]
    async fn list_ads(
        &self,
        Parameters(input): Parameters<ListAdsInput>,
    ) -> Result<Json<ToolResponse<AdList>>, Json<ToolResponse<AdList>>> {
        list_ads(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "read_ad",
        description = "Read compact metadata for one Meta ad by numeric ID.",
        annotations(title = "Read Meta ad", read_only_hint = true, open_world_hint = true)
    )]
    async fn read_ad(
        &self,
        Parameters(input): Parameters<ReadAdInput>,
    ) -> Result<Json<ToolResponse<Ad>>, Json<ToolResponse<Ad>>> {
        read_ad(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "create_ad",
        description = "Create one Meta ad from an existing creative; defaults to PAUSED and repeated calls can duplicate it.",
        annotations(
            title = "Create Meta ad",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_ad(
        &self,
        Parameters(input): Parameters<CreateAdInput>,
    ) -> Result<Json<ToolResponse<CreatedAd>>, Json<ToolResponse<CreatedAd>>> {
        create_ad(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "update_ad",
        description = "Update explicitly supplied fields on one Meta ad; ACTIVE may begin delivery.",
        annotations(
            title = "Update Meta ad",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn update_ad(
        &self,
        Parameters(input): Parameters<UpdateAdInput>,
    ) -> Result<Json<ToolResponse<UpdatedAd>>, Json<ToolResponse<UpdatedAd>>> {
        update_ad(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "clone_ad",
        description = "Copy one Meta ad through the v26 copies edge; defaults to PAUSED and repeated calls can duplicate it.",
        annotations(
            title = "Clone Meta ad",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn clone_ad(
        &self,
        Parameters(input): Parameters<CloneAdInput>,
    ) -> Result<Json<ToolResponse<ClonedAd>>, Json<ToolResponse<ClonedAd>>> {
        clone_ad(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "create_ad_creative",
        description = "Create one typed reusable Meta ad creative; repeated calls can duplicate it.",
        annotations(
            title = "Create Meta ad creative",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_ad_creative(
        &self,
        Parameters(input): Parameters<CreateAdCreativeInput>,
    ) -> Result<Json<ToolResponse<CreatedAdCreative>>, Json<ToolResponse<CreatedAdCreative>>> {
        create_ad_creative(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "update_ad_creative",
        description = "Rename one Meta ad creative; content is immutable, so replace the creative to change it.",
        annotations(
            title = "Rename Meta ad creative",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn update_ad_creative(
        &self,
        Parameters(input): Parameters<UpdateAdCreativeInput>,
    ) -> Result<Json<ToolResponse<UpdatedAdCreative>>, Json<ToolResponse<UpdatedAdCreative>>> {
        update_ad_creative(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "read_ad_image",
        description = "Return one ad's primary image as bounded native MCP image content from approved Meta CDNs.",
        annotations(
            title = "Read Meta ad image",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn read_ad_image(
        &self,
        Parameters(input): Parameters<ReadAdImageInput>,
    ) -> CallToolResult {
        read_ad_image(&self.graph, input).await
    }

    #[tool(
        name = "upload_ad_image_asset",
        description = "Upload or cross-account-copy one JPEG/PNG ad image; local paths are confined to META_MEDIA_ROOT.",
        annotations(
            title = "Upload Meta ad image",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn upload_ad_image_asset(
        &self,
        Parameters(input): Parameters<UploadAdImageAssetInput>,
    ) -> Result<Json<ToolResponse<UploadedAdImageAsset>>, Json<ToolResponse<UploadedAdImageAsset>>>
    {
        upload_ad_image_asset(&self.graph, self.media_root.as_deref(), input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "upload_ad_video_asset",
        description = "Upload one local MP4/MOV from META_MEDIA_ROOT or ask Meta to fetch one public HTTPS video.",
        annotations(
            title = "Upload Meta ad video",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn upload_ad_video_asset(
        &self,
        Parameters(input): Parameters<UploadAdVideoAssetInput>,
    ) -> Result<Json<ToolResponse<UploadedAdVideoAsset>>, Json<ToolResponse<UploadedAdVideoAsset>>>
    {
        upload_ad_video_asset(&self.graph, self.media_root.as_deref(), input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "list_custom_audiences",
        description = "List compact custom-audience metadata without members, rules, or source payloads.",
        annotations(
            title = "List Meta custom audiences",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_custom_audiences(
        &self,
        Parameters(input): Parameters<ListCustomAudiencesInput>,
    ) -> Result<Json<ToolResponse<CustomAudienceList>>, Json<ToolResponse<CustomAudienceList>>>
    {
        list_custom_audiences(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "read_custom_audience",
        description = "Read compact aggregate metadata for one custom audience; never returns members.",
        annotations(
            title = "Read Meta custom audience",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn read_custom_audience(
        &self,
        Parameters(input): Parameters<ReadCustomAudienceInput>,
    ) -> Result<Json<ToolResponse<CustomAudience>>, Json<ToolResponse<CustomAudience>>> {
        read_custom_audience(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "create_custom_audience",
        description = "Create one empty customer-list audience; repeated calls can create duplicates.",
        annotations(
            title = "Create Meta custom audience",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_custom_audience(
        &self,
        Parameters(input): Parameters<CreateCustomAudienceInput>,
    ) -> Result<Json<ToolResponse<CreatedCustomAudience>>, Json<ToolResponse<CreatedCustomAudience>>>
    {
        create_custom_audience(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "update_custom_audience",
        description = "Update the bounded name or description of one customer-list audience.",
        annotations(
            title = "Update Meta custom audience",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn update_custom_audience(
        &self,
        Parameters(input): Parameters<UpdateCustomAudienceInput>,
    ) -> Result<
        Json<ToolResponse<CustomAudienceMutationAck>>,
        Json<ToolResponse<CustomAudienceMutationAck>>,
    > {
        update_custom_audience(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "delete_custom_audience",
        description = "Permanently delete one custom audience after CONFIRM_META_ADS_REMOVALS; reconcile current state before retrying an ambiguous failure.",
        annotations(
            title = "Delete Meta custom audience",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn delete_custom_audience(
        &self,
        Parameters(input): Parameters<DeleteCustomAudienceInput>,
    ) -> Result<
        Json<ToolResponse<CustomAudienceMutationAck>>,
        Json<ToolResponse<CustomAudienceMutationAck>>,
    > {
        delete_custom_audience(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "manage_custom_audience_users",
        description = "Add, remove, or session-replace bounded customer rows with field-specific hashing; removal or replace requires CONFIRM_META_ADS_REMOVALS.",
        annotations(
            title = "Manage Meta custom-audience users",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn manage_custom_audience_users(
        &self,
        Parameters(input): Parameters<ManageCustomAudienceUsersInput>,
    ) -> Result<Json<ToolResponse<ManagedAudienceUsers>>, Json<ToolResponse<ManagedAudienceUsers>>>
    {
        manage_custom_audience_users(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "create_lookalike_audience",
        description = "Create one typed country-based lookalike; repeated calls can create duplicates.",
        annotations(
            title = "Create Meta lookalike audience",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_lookalike_audience(
        &self,
        Parameters(input): Parameters<CreateLookalikeAudienceInput>,
    ) -> Result<Json<ToolResponse<CreatedCustomAudience>>, Json<ToolResponse<CreatedCustomAudience>>>
    {
        create_lookalike_audience(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "create_custom_conversion",
        description = "Create one bounded custom conversion; repeated calls can create duplicates.",
        annotations(
            title = "Create Meta custom conversion",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_custom_conversion(
        &self,
        Parameters(input): Parameters<CreateCustomConversionInput>,
    ) -> Result<
        Json<ToolResponse<CreatedCustomConversion>>,
        Json<ToolResponse<CreatedCustomConversion>>,
    > {
        create_custom_conversion(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "update_custom_conversion",
        description = "Update only the current writable name, description, or default value fields.",
        annotations(
            title = "Update Meta custom conversion",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn update_custom_conversion(
        &self,
        Parameters(input): Parameters<UpdateCustomConversionInput>,
    ) -> Result<
        Json<ToolResponse<UpdatedCustomConversion>>,
        Json<ToolResponse<UpdatedCustomConversion>>,
    > {
        update_custom_conversion(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "delete_custom_conversion",
        description = "Archive or delete one custom conversion after CONFIRM_META_ADS_REMOVALS; reconcile current state before retrying an ambiguous failure.",
        annotations(
            title = "Delete Meta custom conversion",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn delete_custom_conversion(
        &self,
        Parameters(input): Parameters<DeleteCustomConversionInput>,
    ) -> Result<
        Json<ToolResponse<DeletedCustomConversion>>,
        Json<ToolResponse<DeletedCustomConversion>>,
    > {
        delete_custom_conversion(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "search_ads_archive",
        description = "Search Meta's public Ads Library with typed v26 filters and bounded cursor results.",
        annotations(
            title = "Search Meta Ads Library",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn search_ads_archive(
        &self,
        Parameters(input): Parameters<SearchAdsArchiveInput>,
    ) -> Result<Json<ToolResponse<ArchivedAdList>>, Json<ToolResponse<ArchivedAdList>>> {
        search_ads_archive(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "list_ad_creatives",
        description = "List compact creative metadata attached to one ad without large story or asset-feed specs.",
        annotations(
            title = "List Meta ad creatives",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_ad_creatives(
        &self,
        Parameters(input): Parameters<ListAdCreativesInput>,
    ) -> Result<Json<ToolResponse<AdCreativeList>>, Json<ToolResponse<AdCreativeList>>> {
        list_ad_creatives(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "read_ad_creative",
        description = "Read compact metadata for one Meta ad creative by numeric ID.",
        annotations(
            title = "Read Meta ad creative",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn read_ad_creative(
        &self,
        Parameters(input): Parameters<ReadAdCreativeInput>,
    ) -> Result<Json<ToolResponse<AdCreative>>, Json<ToolResponse<AdCreative>>> {
        read_ad_creative(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "list_ad_images",
        description = "List bounded ad-image metadata for an account; returns hashes and URLs, never image bytes.",
        annotations(
            title = "List Meta ad images",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_ad_images(
        &self,
        Parameters(input): Parameters<ListAdImagesInput>,
    ) -> Result<Json<ToolResponse<AdImageList>>, Json<ToolResponse<AdImageList>>> {
        list_ad_images(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "list_ad_videos",
        description = "List bounded ad-video metadata for an account; never returns source media bytes.",
        annotations(
            title = "List Meta ad videos",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_ad_videos(
        &self,
        Parameters(input): Parameters<ListAdVideosInput>,
    ) -> Result<Json<ToolResponse<AdVideoList>>, Json<ToolResponse<AdVideoList>>> {
        list_ad_videos(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "list_insights",
        description = "Read bounded Meta Insights rows without aggregating non-additive metrics such as reach. Prefer page_size 25-50 at ad level to avoid oversized model output.",
        annotations(
            title = "List Meta Ads insights",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_insights(
        &self,
        Parameters(input): Parameters<ListInsightsInput>,
    ) -> Result<Json<ToolResponse<InsightPage>>, Json<ToolResponse<InsightPage>>> {
        list_insights(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "create_insights_job",
        description = "Start one async Meta Insights report run; repeated calls can create duplicate jobs.",
        annotations(
            title = "Create Meta Insights job",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_insights_job(
        &self,
        Parameters(input): Parameters<CreateInsightsJobInput>,
    ) -> Result<Json<ToolResponse<CreatedInsightsJob>>, Json<ToolResponse<CreatedInsightsJob>>>
    {
        create_insights_job(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "read_insights_job",
        description = "Read status and bounded error metadata for one async Meta Insights report run.",
        annotations(
            title = "Read Meta Insights job",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn read_insights_job(
        &self,
        Parameters(input): Parameters<ReadInsightsJobInput>,
    ) -> Result<Json<ToolResponse<InsightsJob>>, Json<ToolResponse<InsightsJob>>> {
        read_insights_job(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "read_insights_job_results",
        description = "Read one bounded cursor page from a completed async Meta Insights report run.",
        annotations(
            title = "Read Meta Insights job results",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn read_insights_job_results(
        &self,
        Parameters(input): Parameters<ReadInsightsJobResultsInput>,
    ) -> Result<Json<ToolResponse<InsightPage>>, Json<ToolResponse<InsightPage>>> {
        read_insights_job_results(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "list_product_catalogs",
        description = "List compact product-catalog metadata owned by one Meta business.",
        annotations(
            title = "List Meta product catalogs",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_product_catalogs(
        &self,
        Parameters(input): Parameters<ListProductCatalogsInput>,
    ) -> Result<Json<ToolResponse<ProductCatalogList>>, Json<ToolResponse<ProductCatalogList>>>
    {
        list_product_catalogs(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "list_products",
        description = "List compact products from one Meta catalog without descriptions or media arrays.",
        annotations(
            title = "List Meta catalog products",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_products(
        &self,
        Parameters(input): Parameters<ListProductsInput>,
    ) -> Result<Json<ToolResponse<ProductList>>, Json<ToolResponse<ProductList>>> {
        list_products(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "list_product_sets",
        description = "List bounded product-set metadata and compact filter text from one catalog.",
        annotations(
            title = "List Meta product sets",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_product_sets(
        &self,
        Parameters(input): Parameters<ListProductSetsInput>,
    ) -> Result<Json<ToolResponse<ProductSetList>>, Json<ToolResponse<ProductSetList>>> {
        list_product_sets(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "upsert_product",
        description = "Create or update one catalog product by stable retailer ID through Meta's v26 product edge.",
        annotations(
            title = "Upsert Meta catalog product",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn upsert_product(
        &self,
        Parameters(input): Parameters<UpsertProductInput>,
    ) -> Result<Json<ToolResponse<UpsertedProduct>>, Json<ToolResponse<UpsertedProduct>>> {
        upsert_product(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "batch_products",
        description = "Submit bounded catalog upserts and deletions; delete operations require CONFIRM_META_ADS_REMOVALS, and ambiguous outcomes require reconciliation.",
        annotations(
            title = "Batch Meta catalog products",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn batch_products(
        &self,
        Parameters(input): Parameters<BatchProductsInput>,
    ) -> Result<Json<ToolResponse<AcceptedProductBatch>>, Json<ToolResponse<AcceptedProductBatch>>>
    {
        batch_products(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "list_custom_conversions",
        description = "List compact custom-conversion metadata for one Meta ad account.",
        annotations(
            title = "List Meta custom conversions",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_custom_conversions(
        &self,
        Parameters(input): Parameters<ListCustomConversionsInput>,
    ) -> Result<Json<ToolResponse<CustomConversionList>>, Json<ToolResponse<CustomConversionList>>>
    {
        list_custom_conversions(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "read_custom_conversion",
        description = "Read one custom conversion, with bounded rule text and explicit truncation flags.",
        annotations(
            title = "Read Meta custom conversion",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn read_custom_conversion(
        &self,
        Parameters(input): Parameters<ReadCustomConversionInput>,
    ) -> Result<Json<ToolResponse<CustomConversion>>, Json<ToolResponse<CustomConversion>>> {
        read_custom_conversion(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "list_business_datasets",
        description = "List compact Ads Dataset metadata from a Meta business's current ads_dataset edge.",
        annotations(
            title = "List Meta business datasets",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_business_datasets(
        &self,
        Parameters(input): Parameters<ListBusinessDatasetsInput>,
    ) -> Result<Json<ToolResponse<BusinessDatasetList>>, Json<ToolResponse<BusinessDatasetList>>>
    {
        list_business_datasets(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "read_dataset_quality",
        description = "Read compact ingestion and matching quality for one Ads Dataset owned by a Meta business.",
        annotations(
            title = "Read Meta dataset quality",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn read_dataset_quality(
        &self,
        Parameters(input): Parameters<ReadDatasetQualityInput>,
    ) -> Result<Json<ToolResponse<DatasetQuality>>, Json<ToolResponse<DatasetQuality>>> {
        read_dataset_quality(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "send_capi_events",
        description = "Submit a bounded Conversions API event batch; matchable PII is normalized and SHA-256 hashed locally.",
        annotations(
            title = "Send Meta Conversions API events",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn send_capi_events(
        &self,
        Parameters(input): Parameters<SendCapiEventsInput>,
    ) -> Result<Json<ToolResponse<SentCapiEvents>>, Json<ToolResponse<SentCapiEvents>>> {
        send_capi_events(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "list_ad_previews",
        description = "Render one bounded Meta ad-preview format; defaults to desktop feed to avoid probe requests.",
        annotations(
            title = "Render Meta ad preview",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_ad_previews(
        &self,
        Parameters(input): Parameters<ListAdPreviewsInput>,
    ) -> Result<Json<ToolResponse<AdPreviewList>>, Json<ToolResponse<AdPreviewList>>> {
        list_ad_previews(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "search_interests",
        description = "Autocomplete current Meta interest targets with bounded audience-size ranges.",
        annotations(
            title = "Search Meta interests",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn search_interests(
        &self,
        Parameters(input): Parameters<SearchInterestsInput>,
    ) -> Result<Json<ToolResponse<InterestPage>>, Json<ToolResponse<InterestPage>>> {
        search_interests(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "suggest_interests",
        description = "Suggest related Meta interest targets from a bounded list of seed names.",
        annotations(
            title = "Suggest Meta interests",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn suggest_interests(
        &self,
        Parameters(input): Parameters<SuggestInterestsInput>,
    ) -> Result<Json<ToolResponse<InterestPage>>, Json<ToolResponse<InterestPage>>> {
        suggest_interests(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "estimate_audience_size",
        description = "Estimate a bounded common v26 targeting spec as readiness plus lower and upper user bounds.",
        annotations(
            title = "Estimate Meta audience size",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn estimate_audience_size(
        &self,
        Parameters(input): Parameters<EstimateAudienceSizeInput>,
    ) -> Result<Json<ToolResponse<AudienceSizeEstimate>>, Json<ToolResponse<AudienceSizeEstimate>>>
    {
        estimate_audience_size(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "search_behaviors",
        description = "List current Meta behavior-targeting categories with bounded cursor pagination.",
        annotations(
            title = "Search Meta behaviors",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn search_behaviors(
        &self,
        Parameters(input): Parameters<SearchBehaviorsInput>,
    ) -> Result<Json<ToolResponse<TargetingCategoryPage>>, Json<ToolResponse<TargetingCategoryPage>>>
    {
        search_behaviors(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "search_demographics",
        description = "List one current Meta demographic targeting class with bounded cursor pagination.",
        annotations(
            title = "Search Meta demographics",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn search_demographics(
        &self,
        Parameters(input): Parameters<SearchDemographicsInput>,
    ) -> Result<Json<ToolResponse<TargetingCategoryPage>>, Json<ToolResponse<TargetingCategoryPage>>>
    {
        search_demographics(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "search_geo_locations",
        description = "Autocomplete current Meta geo targets and return stable keys plus compact hierarchy data.",
        annotations(
            title = "Search Meta geo locations",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn search_geo_locations(
        &self,
        Parameters(input): Parameters<SearchGeoLocationsInput>,
    ) -> Result<Json<ToolResponse<GeoLocationPage>>, Json<ToolResponse<GeoLocationPage>>> {
        search_geo_locations(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "list_pages",
        description = "List or filter user-promotable, business-owned, or business-client Facebook Pages.",
        annotations(
            title = "List Facebook Pages",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_pages(
        &self,
        Parameters(input): Parameters<ListPagesInput>,
    ) -> Result<Json<ToolResponse<PageList>>, Json<ToolResponse<PageList>>> {
        list_pages(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "create_report",
        description = "Create a compact read-only performance summary for one Meta object and optional comparison period.",
        annotations(
            title = "Summarize Meta Ads performance",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn create_report(
        &self,
        Parameters(input): Parameters<CreateReportInput>,
    ) -> Result<Json<ToolResponse<PerformanceReport>>, Json<ToolResponse<PerformanceReport>>> {
        create_report(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "list_ad_custom_derived_metrics",
        description = "List bounded custom derived metrics owned by one Meta Business under the v26 contract.",
        annotations(
            title = "List Meta custom derived metrics",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_ad_custom_derived_metrics(
        &self,
        Parameters(input): Parameters<ListAdCustomDerivedMetricsInput>,
    ) -> Result<Json<ToolResponse<DerivedMetricPage>>, Json<ToolResponse<DerivedMetricPage>>> {
        list_ad_custom_derived_metrics(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "list_recommendations",
        description = "List advisory account recommendations without provider action payloads or signatures.",
        annotations(
            title = "List Meta account recommendations",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_recommendations(
        &self,
        Parameters(input): Parameters<ListRecommendationsInput>,
    ) -> Result<Json<ToolResponse<RecommendationPage>>, Json<ToolResponse<RecommendationPage>>>
    {
        list_recommendations(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "list_branded_content_ad_permissions",
        description = "List account-level partnership-ad permissions for one Instagram professional account.",
        annotations(
            title = "List Meta partnership-ad permissions",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_branded_content_ad_permissions(
        &self,
        Parameters(input): Parameters<ListBrandedContentAdPermissionsInput>,
    ) -> Result<
        Json<ToolResponse<BrandedContentPermissionPage>>,
        Json<ToolResponse<BrandedContentPermissionPage>>,
    > {
        list_branded_content_ad_permissions(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "grant_branded_content_ad_permission",
        description = "Grant one creator partnership-ad permission; verify current permissions before retrying an ambiguous failure.",
        annotations(
            title = "Grant Meta partnership-ad permission",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn grant_branded_content_ad_permission(
        &self,
        Parameters(input): Parameters<GrantBrandedContentAdPermissionInput>,
    ) -> Result<
        Json<ToolResponse<GrantedBrandedContentAdPermission>>,
        Json<ToolResponse<GrantedBrandedContentAdPermission>>,
    > {
        grant_branded_content_ad_permission(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "revoke_branded_content_ad_permission",
        description = "Revoke one creator partnership-ad permission after CONFIRM_META_ADS_REMOVALS; verify current permissions before retrying an ambiguous failure.",
        annotations(
            title = "Revoke Meta partnership-ad permission",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn revoke_branded_content_ad_permission(
        &self,
        Parameters(input): Parameters<RevokeBrandedContentAdPermissionInput>,
    ) -> Result<
        Json<ToolResponse<RevokedBrandedContentAdPermission>>,
        Json<ToolResponse<RevokedBrandedContentAdPermission>>,
    > {
        revoke_branded_content_ad_permission(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "build_mutation_plan",
        description = "Freeze one bounded Graph v26 create, update, or delete plan for campaigns, ad sets, ads, creatives, custom audiences, or reach/frequency. Common delivery writes are validate-only checked; no live change occurs until apply_mutation_plan.",
        annotations(
            title = "Build a Meta Ads mutation plan",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn build_mutation_plan(
        &self,
        Parameters(input): Parameters<BuildMutationPlanInput>,
    ) -> Result<Json<ToolResponse<MutationPlanReceipt>>, Json<ToolResponse<MutationPlanReceipt>>>
    {
        build_mutation_plan(&self.graph, &self.plans, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "apply_mutation_plan",
        description = "Send one reviewed, frozen Meta request once per apply. Requires APPLY_LIVE_META_ADS_CHANGES; deletes, status=DELETED, and reach/frequency cancel or release also require CONFIRM_META_ADS_REMOVALS.",
        annotations(
            title = "Apply a Meta Ads mutation plan",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn apply_mutation_plan(
        &self,
        Parameters(input): Parameters<ApplyMutationPlanInput>,
    ) -> Result<Json<ToolResponse<AppliedMutationPlan>>, Json<ToolResponse<AppliedMutationPlan>>>
    {
        apply_mutation_plan(&self.graph, &self.plans, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "get_mutation_plan",
        description = "Read one unexpired local Meta mutation-plan preview and status without exposing its frozen form.",
        annotations(
            title = "Read a Meta Ads mutation plan",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_mutation_plan(
        &self,
        Parameters(input): Parameters<GetMutationPlanInput>,
    ) -> Result<Json<ToolResponse<MutationPlanView>>, Json<ToolResponse<MutationPlanView>>> {
        get_mutation_plan(&self.plans, input).into_mcp_result()
    }

    #[tool(
        name = "discard_mutation_plan",
        description = "Discard one pending local Meta mutation plan without contacting Meta.",
        annotations(
            title = "Discard a Meta Ads mutation plan",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn discard_mutation_plan(
        &self,
        Parameters(input): Parameters<DiscardMutationPlanInput>,
    ) -> Result<Json<ToolResponse<DiscardedMutationPlan>>, Json<ToolResponse<DiscardedMutationPlan>>>
    {
        discard_mutation_plan(&self.plans, input).into_mcp_result()
    }

    #[tool(
        name = "read_reach_frequency_prediction",
        description = "Read one bounded Meta reach-and-frequency prediction and its capped budget curve.",
        annotations(
            title = "Read Meta reach-and-frequency prediction",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn read_reach_frequency_prediction(
        &self,
        Parameters(input): Parameters<ReadReachFrequencyPredictionInput>,
    ) -> Result<
        Json<ToolResponse<ReachFrequencyPrediction>>,
        Json<ToolResponse<ReachFrequencyPrediction>>,
    > {
        read_reach_frequency_prediction(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "list_reach_frequency_predictions",
        description = "List compact reach-and-frequency predictions for one reservation-capable ad account.",
        annotations(
            title = "List Meta reach-and-frequency predictions",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn list_reach_frequency_predictions(
        &self,
        Parameters(input): Parameters<ListReachFrequencyPredictionsInput>,
    ) -> Result<
        Json<ToolResponse<ReachFrequencyPredictionPage>>,
        Json<ToolResponse<ReachFrequencyPredictionPage>>,
    > {
        list_reach_frequency_predictions(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "create_reach_frequency_prediction",
        description = "Create one bounded Graph v26 budget-to-reach quote without reserving inventory; verify the prediction list before retrying an ambiguous failure.",
        annotations(
            title = "Create Meta reach/frequency prediction",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_reach_frequency_prediction(
        &self,
        Parameters(input): Parameters<CreateReachFrequencyPredictionInput>,
    ) -> Result<
        Json<ToolResponse<CreatedReachFrequencyPrediction>>,
        Json<ToolResponse<CreatedReachFrequencyPrediction>>,
    > {
        create_reach_frequency_prediction(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "get_threads_account",
        description = "Resolve an associated, Instagram-backed, or Page-backed Threads account relationship.",
        annotations(
            title = "Resolve Meta Threads account",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn get_threads_account(
        &self,
        Parameters(input): Parameters<GetThreadsAccountInput>,
    ) -> Result<Json<ToolResponse<ThreadsAccount>>, Json<ToolResponse<ThreadsAccount>>> {
        get_threads_account(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "create_threads_account",
        description = "Create one Instagram- or Page-backed Threads account; v26 does not accept its ID as a creative input.",
        annotations(
            title = "Create Meta Threads account",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_threads_account(
        &self,
        Parameters(input): Parameters<CreateThreadsAccountInput>,
    ) -> Result<Json<ToolResponse<CreatedThreadsAccount>>, Json<ToolResponse<CreatedThreadsAccount>>>
    {
        create_threads_account(&self.graph, input)
            .await
            .into_mcp_result()
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for MetaAdsServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "armavita-meta-ads-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(SERVER_INSTRUCTIONS)
    }
}

#[cfg(test)]
mod tests {
    use rmcp::{
        ServerHandler,
        handler::server::{tool::IntoCallToolResult, wrapper::Parameters},
        model::CallToolResponse,
    };
    use serde_json::Value;

    use super::MetaAdsServer;
    use crate::{MetaConfig, accounts::ListAdAccountsInput};

    fn server() -> MetaAdsServer {
        MetaAdsServer::new(MetaConfig::for_test(
            "https://graph.facebook.com/v26.0",
            None,
        ))
        .unwrap()
    }

    #[test]
    fn tool_surface_is_exact_compact_and_safely_annotated() {
        let tools = server().tool_definitions();
        let names = tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "apply_mutation_plan",
                "batch_products",
                "build_mutation_plan",
                "clone_ad",
                "clone_ad_set",
                "clone_campaign",
                "create_ad",
                "create_ad_creative",
                "create_ad_set",
                "create_campaign",
                "create_campaign_budget_schedule",
                "create_custom_audience",
                "create_custom_conversion",
                "create_insights_job",
                "create_lookalike_audience",
                "create_reach_frequency_prediction",
                "create_report",
                "create_threads_account",
                "delete_custom_audience",
                "delete_custom_conversion",
                "discard_mutation_plan",
                "estimate_audience_size",
                "get_account_controls",
                "get_mutation_plan",
                "get_threads_account",
                "grant_branded_content_ad_permission",
                "list_ad_accounts",
                "list_ad_creatives",
                "list_ad_custom_derived_metrics",
                "list_ad_images",
                "list_ad_previews",
                "list_ad_sets",
                "list_ad_videos",
                "list_ads",
                "list_branded_content_ad_permissions",
                "list_business_datasets",
                "list_campaigns",
                "list_custom_audiences",
                "list_custom_conversions",
                "list_insights",
                "list_pages",
                "list_product_catalogs",
                "list_product_sets",
                "list_products",
                "list_reach_frequency_predictions",
                "list_recommendations",
                "manage_custom_audience_users",
                "read_ad",
                "read_ad_account",
                "read_ad_creative",
                "read_ad_image",
                "read_ad_set",
                "read_campaign",
                "read_custom_audience",
                "read_custom_conversion",
                "read_dataset_quality",
                "read_insights_job",
                "read_insights_job_results",
                "read_reach_frequency_prediction",
                "revoke_branded_content_ad_permission",
                "search_ads_archive",
                "search_behaviors",
                "search_demographics",
                "search_geo_locations",
                "search_interests",
                "send_capi_events",
                "suggest_interests",
                "update_account_controls",
                "update_ad",
                "update_ad_creative",
                "update_ad_set",
                "update_campaign",
                "update_custom_audience",
                "update_custom_conversion",
                "upload_ad_image_asset",
                "upload_ad_video_asset",
                "upsert_product"
            ]
        );

        for tool in tools {
            let schema = serde_json::to_string(&tool.input_schema).unwrap();
            assert!(!schema.contains("meta_access_token"));
            assert!(
                !serde_json::to_string(&tool)
                    .unwrap()
                    .to_ascii_lowercase()
                    .contains("whatsapp")
            );
            if tool.name.as_ref() != "read_ad_image" {
                assert!(tool.output_schema.is_some());
            }
            let annotations = tool.annotations.unwrap();
            match tool.name.as_ref() {
                "clone_ad"
                | "clone_ad_set"
                | "clone_campaign"
                | "create_ad"
                | "create_ad_creative"
                | "create_ad_set"
                | "create_campaign"
                | "create_campaign_budget_schedule"
                | "create_custom_audience"
                | "create_custom_conversion"
                | "create_insights_job"
                | "create_lookalike_audience"
                | "create_reach_frequency_prediction"
                | "send_capi_events"
                | "create_threads_account"
                | "grant_branded_content_ad_permission"
                | "build_mutation_plan"
                | "upload_ad_image_asset"
                | "upload_ad_video_asset" => {
                    assert_eq!(annotations.read_only_hint, Some(false));
                    assert_eq!(annotations.destructive_hint, Some(false));
                    assert_eq!(annotations.idempotent_hint, Some(false));
                }
                "apply_mutation_plan"
                | "batch_products"
                | "delete_custom_conversion"
                | "manage_custom_audience_users"
                | "revoke_branded_content_ad_permission" => {
                    assert_eq!(annotations.read_only_hint, Some(false));
                    assert_eq!(annotations.destructive_hint, Some(true));
                    assert_eq!(annotations.idempotent_hint, Some(false));
                }
                "delete_custom_audience" => {
                    assert_eq!(annotations.read_only_hint, Some(false));
                    assert_eq!(annotations.destructive_hint, Some(true));
                    assert_eq!(annotations.idempotent_hint, Some(true));
                }
                "update_account_controls"
                | "discard_mutation_plan"
                | "update_ad"
                | "update_ad_creative"
                | "update_ad_set"
                | "update_campaign"
                | "update_custom_audience"
                | "update_custom_conversion"
                | "upsert_product" => {
                    assert_eq!(annotations.read_only_hint, Some(false));
                    assert_eq!(annotations.destructive_hint, Some(false));
                    assert_eq!(annotations.idempotent_hint, Some(true));
                }
                _ => assert_eq!(annotations.read_only_hint, Some(true)),
            }
            assert_eq!(
                annotations.open_world_hint,
                Some(!matches!(
                    tool.name.as_ref(),
                    "discard_mutation_plan" | "get_mutation_plan"
                ))
            );
        }
    }

    #[test]
    fn public_input_schemas_are_closed_and_required_arrays_are_bounded() {
        let tools = MetaAdsServer::new(MetaConfig::for_test(
            "https://graph.facebook.com/v26.0",
            None,
        ))
        .unwrap()
        .tool_definitions();
        assert_eq!(tools.len(), 77);

        for tool in &tools {
            let schema = serde_json::to_value(&tool.input_schema).unwrap();
            assert_eq!(
                schema.get("type").and_then(Value::as_str),
                Some("object"),
                "{} input must have an object root",
                tool.name
            );
            assert_eq!(
                schema.get("additionalProperties").and_then(Value::as_bool),
                Some(false),
                "{} input root must reject unknown fields",
                tool.name
            );
        }

        for (tool_name, pointer, expected) in [
            (
                "create_ad_set",
                "/$defs/EstimateGeoLocations/properties/countries/maxItems",
                250,
            ),
            (
                "estimate_audience_size",
                "/$defs/EstimateGeoLocations/properties/countries/maxItems",
                250,
            ),
            (
                "update_ad_set",
                "/$defs/EstimateGeoLocations/properties/countries/maxItems",
                250,
            ),
            (
                "manage_custom_audience_users",
                "/properties/data/maxItems",
                1_000,
            ),
            (
                "manage_custom_audience_users",
                "/properties/data/items/maxItems",
                15,
            ),
            (
                "manage_custom_audience_users",
                "/properties/schema/maxItems",
                15,
            ),
            (
                "search_ads_archive",
                "/properties/ad_reached_countries/maxItems",
                25,
            ),
            (
                "suggest_interests",
                "/properties/interest_list/maxItems",
                25,
            ),
        ] {
            let tool = tools
                .iter()
                .find(|tool| tool.name.as_ref() == tool_name)
                .unwrap();
            let schema = serde_json::to_value(&tool.input_schema).unwrap();
            assert_eq!(
                schema.pointer(pointer).and_then(Value::as_u64),
                Some(expected),
                "{tool_name}{pointer} must expose its runtime maximum"
            );
        }
    }

    #[test]
    fn reach_frequency_quote_is_visible_and_safely_annotated() {
        let tool = server()
            .tool_definitions()
            .into_iter()
            .find(|tool| tool.name == "create_reach_frequency_prediction")
            .unwrap();
        let annotations = tool.annotations.unwrap();
        assert_eq!(annotations.read_only_hint, Some(false));
        assert_eq!(annotations.destructive_hint, Some(false));
        assert_eq!(annotations.idempotent_hint, Some(false));
    }

    #[test]
    fn instructions_are_self_contained_within_codex_prefix() {
        let instructions = server().get_info().instructions.unwrap();
        assert!(instructions.chars().count() <= 512);
        assert!(instructions.contains("never place access tokens"));
        assert!(instructions.contains("reconcile unknown outcomes"));
        assert!(instructions.contains("next_cursor"));
    }

    #[tokio::test]
    async fn caller_errors_are_structured_and_marked_as_tool_errors() {
        let response = server()
            .list_ad_accounts(Parameters(ListAdAccountsInput {
                meta_user_id: None,
                page_size: None,
                page_cursor: None,
            }))
            .await
            .into_call_tool_result()
            .unwrap();

        let CallToolResponse::Complete(result) = response else {
            panic!("tool unexpectedly requested follow-up work");
        };
        assert_eq!(result.is_error, Some(true));
        let structured = result.structured_content.unwrap();
        assert_eq!(structured["status"], "error");
        assert_eq!(structured["error"]["code"], "AUTH_REQUIRED");
    }
}
