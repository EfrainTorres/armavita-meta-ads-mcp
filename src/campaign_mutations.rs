// Copyright (C) 2025 ArmaVita LLC
// SPDX-License-Identifier: AGPL-3.0-only

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{
        ad_account as normalize_ad_account_id, numeric_owned as normalize_numeric_id,
        numeric_value as normalize_numeric_value,
    },
    mutation_result::{ambiguous_mutation_result, mutation_error_without_blind_retry},
    node_identity::{MetaNodeKind, verify_meta_node},
};

const MAX_NAME_CHARS: usize = 256;
const MAX_RENAME_CHARS: usize = 100;
const MAX_SPECIAL_CATEGORIES: usize = 6;
const MAX_COUNTRIES: usize = 250;
const MAX_COPY_MAPPINGS: usize = 64;

/// Create a campaign. This is a non-destructive, non-idempotent external mutation: a
/// repeated call can create another campaign. The status defaults to `PAUSED`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCampaignInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    pub ad_account_id: String,
    /// Campaign name, from 1 through 256 characters.
    pub name: String,
    /// Current outcome-based Meta campaign objective.
    pub objective: CampaignObjective,
    /// Exactly one campaign- or ad-set-level budget strategy.
    pub budget: CampaignBudget,
    /// Delivery status. Defaults to `PAUSED`; use `ACTIVE` only to start delivery now.
    pub status: Option<CampaignCreateStatus>,
    /// Special-ad categories. Omit or use an empty list when none apply.
    pub special_ad_categories: Option<Vec<SpecialAdCategory>>,
    /// Two-letter country codes required by regulated special-ad categories.
    pub special_ad_category_countries: Option<Vec<String>>,
    /// Optional campaign bid strategy. Bid amounts belong on ad sets.
    pub bid_strategy: Option<CampaignBidStrategy>,
    /// Optional account-currency spend cap in minor units.
    pub spend_cap: Option<u32>,
}

/// Update one campaign. This is an idempotent external mutation for an unchanged
/// input. `ACTIVE` can begin delivery; `ARCHIVED` hides the campaign but is reversible.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateCampaignInput {
    /// Expected owner ad-account ID, with or without the `act_` prefix.
    #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
    pub ad_account_id: String,
    /// Numeric Meta campaign ID.
    pub campaign_id: String,
    /// New campaign name, from 1 through 256 characters.
    pub name: Option<String>,
    /// New delivery status. Deletion is intentionally not exposed here.
    pub status: Option<CampaignUpdateStatus>,
    /// New outcome-based campaign objective, subject to Meta's state restrictions.
    pub objective: Option<CampaignObjective>,
    /// Set one campaign-level budget amount.
    pub budget: Option<CampaignBudgetAmount>,
    /// Enable or disable budget sharing between child ad sets.
    pub is_adset_budget_sharing_enabled: Option<bool>,
    /// Replace special-ad categories. An empty list clears them.
    pub special_ad_categories: Option<Vec<SpecialAdCategory>>,
    /// Replace special-ad-category countries. An empty list clears them.
    pub special_ad_category_countries: Option<Vec<String>>,
    /// New campaign bid strategy. Bid amounts belong on ad sets.
    pub bid_strategy: Option<CampaignBidStrategy>,
    /// New account-currency spend cap in minor units.
    pub spend_cap: Option<u32>,
}

/// Copy a campaign through Meta's `/copies` edge. This is a non-destructive,
/// non-idempotent external mutation: a repeated call can create another copy. Copies
/// default to `PAUSED`; `deep_copy` also copies child ads within Meta's limits.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloneCampaignInput {
    /// Expected source ad-account ID, with or without the `act_` prefix.
    #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
    pub ad_account_id: String,
    /// Numeric source campaign ID.
    pub campaign_id: String,
    /// Copy child ads as well as the campaign. Defaults to false.
    pub deep_copy: Option<bool>,
    /// Status for the copy. Defaults to `PAUSED`.
    pub status: Option<CampaignCopyStatus>,
    /// Optional typed rename behavior. Omit for Meta's localized copy suffix.
    pub rename: Option<CampaignCopyRename>,
}

/// Create a high-demand budget schedule. This is a non-destructive, non-idempotent
/// external mutation: a repeated call can create another schedule.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCampaignBudgetScheduleInput {
    /// Expected owner ad-account ID, with or without the `act_` prefix.
    #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
    pub ad_account_id: String,
    /// Numeric Meta campaign ID.
    pub campaign_id: String,
    /// Absolute or multiplier budget adjustment.
    pub budget: CampaignBudgetScheduleValue,
    /// Inclusive Unix start timestamp accepted by Meta, greater than zero.
    pub time_start: u32,
    /// Unix end timestamp accepted by Meta; must be after `time_start`.
    pub time_end: u32,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
pub enum CampaignObjective {
    #[serde(rename = "OUTCOME_APP_PROMOTION")]
    AppPromotion,
    #[serde(rename = "OUTCOME_AWARENESS")]
    Awareness,
    #[serde(rename = "OUTCOME_ENGAGEMENT")]
    Engagement,
    #[serde(rename = "OUTCOME_LEADS")]
    Leads,
    #[serde(rename = "OUTCOME_SALES")]
    Sales,
    #[serde(rename = "OUTCOME_TRAFFIC")]
    Traffic,
}

impl CampaignObjective {
    const fn as_str(self) -> &'static str {
        match self {
            Self::AppPromotion => "OUTCOME_APP_PROMOTION",
            Self::Awareness => "OUTCOME_AWARENESS",
            Self::Engagement => "OUTCOME_ENGAGEMENT",
            Self::Leads => "OUTCOME_LEADS",
            Self::Sales => "OUTCOME_SALES",
            Self::Traffic => "OUTCOME_TRAFFIC",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CampaignBidStrategy {
    CostCap,
    LowestCostWithoutCap,
    LowestCostWithBidCap,
    LowestCostWithMinRoas,
}

impl CampaignBidStrategy {
    const fn as_str(self) -> &'static str {
        match self {
            Self::CostCap => "COST_CAP",
            Self::LowestCostWithoutCap => "LOWEST_COST_WITHOUT_CAP",
            Self::LowestCostWithBidCap => "LOWEST_COST_WITH_BID_CAP",
            Self::LowestCostWithMinRoas => "LOWEST_COST_WITH_MIN_ROAS",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SpecialAdCategory {
    Credit,
    Employment,
    FinancialProductsServices,
    Housing,
    IssuesElectionsPolitics,
    OnlineGamblingAndGaming,
}

impl SpecialAdCategory {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Credit => "CREDIT",
            Self::Employment => "EMPLOYMENT",
            Self::FinancialProductsServices => "FINANCIAL_PRODUCTS_SERVICES",
            Self::Housing => "HOUSING",
            Self::IssuesElectionsPolitics => "ISSUES_ELECTIONS_POLITICS",
            Self::OnlineGamblingAndGaming => "ONLINE_GAMBLING_AND_GAMING",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CampaignCreateStatus {
    Active,
    Paused,
}

impl CampaignCreateStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Paused => "PAUSED",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CampaignUpdateStatus {
    Active,
    Archived,
    Paused,
}

impl CampaignUpdateStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Archived => "ARCHIVED",
            Self::Paused => "PAUSED",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CampaignCopyStatus {
    Active,
    InheritedFromSource,
    Paused,
}

impl CampaignCopyStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::InheritedFromSource => "INHERITED_FROM_SOURCE",
            Self::Paused => "PAUSED",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CampaignBudget {
    /// Daily campaign budget in account-currency minor units.
    Daily { amount: u32 },
    /// Lifetime campaign budget in account-currency minor units.
    Lifetime { amount: u32 },
    /// Budgets are set on child ad sets instead of this campaign.
    AdSetLevel { share_across_ad_sets: bool },
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CampaignBudgetAmount {
    /// Daily campaign budget in account-currency minor units.
    Daily { amount: u32 },
    /// Lifetime campaign budget in account-currency minor units.
    Lifetime { amount: u32 },
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CampaignBudgetScheduleValue {
    /// Absolute adjustment in account-currency minor units.
    Absolute { value: u32 },
    /// Meta-defined integer multiplier value.
    Multiplier { value: u32 },
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CampaignCopyRename {
    /// Which copied object names Meta changes.
    pub strategy: CampaignRenameStrategy,
    /// Optional prefix, from 1 through 100 characters.
    pub prefix: Option<String>,
    /// Optional suffix, from 1 through 100 characters.
    pub suffix: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
pub enum CampaignRenameStrategy {
    #[serde(rename = "DEEP_RENAME")]
    Deep,
    #[serde(rename = "NO_RENAME")]
    None,
    #[serde(rename = "ONLY_TOP_LEVEL_RENAME")]
    TopLevel,
}

impl CampaignRenameStrategy {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Deep => "DEEP_RENAME",
            Self::None => "NO_RENAME",
            Self::TopLevel => "ONLY_TOP_LEVEL_RENAME",
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CreatedCampaign {
    pub campaign_id: String,
    pub status: CampaignCreateStatus,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UpdatedCampaign {
    pub campaign_id: String,
    pub updated: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ClonedCampaign {
    pub campaign_id: String,
    pub status: CampaignCopyStatus,
    pub copied_object_count: u16,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CreatedCampaignBudgetSchedule {
    pub schedule_id: String,
    pub campaign_id: String,
}

#[derive(Debug, PartialEq, Eq)]
struct MutationRequest {
    endpoint: String,
    form: Vec<(String, String)>,
}

pub(crate) async fn create_campaign(
    graph: &GraphClient,
    input: CreateCampaignInput,
) -> ToolResponse<CreatedCampaign> {
    let status = input.status.unwrap_or(CampaignCreateStatus::Paused);
    let request = match build_create_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(mutation_error_without_blind_retry(
                error,
                "Check Meta Ads Manager for the campaign before trying again",
            ));
        }
    };
    match created_id(&payload, "campaign") {
        Ok(campaign_id) => ToolResponse::success(CreatedCampaign {
            campaign_id,
            status,
        }),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn update_campaign(
    graph: &GraphClient,
    input: UpdateCampaignInput,
) -> ToolResponse<UpdatedCampaign> {
    let request = match build_update_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let campaign_id = request.endpoint.clone();
    if let Err(error) = verify_meta_node(
        graph,
        &campaign_id,
        MetaNodeKind::Campaign,
        Some(&input.ad_account_id),
    )
    .await
    {
        return ToolResponse::error(error);
    }
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    match confirmed_update(&payload, &campaign_id) {
        Ok(()) => ToolResponse::success(UpdatedCampaign {
            campaign_id,
            updated: true,
        }),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn clone_campaign(
    graph: &GraphClient,
    input: CloneCampaignInput,
) -> ToolResponse<ClonedCampaign> {
    let status = input.status.unwrap_or(CampaignCopyStatus::Paused);
    let request = match build_clone_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let campaign_id = request
        .endpoint
        .strip_suffix("/copies")
        .expect("validated static suffix");
    if let Err(error) = verify_meta_node(
        graph,
        campaign_id,
        MetaNodeKind::Campaign,
        Some(&input.ad_account_id),
    )
    .await
    {
        return ToolResponse::error(error);
    }
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(mutation_error_without_blind_retry(
                error,
                "Check Meta Ads Manager for the copied campaign before trying again",
            ));
        }
    };
    match cloned_campaign(&payload, status) {
        Ok(campaign) => ToolResponse::success(campaign),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn create_campaign_budget_schedule(
    graph: &GraphClient,
    input: CreateCampaignBudgetScheduleInput,
) -> ToolResponse<CreatedCampaignBudgetSchedule> {
    let request = match build_budget_schedule_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let campaign_id = request
        .endpoint
        .strip_suffix("/budget_schedules")
        .expect("validated static suffix")
        .to_owned();
    if let Err(error) = verify_meta_node(
        graph,
        &campaign_id,
        MetaNodeKind::Campaign,
        Some(&input.ad_account_id),
    )
    .await
    {
        return ToolResponse::error(error);
    }
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(mutation_error_without_blind_retry(
                error,
                "Inspect the campaign budget schedules before trying again",
            ));
        }
    };
    match created_id(&payload, "budget schedule") {
        Ok(schedule_id) => ToolResponse::success(CreatedCampaignBudgetSchedule {
            schedule_id,
            campaign_id,
        }),
        Err(error) => ToolResponse::error(error),
    }
}

fn build_create_request(input: &CreateCampaignInput) -> Result<MutationRequest, PublicError> {
    let account_id = normalize_ad_account_id(&input.ad_account_id).ok_or_else(|| {
        PublicError::invalid_input(
            "ad_account_id must be a numeric Meta account ID",
            "Use an ID such as `act_123456789` or `123456789`",
        )
    })?;
    let name = normalize_text(&input.name, MAX_NAME_CHARS, "name")?;
    let categories =
        normalize_categories(input.special_ad_categories.as_deref().unwrap_or_default())?;
    let countries = normalize_countries(
        input
            .special_ad_category_countries
            .as_deref()
            .unwrap_or_default(),
    )?;
    validate_category_countries(&categories, &countries, true)?;

    let mut form = vec![
        ("name".to_owned(), name),
        ("objective".to_owned(), input.objective.as_str().to_owned()),
        (
            "status".to_owned(),
            input
                .status
                .unwrap_or(CampaignCreateStatus::Paused)
                .as_str()
                .to_owned(),
        ),
        (
            "special_ad_categories".to_owned(),
            encode_string_list(&categories)?,
        ),
    ];
    if !countries.is_empty() {
        form.push((
            "special_ad_category_country".to_owned(),
            encode_string_list(&countries)?,
        ));
    }
    append_create_budget(&mut form, &input.budget)?;
    if let Some(strategy) = input.bid_strategy {
        form.push(("bid_strategy".to_owned(), strategy.as_str().to_owned()));
    }
    if let Some(spend_cap) = input.spend_cap {
        form.push(("spend_cap".to_owned(), spend_cap.to_string()));
    }

    Ok(MutationRequest {
        endpoint: format!("{account_id}/campaigns"),
        form,
    })
}

fn build_update_request(input: &UpdateCampaignInput) -> Result<MutationRequest, PublicError> {
    let campaign_id = normalize_numeric_id(&input.campaign_id).ok_or_else(|| {
        PublicError::invalid_input(
            "campaign_id must be a numeric Meta campaign ID",
            "Use the ID returned by list_campaigns",
        )
    })?;
    let categories = input
        .special_ad_categories
        .as_deref()
        .map(normalize_categories)
        .transpose()?;
    let countries = input
        .special_ad_category_countries
        .as_deref()
        .map(normalize_countries)
        .transpose()?;
    if let Some(categories) = &categories {
        validate_category_countries(categories, countries.as_deref().unwrap_or_default(), false)?;
    }

    let mut form = Vec::with_capacity(9);
    if let Some(name) = &input.name {
        form.push((
            "name".to_owned(),
            normalize_text(name, MAX_NAME_CHARS, "name")?,
        ));
    }
    if let Some(status) = input.status {
        form.push(("status".to_owned(), status.as_str().to_owned()));
    }
    if let Some(objective) = input.objective {
        form.push(("objective".to_owned(), objective.as_str().to_owned()));
    }
    if let Some(budget) = &input.budget {
        append_budget_amount(&mut form, budget)?;
    }
    if let Some(enabled) = input.is_adset_budget_sharing_enabled {
        form.push((
            "is_adset_budget_sharing_enabled".to_owned(),
            enabled.to_string(),
        ));
    }
    if let Some(categories) = categories {
        form.push((
            "special_ad_categories".to_owned(),
            encode_string_list(&categories)?,
        ));
    }
    if let Some(countries) = countries {
        form.push((
            "special_ad_category_country".to_owned(),
            encode_string_list(&countries)?,
        ));
    }
    if let Some(strategy) = input.bid_strategy {
        form.push(("bid_strategy".to_owned(), strategy.as_str().to_owned()));
    }
    if let Some(spend_cap) = input.spend_cap {
        form.push(("spend_cap".to_owned(), spend_cap.to_string()));
    }
    if form.is_empty() {
        return Err(PublicError::invalid_input(
            "at least one campaign field must be provided",
            "Set name, status, objective, budget, sharing, categories, bid strategy, or spend cap",
        ));
    }

    Ok(MutationRequest {
        endpoint: campaign_id,
        form,
    })
}

fn build_clone_request(input: &CloneCampaignInput) -> Result<MutationRequest, PublicError> {
    let campaign_id = normalize_numeric_id(&input.campaign_id).ok_or_else(|| {
        PublicError::invalid_input(
            "campaign_id must be a numeric Meta campaign ID",
            "Use the source ID returned by list_campaigns",
        )
    })?;
    let mut form = vec![
        (
            "deep_copy".to_owned(),
            input.deep_copy.unwrap_or(false).to_string(),
        ),
        (
            "status_option".to_owned(),
            input
                .status
                .unwrap_or(CampaignCopyStatus::Paused)
                .as_str()
                .to_owned(),
        ),
    ];
    if let Some(rename) = &input.rename {
        form.push(("rename_options".to_owned(), encode_rename_options(rename)?));
    }

    Ok(MutationRequest {
        endpoint: format!("{campaign_id}/copies"),
        form,
    })
}

fn build_budget_schedule_request(
    input: &CreateCampaignBudgetScheduleInput,
) -> Result<MutationRequest, PublicError> {
    let campaign_id = normalize_numeric_id(&input.campaign_id).ok_or_else(|| {
        PublicError::invalid_input(
            "campaign_id must be a numeric Meta campaign ID",
            "Use the ID returned by list_campaigns",
        )
    })?;
    if input.time_start == 0 || input.time_end <= input.time_start {
        return Err(PublicError::invalid_input(
            "time_start must be positive and time_end must be later",
            "Use an ordered Unix timestamp interval accepted by Meta",
        ));
    }
    let (value_type, value) = match input.budget {
        CampaignBudgetScheduleValue::Absolute { value } => ("ABSOLUTE", value),
        CampaignBudgetScheduleValue::Multiplier { value } => ("MULTIPLIER", value),
    };
    if value == 0 {
        return Err(PublicError::invalid_input(
            "budget schedule value must be greater than zero",
            "Provide a positive absolute or multiplier value",
        ));
    }

    Ok(MutationRequest {
        endpoint: format!("{campaign_id}/budget_schedules"),
        form: vec![
            ("budget_value".to_owned(), value.to_string()),
            ("budget_value_type".to_owned(), value_type.to_owned()),
            ("time_start".to_owned(), input.time_start.to_string()),
            ("time_end".to_owned(), input.time_end.to_string()),
        ],
    })
}

fn append_create_budget(
    form: &mut Vec<(String, String)>,
    budget: &CampaignBudget,
) -> Result<(), PublicError> {
    match budget {
        CampaignBudget::Daily { amount } => {
            ensure_positive_amount(*amount)?;
            form.push(("daily_budget".to_owned(), amount.to_string()));
        }
        CampaignBudget::Lifetime { amount } => {
            ensure_positive_amount(*amount)?;
            form.push(("lifetime_budget".to_owned(), amount.to_string()));
        }
        CampaignBudget::AdSetLevel {
            share_across_ad_sets,
        } => form.push((
            "is_adset_budget_sharing_enabled".to_owned(),
            share_across_ad_sets.to_string(),
        )),
    }
    Ok(())
}

fn append_budget_amount(
    form: &mut Vec<(String, String)>,
    budget: &CampaignBudgetAmount,
) -> Result<(), PublicError> {
    match budget {
        CampaignBudgetAmount::Daily { amount } => {
            ensure_positive_amount(*amount)?;
            form.push(("daily_budget".to_owned(), amount.to_string()));
        }
        CampaignBudgetAmount::Lifetime { amount } => {
            ensure_positive_amount(*amount)?;
            form.push(("lifetime_budget".to_owned(), amount.to_string()));
        }
    }
    Ok(())
}

fn ensure_positive_amount(amount: u32) -> Result<(), PublicError> {
    if amount == 0 {
        return Err(PublicError::invalid_input(
            "campaign budget must be greater than zero",
            "Use a positive account-currency minor-unit amount",
        ));
    }
    Ok(())
}

fn normalize_categories(values: &[SpecialAdCategory]) -> Result<Vec<&'static str>, PublicError> {
    if values.len() > MAX_SPECIAL_CATEGORIES {
        return Err(PublicError::invalid_input(
            "too many special-ad categories",
            "Provide at most the six distinct categories exposed by Meta v26",
        ));
    }
    let mut output = Vec::with_capacity(values.len());
    for category in values {
        let value = category.as_str();
        if !output.contains(&value) {
            output.push(value);
        }
    }
    Ok(output)
}

fn normalize_countries(values: &[String]) -> Result<Vec<String>, PublicError> {
    if values.len() > MAX_COUNTRIES {
        return Err(PublicError::invalid_input(
            "too many special-ad-category countries",
            "Provide at most 250 two-letter country codes",
        ));
    }
    let mut output = Vec::with_capacity(values.len());
    for country in values {
        let country = country.trim().to_ascii_uppercase();
        if country.len() != 2 || !country.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(PublicError::invalid_input(
                "special-ad-category countries must be two-letter codes",
                "Use uppercase ISO-style codes such as US or GB",
            ));
        }
        if !output.contains(&country) {
            output.push(country);
        }
    }
    Ok(output)
}

fn validate_category_countries(
    categories: &[&str],
    countries: &[String],
    reject_orphan_countries: bool,
) -> Result<(), PublicError> {
    if reject_orphan_countries && categories.is_empty() && !countries.is_empty() {
        return Err(PublicError::invalid_input(
            "special-ad-category countries require a special-ad category",
            "Remove the countries or select the applicable regulated category",
        ));
    }
    let requires_country = categories.iter().any(|category| {
        matches!(
            *category,
            "CREDIT" | "EMPLOYMENT" | "FINANCIAL_PRODUCTS_SERVICES" | "HOUSING"
        )
    });
    if requires_country && countries.is_empty() {
        return Err(PublicError::invalid_input(
            "the selected special-ad category requires at least one country",
            "Provide special_ad_category_countries such as [\"US\"]",
        ));
    }
    Ok(())
}

fn normalize_text(raw: &str, max_chars: usize, field: &str) -> Result<String, PublicError> {
    let value = raw.trim();
    if value.is_empty() || value.chars().count() > max_chars || value.chars().any(char::is_control)
    {
        return Err(PublicError::invalid_input(
            format!("{field} must contain 1 to {max_chars} characters without controls"),
            format!("Provide a shorter plain-text {field}"),
        ));
    }
    Ok(value.to_owned())
}

fn encode_string_list(values: &[impl AsRef<str>]) -> Result<String, PublicError> {
    let values = values.iter().map(AsRef::as_ref).collect::<Vec<_>>();
    serde_json::to_string(&values).map_err(|_| {
        PublicError::invalid_input(
            "campaign values could not be encoded",
            "Use plain enum values and two-letter country codes",
        )
    })
}

fn encode_rename_options(rename: &CampaignCopyRename) -> Result<String, PublicError> {
    #[derive(Serialize)]
    struct RenameOptions<'a> {
        rename_strategy: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        rename_prefix: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        rename_suffix: Option<&'a str>,
    }

    let prefix = rename
        .prefix
        .as_deref()
        .map(|value| normalize_affix(value, "rename prefix"))
        .transpose()?;
    let suffix = rename
        .suffix
        .as_deref()
        .map(|value| normalize_affix(value, "rename suffix"))
        .transpose()?;
    if matches!(rename.strategy, CampaignRenameStrategy::None)
        && (prefix.is_some() || suffix.is_some())
    {
        return Err(PublicError::invalid_input(
            "NO_RENAME cannot include a rename prefix or suffix",
            "Remove prefix and suffix or choose a renaming strategy",
        ));
    }
    serde_json::to_string(&RenameOptions {
        rename_strategy: rename.strategy.as_str(),
        rename_prefix: prefix.as_deref(),
        rename_suffix: suffix.as_deref(),
    })
    .map_err(|_| {
        PublicError::invalid_input(
            "rename options could not be encoded",
            "Use a supported strategy and bounded plain-text affixes",
        )
    })
}

fn normalize_affix(raw: &str, field: &str) -> Result<String, PublicError> {
    if raw.trim().is_empty()
        || raw.chars().count() > MAX_RENAME_CHARS
        || raw.chars().any(char::is_control)
    {
        return Err(PublicError::invalid_input(
            format!("{field} must contain 1 to {MAX_RENAME_CHARS} characters without controls"),
            format!("Provide a shorter plain-text {field}"),
        ));
    }
    Ok(raw.to_owned())
}

fn created_id(payload: &Value, resource: &str) -> Result<String, PublicError> {
    payload
        .get("id")
        .and_then(normalize_numeric_value)
        .ok_or_else(|| ambiguous_result(format!("Meta did not confirm the new {resource} ID")))
}

fn confirmed_update(payload: &Value, campaign_id: &str) -> Result<(), PublicError> {
    if payload.get("success").and_then(Value::as_bool) == Some(true)
        || payload
            .get("id")
            .and_then(normalize_numeric_value)
            .is_some_and(|id| id == campaign_id)
    {
        return Ok(());
    }
    Err(ambiguous_result("Meta did not confirm the campaign update"))
}

fn cloned_campaign(
    payload: &Value,
    status: CampaignCopyStatus,
) -> Result<ClonedCampaign, PublicError> {
    let campaign_id = payload
        .get("copied_campaign_id")
        .and_then(normalize_numeric_value)
        .ok_or_else(|| ambiguous_result("Meta did not confirm the copied campaign ID"))?;
    let copied_object_count = match payload.get("ad_object_ids") {
        None | Some(Value::Null) => 0,
        Some(Value::Array(values)) if values.len() <= MAX_COPY_MAPPINGS => {
            u16::try_from(values.len()).expect("copy mapping limit fits u16")
        }
        _ => {
            return Err(ambiguous_result(
                "Meta returned an invalid copied-object summary",
            ));
        }
    };
    Ok(ClonedCampaign {
        campaign_id,
        status,
        copied_object_count,
    })
}

fn ambiguous_result(message: impl Into<String>) -> PublicError {
    ambiguous_mutation_result(
        message,
        "Verify the result in Meta Ads Manager before retrying",
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        CampaignBidStrategy, CampaignBudget, CampaignBudgetAmount, CampaignBudgetScheduleValue,
        CampaignCopyRename, CampaignCopyStatus, CampaignCreateStatus, CampaignObjective,
        CampaignRenameStrategy, CampaignUpdateStatus, CloneCampaignInput,
        CreateCampaignBudgetScheduleInput, CreateCampaignInput, MAX_SPECIAL_CATEGORIES,
        SpecialAdCategory, UpdateCampaignInput, build_budget_schedule_request, build_clone_request,
        build_create_request, build_update_request, cloned_campaign, confirmed_update,
        normalize_categories,
    };

    #[test]
    fn builds_exact_paused_campaign_create_form() {
        let request = build_create_request(&CreateCampaignInput {
            ad_account_id: "123".to_owned(),
            name: "  Launch  ".to_owned(),
            objective: CampaignObjective::Sales,
            budget: CampaignBudget::Daily { amount: 2_500 },
            status: None,
            special_ad_categories: Some(vec![SpecialAdCategory::Housing]),
            special_ad_category_countries: Some(vec!["us".to_owned(), "US".to_owned()]),
            bid_strategy: Some(CampaignBidStrategy::LowestCostWithoutCap),
            spend_cap: Some(50_000),
        })
        .unwrap();

        assert_eq!(request.endpoint, "act_123/campaigns");
        assert_eq!(
            request.form,
            vec![
                ("name".to_owned(), "Launch".to_owned()),
                ("objective".to_owned(), "OUTCOME_SALES".to_owned()),
                ("status".to_owned(), "PAUSED".to_owned()),
                (
                    "special_ad_categories".to_owned(),
                    "[\"HOUSING\"]".to_owned(),
                ),
                (
                    "special_ad_category_country".to_owned(),
                    "[\"US\"]".to_owned(),
                ),
                ("daily_budget".to_owned(), "2500".to_owned()),
                (
                    "bid_strategy".to_owned(),
                    "LOWEST_COST_WITHOUT_CAP".to_owned(),
                ),
                ("spend_cap".to_owned(), "50000".to_owned()),
            ]
        );
    }

    #[test]
    fn create_requires_explicit_safe_budget_and_regulated_countries() {
        assert!(
            normalize_categories(&[SpecialAdCategory::Credit; MAX_SPECIAL_CATEGORIES + 1]).is_err()
        );

        let invalid_budget = CreateCampaignInput {
            ad_account_id: "act_123".to_owned(),
            name: "Launch".to_owned(),
            objective: CampaignObjective::Traffic,
            budget: CampaignBudget::Lifetime { amount: 0 },
            status: Some(CampaignCreateStatus::Active),
            special_ad_categories: None,
            special_ad_category_countries: None,
            bid_strategy: None,
            spend_cap: None,
        };
        assert!(build_create_request(&invalid_budget).is_err());

        let missing_country = CreateCampaignInput {
            budget: CampaignBudget::AdSetLevel {
                share_across_ad_sets: true,
            },
            special_ad_categories: Some(vec![SpecialAdCategory::FinancialProductsServices]),
            ..invalid_budget
        };
        assert!(build_create_request(&missing_country).is_err());
    }

    #[test]
    fn builds_exact_typed_campaign_update_form() {
        let request = build_update_request(&UpdateCampaignInput {
            ad_account_id: "act_123".to_owned(),
            campaign_id: "456".to_owned(),
            name: Some("Revised".to_owned()),
            status: Some(CampaignUpdateStatus::Paused),
            objective: Some(CampaignObjective::Leads),
            budget: Some(CampaignBudgetAmount::Lifetime { amount: 90_000 }),
            is_adset_budget_sharing_enabled: Some(false),
            special_ad_categories: Some(vec![]),
            special_ad_category_countries: Some(vec![]),
            bid_strategy: Some(CampaignBidStrategy::CostCap),
            spend_cap: Some(0),
        })
        .unwrap();

        assert_eq!(request.endpoint, "456");
        assert_eq!(
            request.form,
            vec![
                ("name".to_owned(), "Revised".to_owned()),
                ("status".to_owned(), "PAUSED".to_owned()),
                ("objective".to_owned(), "OUTCOME_LEADS".to_owned()),
                ("lifetime_budget".to_owned(), "90000".to_owned()),
                (
                    "is_adset_budget_sharing_enabled".to_owned(),
                    "false".to_owned(),
                ),
                ("special_ad_categories".to_owned(), "[]".to_owned()),
                ("special_ad_category_country".to_owned(), "[]".to_owned(),),
                ("bid_strategy".to_owned(), "COST_CAP".to_owned()),
                ("spend_cap".to_owned(), "0".to_owned()),
            ]
        );
        assert!(confirmed_update(&json!({"success": true}), "456").is_ok());
        assert!(confirmed_update(&json!({"id": "456"}), "456").is_ok());
        assert!(confirmed_update(&json!({"success": false}), "456").is_err());
    }

    #[test]
    fn update_rejects_empty_or_unsafe_mutations() {
        let empty = UpdateCampaignInput {
            ad_account_id: "act_123".to_owned(),
            campaign_id: "456".to_owned(),
            name: None,
            status: None,
            objective: None,
            budget: None,
            is_adset_budget_sharing_enabled: None,
            special_ad_categories: None,
            special_ad_category_countries: None,
            bid_strategy: None,
            spend_cap: None,
        };
        assert!(build_update_request(&empty).is_err());

        let unsafe_id = UpdateCampaignInput {
            campaign_id: "../456".to_owned(),
            status: Some(CampaignUpdateStatus::Archived),
            ..empty
        };
        assert!(build_update_request(&unsafe_id).is_err());
    }

    #[test]
    fn builds_exact_paused_copy_request_and_compact_result() {
        let request = build_clone_request(&CloneCampaignInput {
            ad_account_id: "act_123".to_owned(),
            campaign_id: "789".to_owned(),
            deep_copy: Some(true),
            status: None,
            rename: Some(CampaignCopyRename {
                strategy: CampaignRenameStrategy::Deep,
                prefix: None,
                suffix: Some(" - Q4".to_owned()),
            }),
        })
        .unwrap();
        assert_eq!(request.endpoint, "789/copies");
        assert_eq!(
            request.form,
            vec![
                ("deep_copy".to_owned(), "true".to_owned()),
                ("status_option".to_owned(), "PAUSED".to_owned()),
                (
                    "rename_options".to_owned(),
                    "{\"rename_strategy\":\"DEEP_RENAME\",\"rename_suffix\":\" - Q4\"}".to_owned(),
                ),
            ]
        );

        let result = cloned_campaign(
            &json!({
                "copied_campaign_id": "900",
                "ad_object_ids": [{"copied_id": "901"}, {"copied_id": "902"}]
            }),
            CampaignCopyStatus::Paused,
        )
        .unwrap();
        assert_eq!(result.campaign_id, "900");
        assert_eq!(result.copied_object_count, 2);
    }

    #[test]
    fn no_rename_rejects_conflicting_affixes() {
        let input = CloneCampaignInput {
            ad_account_id: "act_123".to_owned(),
            campaign_id: "789".to_owned(),
            deep_copy: None,
            status: Some(CampaignCopyStatus::InheritedFromSource),
            rename: Some(CampaignCopyRename {
                strategy: CampaignRenameStrategy::None,
                prefix: Some("Copy".to_owned()),
                suffix: None,
            }),
        };
        assert!(build_clone_request(&input).is_err());
    }

    #[test]
    fn builds_exact_budget_schedule_request_and_validates_interval() {
        let request = build_budget_schedule_request(&CreateCampaignBudgetScheduleInput {
            ad_account_id: "act_123".to_owned(),
            campaign_id: "321".to_owned(),
            budget: CampaignBudgetScheduleValue::Multiplier { value: 150 },
            time_start: 2_000_000_000,
            time_end: 2_000_003_600,
        })
        .unwrap();
        assert_eq!(request.endpoint, "321/budget_schedules");
        assert_eq!(
            request.form,
            vec![
                ("budget_value".to_owned(), "150".to_owned()),
                ("budget_value_type".to_owned(), "MULTIPLIER".to_owned()),
                ("time_start".to_owned(), "2000000000".to_owned()),
                ("time_end".to_owned(), "2000003600".to_owned()),
            ]
        );

        assert!(
            build_budget_schedule_request(&CreateCampaignBudgetScheduleInput {
                ad_account_id: "act_123".to_owned(),
                campaign_id: "321".to_owned(),
                budget: CampaignBudgetScheduleValue::Absolute { value: 1_000 },
                time_start: 10,
                time_end: 10,
            })
            .is_err()
        );
    }
}
