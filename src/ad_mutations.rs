// Copyright (C) 2025 ArmaVita LLC
// SPDX-License-Identifier: AGPL-3.0-only

use reqwest::Url;
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    bounded_json::credential_value,
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{
        ad_account, numeric_owned as normalize_numeric_id, numeric_value as normalize_numeric_value,
    },
    mutation_result::{ambiguous_mutation_result, mutation_error_without_blind_retry},
    node_identity::{MetaNodeKind, verify_meta_node},
};

const MAX_NAME_CHARS: usize = 256;
const MAX_PRIMARY_TEXT_CHARS: usize = 2_000;
const MAX_HEADLINE_CHARS: usize = 255;
const MAX_DESCRIPTION_CHARS: usize = 1_000;
const MAX_URL_CHARS: usize = 4_096;
const MAX_URL_TAGS_CHARS: usize = 2_048;
const MAX_HASH_CHARS: usize = 128;
const MAX_TEXT_VARIANTS: usize = 5;
const MAX_FLEX_IMAGES: usize = 10;
const MIN_CAROUSEL_CARDS: usize = 2;
const MAX_CAROUSEL_CARDS: usize = 10;
const MAX_RENAME_CHARS: usize = 100;
const MAX_NESTED_SPEC_BYTES: usize = 64 * 1024;

/// Create an ad from an existing creative. This non-idempotent external mutation
/// defaults to `PAUSED`; a repeated call can create another ad.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAdInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
    pub ad_account_id: String,
    /// Ad name, from 1 through 256 characters.
    #[schemars(length(min = 1, max = 256))]
    pub name: String,
    /// Numeric destination ad-set ID.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub ad_set_id: String,
    /// Numeric existing ad-creative ID.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub ad_creative_id: String,
    /// Delivery status. Defaults to `PAUSED`; `ACTIVE` can begin delivery.
    #[serde(default)]
    #[schemars(default)]
    pub status: AdCreateStatus,
    /// Optional bid amount in account-currency minor units.
    #[schemars(range(min = 1))]
    pub bid_amount: Option<u32>,
    /// Optional website conversion domain without a scheme or path.
    #[schemars(length(min = 1, max = 253))]
    pub conversion_domain: Option<String>,
    /// Optional bounded pixel tracking for offsite conversions.
    pub pixel_tracking: Option<PixelTracking>,
}

/// Update stable writable ad fields. This is idempotent for an unchanged input;
/// setting `ACTIVE` can begin delivery.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateAdInput {
    /// Expected owner ad-account ID, with or without the `act_` prefix.
    #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
    pub ad_account_id: String,
    /// Numeric Meta ad ID.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub ad_id: String,
    /// New ad name, from 1 through 256 characters.
    #[schemars(length(min = 1, max = 256))]
    pub name: Option<String>,
    /// New delivery status. Deletion is intentionally not exposed here.
    pub status: Option<AdUpdateStatus>,
    /// New bid amount in account-currency minor units.
    #[schemars(range(min = 1))]
    pub bid_amount: Option<u32>,
    /// New website conversion domain without a scheme or path.
    #[schemars(length(min = 1, max = 253))]
    pub conversion_domain: Option<String>,
    /// Swap to this numeric existing ad-creative ID.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub ad_creative_id: Option<String>,
    /// Replace tracking with bounded offsite-conversion pixel tracking.
    pub pixel_tracking: Option<PixelTracking>,
}

/// Copy an ad through Meta's v26 `/copies` edge. This non-idempotent mutation
/// defaults to `PAUSED`; a repeated call can create another copy.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloneAdInput {
    /// Expected source ad-account ID, with or without the `act_` prefix.
    #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
    pub ad_account_id: String,
    /// Numeric source ad ID.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub ad_id: String,
    /// Optional numeric destination ad-set ID.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub target_ad_set_id: Option<String>,
    /// Status for the copy. Defaults to `PAUSED`; other values can begin delivery.
    #[serde(default)]
    #[schemars(default)]
    pub status: AdCopyStatus,
    /// Optional copy-name suffix. Omit for Meta's localized default.
    #[schemars(length(min = 1, max = 100))]
    pub name_suffix: Option<String>,
}

/// Create an independently reusable ad creative. This non-idempotent external
/// mutation can create another creative if retried after an ambiguous failure.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAdCreativeInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
    pub ad_account_id: String,
    /// Creative name, from 1 through 256 characters.
    #[schemars(length(min = 1, max = 256))]
    pub name: String,
    /// One closed, typed creative content mode.
    pub content: AdCreativeContent,
    /// Optional query-string tags without a leading `?`; never include secrets.
    #[schemars(length(min = 1, max = 2048))]
    pub url_tags: Option<String>,
}

/// Rename an existing creative. Meta v26 does not permit content/spec updates on
/// the AdCreative node; create a new creative and swap the ad instead.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateAdCreativeInput {
    /// Expected owner ad-account ID, with or without the `act_` prefix.
    #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
    pub ad_account_id: String,
    /// Numeric Meta ad-creative ID.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub ad_creative_id: String,
    /// New creative name, from 1 through 256 characters.
    #[schemars(length(min = 1, max = 256))]
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PixelTracking {
    /// Numeric Meta pixel/dataset ID for `offsite_conversion` tracking.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub pixel_id: String,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AdCreateStatus {
    Active,
    #[default]
    Paused,
}

impl AdCreateStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Paused => "PAUSED",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AdUpdateStatus {
    Active,
    Archived,
    Paused,
}

impl AdUpdateStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Archived => "ARCHIVED",
            Self::Paused => "PAUSED",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AdCopyStatus {
    Active,
    InheritedFromSource,
    #[default]
    Paused,
}

impl AdCopyStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::InheritedFromSource => "INHERITED_FROM_SOURCE",
            Self::Paused => "PAUSED",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AdCreativeContent {
    /// Reuse an existing Page post identified as `page_id_post_id`.
    ExistingPost {
        #[schemars(
            length(min = 3, max = 129),
            regex(pattern = "^[0-9]{1,64}_[0-9]{1,64}$")
        )]
        object_story_id: String,
    },
    /// Reuse authorized existing Instagram media, including partnership content.
    InstagramMedia {
        #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
        source_instagram_media_id: String,
    },
    /// One image with link/story copy.
    Image {
        #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
        page_id: String,
        #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
        instagram_user_id: Option<String>,
        image: CreativeImageSource,
        #[schemars(length(min = 1, max = 4096), url)]
        link_url: String,
        #[schemars(length(min = 1, max = 2000))]
        primary_text: Option<String>,
        #[schemars(length(min = 1, max = 255))]
        headline: Option<String>,
        #[schemars(length(min = 1, max = 1000))]
        description: Option<String>,
        call_to_action: Option<CreativeCallToAction>,
    },
    /// One existing Meta video with a required link call to action.
    Video {
        #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
        page_id: String,
        #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
        instagram_user_id: Option<String>,
        #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
        video_id: String,
        #[schemars(length(min = 1, max = 4096), url)]
        thumbnail_url: Option<String>,
        #[schemars(length(min = 1, max = 4096), url)]
        link_url: String,
        #[schemars(length(min = 1, max = 2000))]
        primary_text: Option<String>,
        #[schemars(length(min = 1, max = 255))]
        headline: Option<String>,
        #[schemars(length(min = 1, max = 1000))]
        description: Option<String>,
        call_to_action: CreativeCallToAction,
    },
    /// A bounded two-to-ten-card link carousel.
    Carousel {
        #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
        page_id: String,
        #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
        instagram_user_id: Option<String>,
        #[schemars(length(min = 1, max = 4096), url)]
        link_url: String,
        #[schemars(length(min = 1, max = 2000))]
        primary_text: Option<String>,
        call_to_action: Option<CreativeCallToAction>,
        #[schemars(length(min = 2, max = 10))]
        cards: Vec<CarouselCard>,
    },
    /// Bounded image and text variants using the documented v26 asset-feed shape.
    FlexibleImage {
        #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
        page_id: String,
        #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
        instagram_user_id: Option<String>,
        #[schemars(length(min = 1, max = 4096), url)]
        link_url: String,
        #[schemars(length(min = 1, max = 10))]
        images: Vec<CreativeImageSource>,
        #[schemars(length(min = 1, max = 5), inner(length(min = 1, max = 2000)))]
        primary_texts: Vec<String>,
        #[serde(default)]
        #[schemars(default, length(max = 5), inner(length(min = 1, max = 255)))]
        headlines: Vec<String>,
        #[serde(default)]
        #[schemars(default, length(max = 5), inner(length(min = 1, max = 1000)))]
        descriptions: Vec<String>,
        call_to_action: Option<CreativeCallToActionType>,
    },
    /// A product-set creative with one closed catalog template.
    Catalog {
        #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
        page_id: String,
        #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
        instagram_user_id: Option<String>,
        #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
        product_set_id: String,
        #[schemars(length(min = 1, max = 4096), url)]
        link_url: String,
        #[schemars(length(min = 1, max = 2000))]
        primary_text: Option<String>,
        #[schemars(length(min = 1, max = 255))]
        headline: Option<String>,
        #[schemars(length(min = 1, max = 1000))]
        description: Option<String>,
        call_to_action: Option<CreativeCallToActionType>,
    },
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CreativeImageSource {
    /// Reuse an image from the ad account's image library.
    Hash {
        #[schemars(length(min = 1, max = 128), regex(pattern = "^[A-Za-z0-9_-]{1,128}$"))]
        hash: String,
    },
    /// Ask Meta to fetch an HTTPS image URL.
    Url {
        #[schemars(length(min = 1, max = 4096), url)]
        url: String,
    },
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CarouselCard {
    /// Card destination URL.
    #[schemars(length(min = 1, max = 4096), url)]
    pub link_url: String,
    /// Card headline.
    #[schemars(length(min = 1, max = 255))]
    pub headline: String,
    /// Optional card description.
    #[schemars(length(min = 1, max = 1000))]
    pub description: Option<String>,
    /// Exactly one card media source.
    pub media: CarouselMedia,
    /// Optional card-specific call to action.
    pub call_to_action: Option<CreativeCallToAction>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CarouselMedia {
    Image {
        #[schemars(length(min = 1, max = 128), regex(pattern = "^[A-Za-z0-9_-]{1,128}$"))]
        hash: String,
    },
    Video {
        #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
        video_id: String,
    },
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreativeCallToAction {
    /// Button type from the bounded commonly used v26 set.
    pub button: CreativeCallToActionType,
    /// Optional numeric lead-generation form ID.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub lead_form_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CreativeCallToActionType {
    AddToCart,
    ApplyNow,
    BookNow,
    BuyNow,
    ContactUs,
    DonateNow,
    Download,
    GetOffer,
    GetQuote,
    LearnMore,
    OrderNow,
    ShopNow,
    SignUp,
    Subscribe,
    WatchMore,
}

impl CreativeCallToActionType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::AddToCart => "ADD_TO_CART",
            Self::ApplyNow => "APPLY_NOW",
            Self::BookNow => "BOOK_NOW",
            Self::BuyNow => "BUY_NOW",
            Self::ContactUs => "CONTACT_US",
            Self::DonateNow => "DONATE_NOW",
            Self::Download => "DOWNLOAD",
            Self::GetOffer => "GET_OFFER",
            Self::GetQuote => "GET_QUOTE",
            Self::LearnMore => "LEARN_MORE",
            Self::OrderNow => "ORDER_NOW",
            Self::ShopNow => "SHOP_NOW",
            Self::SignUp => "SIGN_UP",
            Self::Subscribe => "SUBSCRIBE",
            Self::WatchMore => "WATCH_MORE",
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CreatedAd {
    pub ad_id: String,
    /// Requested configured status, not effective review/delivery state.
    pub status: AdCreateStatus,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UpdatedAd {
    pub ad_id: String,
    pub updated: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ClonedAd {
    pub ad_id: String,
    /// Requested configured status, not effective review/delivery state.
    pub status: AdCopyStatus,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CreatedAdCreative {
    pub ad_creative_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UpdatedAdCreative {
    pub ad_creative_id: String,
    pub updated: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct MutationRequest {
    endpoint: String,
    form: Vec<(String, String)>,
}

#[derive(Debug, Serialize)]
struct CreativeReference<'a> {
    creative_id: &'a str,
}

#[derive(Debug, Serialize)]
struct RawStorySpec {
    page_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    instagram_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    link_data: Option<RawLinkData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    video_data: Option<RawVideoData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    template_data: Option<RawTemplateData>,
}

#[derive(Debug, Serialize)]
struct RawTemplateData {
    link: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    call_to_action: Option<RawCallToActionType>,
}

#[derive(Debug, Serialize)]
struct RawCallToActionType {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Debug, Serialize)]
struct RawLinkData {
    link: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    picture: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    call_to_action: Option<RawCallToAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    child_attachments: Option<Vec<RawCarouselCard>>,
}

#[derive(Debug, Serialize)]
struct RawVideoData {
    video_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    link_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    call_to_action: Option<RawCallToAction>,
}

#[derive(Debug, Serialize)]
struct RawCarouselCard {
    link: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    video_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    call_to_action: Option<RawCallToAction>,
}

#[derive(Debug, Serialize)]
struct RawCallToAction {
    #[serde(rename = "type")]
    kind: &'static str,
    value: RawCallToActionValue,
}

#[derive(Debug, Serialize)]
struct RawCallToActionValue {
    link: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    lead_gen_form_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct RawAssetFeedSpec {
    images: Vec<RawAssetImage>,
    bodies: Vec<RawTextAsset>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    titles: Vec<RawTextAsset>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    descriptions: Vec<RawTextAsset>,
    link_urls: Vec<RawLinkAsset>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    call_to_action_types: Vec<&'static str>,
    ad_formats: [&'static str; 1],
}

#[derive(Debug, Serialize)]
struct RawAssetImage {
    #[serde(skip_serializing_if = "Option::is_none")]
    hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

#[derive(Debug, Serialize)]
struct RawTextAsset {
    text: String,
}

#[derive(Debug, Serialize)]
struct RawLinkAsset {
    website_url: String,
}

pub(crate) async fn create_ad(
    graph: &GraphClient,
    input: CreateAdInput,
) -> ToolResponse<CreatedAd> {
    let status = input.status;
    let request = match build_create_ad_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(mutation_error_without_blind_retry(
                error,
                "Check Meta Ads Manager for the ad before trying again",
            ));
        }
    };
    match created_id(&payload, "ad") {
        Ok(ad_id) => ToolResponse::success(CreatedAd { ad_id, status }),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn update_ad(
    graph: &GraphClient,
    input: UpdateAdInput,
) -> ToolResponse<UpdatedAd> {
    let request = match build_update_ad_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let ad_id = request.endpoint.clone();
    if let Err(error) =
        verify_meta_node(graph, &ad_id, MetaNodeKind::Ad, Some(&input.ad_account_id)).await
    {
        return ToolResponse::error(error);
    }
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    match confirmed_update(&payload, &ad_id, "ad") {
        Ok(()) => ToolResponse::success(UpdatedAd {
            ad_id,
            updated: true,
        }),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn clone_ad(graph: &GraphClient, input: CloneAdInput) -> ToolResponse<ClonedAd> {
    let status = input.status;
    let request = match build_clone_ad_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let ad_id = request
        .endpoint
        .strip_suffix("/copies")
        .expect("validated static suffix");
    if let Err(error) =
        verify_meta_node(graph, ad_id, MetaNodeKind::Ad, Some(&input.ad_account_id)).await
    {
        return ToolResponse::error(error);
    }
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(mutation_error_without_blind_retry(
                error,
                "Check Meta Ads Manager for the copied ad before trying again",
            ));
        }
    };
    match copied_ad_id(&payload) {
        Ok(ad_id) => ToolResponse::success(ClonedAd { ad_id, status }),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn create_ad_creative(
    graph: &GraphClient,
    input: CreateAdCreativeInput,
) -> ToolResponse<CreatedAdCreative> {
    let request = match build_create_creative_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(mutation_error_without_blind_retry(
                error,
                "Check the Meta creative library before trying again",
            ));
        }
    };
    match created_id(&payload, "ad creative") {
        Ok(ad_creative_id) => ToolResponse::success(CreatedAdCreative { ad_creative_id }),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn update_ad_creative(
    graph: &GraphClient,
    input: UpdateAdCreativeInput,
) -> ToolResponse<UpdatedAdCreative> {
    let request = match build_update_creative_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let ad_creative_id = request.endpoint.clone();
    if let Err(error) = verify_meta_node(
        graph,
        &ad_creative_id,
        MetaNodeKind::AdCreative,
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
    match confirmed_update(&payload, &ad_creative_id, "ad creative") {
        Ok(()) => ToolResponse::success(UpdatedAdCreative {
            ad_creative_id,
            updated: true,
        }),
        Err(error) => ToolResponse::error(error),
    }
}

fn build_create_ad_request(input: &CreateAdInput) -> Result<MutationRequest, PublicError> {
    let account_id = required_account_id(&input.ad_account_id)?;
    let ad_set_id = required_id(&input.ad_set_id, "ad_set_id", "Use an ID returned by Meta")?;
    let creative_id = required_id(
        &input.ad_creative_id,
        "ad_creative_id",
        "Use the ID returned by create_ad_creative",
    )?;
    let mut form = vec![
        (
            "name".to_owned(),
            required_text(&input.name, MAX_NAME_CHARS, "name")?,
        ),
        ("adset_id".to_owned(), ad_set_id),
        (
            "creative".to_owned(),
            encode_nested(&CreativeReference {
                creative_id: &creative_id,
            })?,
        ),
        ("status".to_owned(), input.status.as_str().to_owned()),
    ];
    append_ad_options(
        &mut form,
        input.bid_amount,
        input.conversion_domain.as_deref(),
        input.pixel_tracking.as_ref(),
    )?;
    Ok(MutationRequest {
        endpoint: format!("{account_id}/ads"),
        form,
    })
}

fn build_update_ad_request(input: &UpdateAdInput) -> Result<MutationRequest, PublicError> {
    let ad_id = required_id(&input.ad_id, "ad_id", "Use the ID returned by list_ads")?;
    let mut form = Vec::with_capacity(6);
    if let Some(name) = &input.name {
        form.push((
            "name".to_owned(),
            required_text(name, MAX_NAME_CHARS, "name")?,
        ));
    }
    if let Some(status) = input.status {
        form.push(("status".to_owned(), status.as_str().to_owned()));
    }
    append_ad_options(
        &mut form,
        input.bid_amount,
        input.conversion_domain.as_deref(),
        input.pixel_tracking.as_ref(),
    )?;
    if let Some(creative_id) = &input.ad_creative_id {
        let creative_id = required_id(
            creative_id,
            "ad_creative_id",
            "Use the ID returned by create_ad_creative",
        )?;
        form.push((
            "creative".to_owned(),
            encode_nested(&CreativeReference {
                creative_id: &creative_id,
            })?,
        ));
    }
    if form.is_empty() {
        return Err(PublicError::invalid_input(
            "at least one ad field must be provided",
            "Set name, status, bid amount, conversion domain, creative, or pixel tracking",
        ));
    }
    Ok(MutationRequest {
        endpoint: ad_id,
        form,
    })
}

fn build_clone_ad_request(input: &CloneAdInput) -> Result<MutationRequest, PublicError> {
    let ad_id = required_id(
        &input.ad_id,
        "ad_id",
        "Use the source ID returned by list_ads",
    )?;
    let mut form = vec![("status_option".to_owned(), input.status.as_str().to_owned())];
    if let Some(ad_set_id) = &input.target_ad_set_id {
        form.push((
            "adset_id".to_owned(),
            required_id(
                ad_set_id,
                "target_ad_set_id",
                "Use a numeric Meta ad-set ID",
            )?,
        ));
    }
    if let Some(suffix) = &input.name_suffix {
        let suffix = bounded_affix(suffix)?;
        #[derive(Serialize)]
        struct RenameOptions<'a> {
            rename_strategy: &'static str,
            rename_suffix: &'a str,
        }
        form.push((
            "rename_options".to_owned(),
            encode_nested(&RenameOptions {
                rename_strategy: "ONLY_TOP_LEVEL_RENAME",
                rename_suffix: &suffix,
            })?,
        ));
    }
    Ok(MutationRequest {
        endpoint: format!("{ad_id}/copies"),
        form,
    })
}

fn build_create_creative_request(
    input: &CreateAdCreativeInput,
) -> Result<MutationRequest, PublicError> {
    let account_id = required_account_id(&input.ad_account_id)?;
    let mut form = vec![(
        "name".to_owned(),
        required_text(&input.name, MAX_NAME_CHARS, "name")?,
    )];
    append_creative_content(&mut form, &input.content)?;
    if let Some(url_tags) = &input.url_tags {
        form.push(("url_tags".to_owned(), bounded_url_tags(url_tags)?));
    }
    Ok(MutationRequest {
        endpoint: format!("{account_id}/adcreatives"),
        form,
    })
}

fn build_update_creative_request(
    input: &UpdateAdCreativeInput,
) -> Result<MutationRequest, PublicError> {
    let creative_id = required_id(
        &input.ad_creative_id,
        "ad_creative_id",
        "Use the ID returned by list_ad_creatives",
    )?;
    Ok(MutationRequest {
        endpoint: creative_id,
        form: vec![(
            "name".to_owned(),
            required_text(&input.name, MAX_NAME_CHARS, "name")?,
        )],
    })
}

fn append_ad_options(
    form: &mut Vec<(String, String)>,
    bid_amount: Option<u32>,
    conversion_domain: Option<&str>,
    pixel_tracking: Option<&PixelTracking>,
) -> Result<(), PublicError> {
    if let Some(bid_amount) = bid_amount {
        if bid_amount == 0 {
            return Err(PublicError::invalid_input(
                "bid_amount must be greater than zero",
                "Use a positive account-currency minor-unit amount",
            ));
        }
        form.push(("bid_amount".to_owned(), bid_amount.to_string()));
    }
    if let Some(domain) = conversion_domain {
        form.push(("conversion_domain".to_owned(), normalized_domain(domain)?));
    }
    if let Some(tracking) = pixel_tracking {
        let pixel_id = required_id(
            &tracking.pixel_id,
            "pixel_tracking.pixel_id",
            "Use a numeric pixel/dataset ID",
        )?;
        #[derive(Serialize)]
        struct TrackingSpec<'a> {
            #[serde(rename = "action.type")]
            action_type: [&'static str; 1],
            fb_pixel: [&'a str; 1],
        }
        form.push((
            "tracking_specs".to_owned(),
            encode_nested(&[TrackingSpec {
                action_type: ["offsite_conversion"],
                fb_pixel: [&pixel_id],
            }])?,
        ));
    }
    Ok(())
}

fn append_creative_content(
    form: &mut Vec<(String, String)>,
    content: &AdCreativeContent,
) -> Result<(), PublicError> {
    match content {
        AdCreativeContent::ExistingPost { object_story_id } => {
            form.push((
                "object_story_id".to_owned(),
                normalized_story_id(object_story_id)?,
            ));
        }
        AdCreativeContent::InstagramMedia {
            source_instagram_media_id,
        } => {
            form.push((
                "source_instagram_media_id".to_owned(),
                required_id(
                    source_instagram_media_id,
                    "source_instagram_media_id",
                    "Use an authorized media ID returned by the partnership or Instagram reads",
                )?,
            ));
        }
        AdCreativeContent::Image {
            page_id,
            instagram_user_id,
            image,
            link_url,
            primary_text,
            headline,
            description,
            call_to_action,
        } => {
            let link = https_url(link_url, "link_url")?;
            let (image_hash, picture) = image_link_fields(image)?;
            let story = RawStorySpec {
                page_id: required_id(page_id, "page_id", "Use a numeric Facebook Page ID")?,
                instagram_user_id: optional_id(instagram_user_id.as_deref(), "instagram_user_id")?,
                link_data: Some(RawLinkData {
                    link: link.clone(),
                    image_hash,
                    picture,
                    message: optional_copy_text(
                        primary_text.as_deref(),
                        MAX_PRIMARY_TEXT_CHARS,
                        "primary_text",
                    )?,
                    name: optional_text(headline.as_deref(), MAX_HEADLINE_CHARS, "headline")?,
                    description: optional_text(
                        description.as_deref(),
                        MAX_DESCRIPTION_CHARS,
                        "description",
                    )?,
                    call_to_action: raw_call_to_action(call_to_action.as_ref(), &link)?,
                    child_attachments: None,
                }),
                video_data: None,
                template_data: None,
            };
            form.push(("object_story_spec".to_owned(), encode_nested(&story)?));
        }
        AdCreativeContent::Video {
            page_id,
            instagram_user_id,
            video_id,
            thumbnail_url,
            link_url,
            primary_text,
            headline,
            description,
            call_to_action,
        } => {
            let link = https_url(link_url, "link_url")?;
            let story = RawStorySpec {
                page_id: required_id(page_id, "page_id", "Use a numeric Facebook Page ID")?,
                instagram_user_id: optional_id(instagram_user_id.as_deref(), "instagram_user_id")?,
                link_data: None,
                video_data: Some(RawVideoData {
                    video_id: required_id(video_id, "video_id", "Use a numeric Meta video ID")?,
                    image_url: thumbnail_url
                        .as_deref()
                        .map(|url| https_url(url, "thumbnail_url"))
                        .transpose()?,
                    message: optional_copy_text(
                        primary_text.as_deref(),
                        MAX_PRIMARY_TEXT_CHARS,
                        "primary_text",
                    )?,
                    title: optional_text(headline.as_deref(), MAX_HEADLINE_CHARS, "headline")?,
                    link_description: optional_text(
                        description.as_deref(),
                        MAX_DESCRIPTION_CHARS,
                        "description",
                    )?,
                    call_to_action: raw_call_to_action(Some(call_to_action), &link)?,
                }),
                template_data: None,
            };
            form.push(("object_story_spec".to_owned(), encode_nested(&story)?));
        }
        AdCreativeContent::Carousel {
            page_id,
            instagram_user_id,
            link_url,
            primary_text,
            call_to_action,
            cards,
        } => {
            if !(MIN_CAROUSEL_CARDS..=MAX_CAROUSEL_CARDS).contains(&cards.len()) {
                return Err(PublicError::invalid_input(
                    "carousel cards must contain between 2 and 10 items",
                    "Provide a bounded Meta carousel",
                ));
            }
            let link = https_url(link_url, "link_url")?;
            let mut raw_cards = Vec::with_capacity(cards.len());
            for card in cards {
                let card_link = https_url(&card.link_url, "cards.link_url")?;
                let (image_hash, video_id) = match &card.media {
                    CarouselMedia::Image { hash } => (Some(normalized_hash(hash)?), None),
                    CarouselMedia::Video { video_id } => (
                        None,
                        Some(required_id(
                            video_id,
                            "cards.media.video_id",
                            "Use a numeric Meta video ID",
                        )?),
                    ),
                };
                raw_cards.push(RawCarouselCard {
                    link: card_link.clone(),
                    name: required_text(&card.headline, MAX_HEADLINE_CHARS, "cards.headline")?,
                    description: optional_text(
                        card.description.as_deref(),
                        MAX_DESCRIPTION_CHARS,
                        "cards.description",
                    )?,
                    image_hash,
                    video_id,
                    call_to_action: raw_call_to_action(card.call_to_action.as_ref(), &card_link)?,
                });
            }
            let story = RawStorySpec {
                page_id: required_id(page_id, "page_id", "Use a numeric Facebook Page ID")?,
                instagram_user_id: optional_id(instagram_user_id.as_deref(), "instagram_user_id")?,
                link_data: Some(RawLinkData {
                    link: link.clone(),
                    image_hash: None,
                    picture: None,
                    message: optional_copy_text(
                        primary_text.as_deref(),
                        MAX_PRIMARY_TEXT_CHARS,
                        "primary_text",
                    )?,
                    name: None,
                    description: None,
                    call_to_action: raw_call_to_action(call_to_action.as_ref(), &link)?,
                    child_attachments: Some(raw_cards),
                }),
                video_data: None,
                template_data: None,
            };
            form.push(("object_story_spec".to_owned(), encode_nested(&story)?));
        }
        AdCreativeContent::FlexibleImage {
            page_id,
            instagram_user_id,
            link_url,
            images,
            primary_texts,
            headlines,
            descriptions,
            call_to_action,
        } => {
            if images.is_empty() || images.len() > MAX_FLEX_IMAGES {
                return Err(PublicError::invalid_input(
                    "flexible images must contain between 1 and 10 items",
                    "Use bounded image variants",
                ));
            }
            let link = https_url(link_url, "link_url")?;
            let bodies = normalized_text_assets(
                primary_texts,
                1,
                MAX_TEXT_VARIANTS,
                MAX_PRIMARY_TEXT_CHARS,
                "primary_texts",
                true,
            )?;
            let titles = normalized_text_assets(
                headlines,
                0,
                MAX_TEXT_VARIANTS,
                MAX_HEADLINE_CHARS,
                "headlines",
                false,
            )?;
            let descriptions = normalized_text_assets(
                descriptions,
                0,
                MAX_TEXT_VARIANTS,
                MAX_DESCRIPTION_CHARS,
                "descriptions",
                false,
            )?;
            let raw_images = images
                .iter()
                .map(raw_asset_image)
                .collect::<Result<Vec<_>, _>>()?;
            let story = RawStorySpec {
                page_id: required_id(page_id, "page_id", "Use a numeric Facebook Page ID")?,
                instagram_user_id: optional_id(instagram_user_id.as_deref(), "instagram_user_id")?,
                link_data: None,
                video_data: None,
                template_data: None,
            };
            let feed = RawAssetFeedSpec {
                images: raw_images,
                bodies,
                titles,
                descriptions,
                link_urls: vec![RawLinkAsset { website_url: link }],
                call_to_action_types: call_to_action
                    .map(|cta| vec![cta.as_str()])
                    .unwrap_or_default(),
                ad_formats: ["SINGLE_IMAGE"],
            };
            form.push(("object_story_spec".to_owned(), encode_nested(&story)?));
            form.push(("asset_feed_spec".to_owned(), encode_nested(&feed)?));
        }
        AdCreativeContent::Catalog {
            page_id,
            instagram_user_id,
            product_set_id,
            link_url,
            primary_text,
            headline,
            description,
            call_to_action,
        } => {
            let link = https_url(link_url, "link_url")?;
            let story = RawStorySpec {
                page_id: required_id(page_id, "page_id", "Use a numeric Facebook Page ID")?,
                instagram_user_id: optional_id(instagram_user_id.as_deref(), "instagram_user_id")?,
                link_data: None,
                video_data: None,
                template_data: Some(RawTemplateData {
                    link,
                    message: optional_copy_text(
                        primary_text.as_deref(),
                        MAX_PRIMARY_TEXT_CHARS,
                        "primary_text",
                    )?,
                    name: optional_text(headline.as_deref(), MAX_HEADLINE_CHARS, "headline")?,
                    description: optional_text(
                        description.as_deref(),
                        MAX_DESCRIPTION_CHARS,
                        "description",
                    )?,
                    call_to_action: call_to_action
                        .map(|cta| RawCallToActionType { kind: cta.as_str() }),
                }),
            };
            form.push((
                "product_set_id".to_owned(),
                required_id(
                    product_set_id,
                    "product_set_id",
                    "Use an ID returned by list_product_sets",
                )?,
            ));
            form.push(("object_story_spec".to_owned(), encode_nested(&story)?));
        }
    }
    Ok(())
}

fn raw_call_to_action(
    input: Option<&CreativeCallToAction>,
    link: &str,
) -> Result<Option<RawCallToAction>, PublicError> {
    input
        .map(|input| {
            Ok(RawCallToAction {
                kind: input.button.as_str(),
                value: RawCallToActionValue {
                    link: link.to_owned(),
                    lead_gen_form_id: optional_id(
                        input.lead_form_id.as_deref(),
                        "call_to_action.lead_form_id",
                    )?,
                },
            })
        })
        .transpose()
}

fn image_link_fields(
    image: &CreativeImageSource,
) -> Result<(Option<String>, Option<String>), PublicError> {
    match image {
        CreativeImageSource::Hash { hash } => Ok((Some(normalized_hash(hash)?), None)),
        CreativeImageSource::Url { url } => Ok((None, Some(https_url(url, "image.url")?))),
    }
}

fn raw_asset_image(image: &CreativeImageSource) -> Result<RawAssetImage, PublicError> {
    let (hash, url) = image_link_fields(image)?;
    Ok(RawAssetImage { hash, url })
}

fn normalized_text_assets(
    values: &[String],
    min: usize,
    max: usize,
    max_chars: usize,
    field: &str,
    allow_newlines: bool,
) -> Result<Vec<RawTextAsset>, PublicError> {
    if values.len() < min || values.len() > max {
        return Err(PublicError::invalid_input(
            format!("{field} must contain between {min} and {max} items"),
            "Use a bounded set of distinct text variants",
        ));
    }
    let mut output = Vec::with_capacity(values.len());
    for value in values {
        let text = bounded_text(value, max_chars, field, allow_newlines)?;
        if output
            .iter()
            .any(|existing: &RawTextAsset| existing.text == text)
        {
            return Err(PublicError::invalid_input(
                format!("{field} contains duplicate text"),
                "Remove duplicate variants before retrying",
            ));
        }
        output.push(RawTextAsset { text });
    }
    Ok(output)
}

fn required_account_id(raw: &str) -> Result<String, PublicError> {
    ad_account(raw).ok_or_else(|| {
        PublicError::invalid_input(
            "ad_account_id must be a numeric Meta account ID",
            "Use an ID such as `act_123456789` or `123456789`",
        )
    })
}

fn required_id(raw: &str, field: &str, action: &str) -> Result<String, PublicError> {
    normalize_numeric_id(raw).ok_or_else(|| {
        PublicError::invalid_input(
            format!("{field} must be a numeric Meta ID"),
            action.to_owned(),
        )
    })
}

fn optional_id(raw: Option<&str>, field: &str) -> Result<Option<String>, PublicError> {
    raw.map(|value| required_id(value, field, "Use a numeric Meta ID"))
        .transpose()
}

fn normalized_story_id(raw: &str) -> Result<String, PublicError> {
    let value = raw.trim();
    let mut segments = value.split('_');
    let page = segments.next().and_then(normalize_numeric_id);
    let post = segments.next().and_then(normalize_numeric_id);
    match (page, post, segments.next()) {
        (Some(page), Some(post), None) => Ok(format!("{page}_{post}")),
        _ => Err(PublicError::invalid_input(
            "object_story_id must use numeric `page_id_post_id` form",
            "Use the existing Page post identifier returned by Meta",
        )),
    }
}

fn normalized_hash(raw: &str) -> Result<String, PublicError> {
    let value = raw.trim();
    if value.is_empty()
        || value.len() > MAX_HASH_CHARS
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(PublicError::invalid_input(
            "image hash is invalid or too long",
            "Use a hash returned by list_ad_images",
        ));
    }
    Ok(value.to_owned())
}

fn required_text(raw: &str, max_chars: usize, field: &str) -> Result<String, PublicError> {
    bounded_text(raw, max_chars, field, false)
}

fn bounded_text(
    raw: &str,
    max_chars: usize,
    field: &str,
    allow_newlines: bool,
) -> Result<String, PublicError> {
    let value = raw.trim();
    if value.is_empty()
        || value.chars().count() > max_chars
        || value
            .chars()
            .any(|character| character.is_control() && !(allow_newlines && character == '\n'))
    {
        return Err(PublicError::invalid_input(
            format!(
                "{field} must contain 1 to {max_chars} characters without unsupported controls"
            ),
            format!("Provide shorter plain-text {field}"),
        ));
    }
    Ok(value.to_owned())
}

fn optional_text(
    raw: Option<&str>,
    max_chars: usize,
    field: &str,
) -> Result<Option<String>, PublicError> {
    raw.map(|value| required_text(value, max_chars, field))
        .transpose()
}

fn optional_copy_text(
    raw: Option<&str>,
    max_chars: usize,
    field: &str,
) -> Result<Option<String>, PublicError> {
    raw.map(|value| bounded_text(value, max_chars, field, true))
        .transpose()
}

fn https_url(raw: &str, field: &str) -> Result<String, PublicError> {
    let value = raw.trim();
    if value.is_empty()
        || value.chars().count() > MAX_URL_CHARS
        || value.chars().any(char::is_control)
    {
        return Err(PublicError::invalid_input(
            format!("{field} is empty, invalid, or too long"),
            "Use a bounded public HTTPS URL",
        ));
    }
    let url = Url::parse(value).map_err(|_| {
        PublicError::invalid_input(
            format!("{field} must be an absolute HTTPS URL"),
            "Use a URL such as https://example.com/landing-page",
        )
    })?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || credential_value(value)
    {
        return Err(PublicError::invalid_input(
            format!("{field} must be a credential-free HTTPS URL"),
            "Remove embedded credentials and use a public HTTPS destination",
        ));
    }
    Ok(url.to_string())
}

fn normalized_domain(raw: &str) -> Result<String, PublicError> {
    let domain = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    if domain.is_empty()
        || domain.len() > 253
        || domain
            .bytes()
            .any(|byte| matches!(byte, b'/' | b':' | b'?' | b'#' | b'@'))
        || domain.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err(PublicError::invalid_input(
            "conversion_domain must be a hostname without scheme or path",
            "Use a domain such as www.example.com",
        ));
    }
    Ok(domain)
}

fn bounded_url_tags(raw: &str) -> Result<String, PublicError> {
    let value = raw.trim();
    if value.is_empty()
        || value.starts_with('?')
        || value.contains('#')
        || value.chars().count() > MAX_URL_TAGS_CHARS
        || value.chars().any(char::is_control)
        || credential_value(&format!("https://tags.invalid/?{value}"))
    {
        return Err(PublicError::invalid_input(
            "url_tags must be credential-free, non-empty, and at most 2048 characters",
            "Use a bounded query string such as utm_source=meta",
        ));
    }
    Ok(value.to_owned())
}

fn bounded_affix(raw: &str) -> Result<String, PublicError> {
    if raw.trim().is_empty()
        || raw.chars().count() > MAX_RENAME_CHARS
        || raw.chars().any(char::is_control)
    {
        return Err(PublicError::invalid_input(
            "name_suffix must contain 1 to 100 characters without controls",
            "Use a short plain-text suffix or omit it",
        ));
    }
    Ok(raw.to_owned())
}

fn encode_nested<T: Serialize>(value: &T) -> Result<String, PublicError> {
    let encoded = serde_json::to_string(value).map_err(|_| {
        PublicError::invalid_input(
            "the nested Meta specification could not be encoded",
            "Use the closed typed creative fields",
        )
    })?;
    if encoded.len() > MAX_NESTED_SPEC_BYTES {
        return Err(PublicError::invalid_input(
            "the nested Meta specification exceeds 64 KiB",
            "Use fewer or shorter creative assets",
        ));
    }
    Ok(encoded)
}

fn created_id(payload: &Value, resource: &str) -> Result<String, PublicError> {
    payload
        .get("id")
        .and_then(normalize_numeric_value)
        .ok_or_else(|| ambiguous_result(format!("Meta did not confirm the new {resource} ID")))
}

fn copied_ad_id(payload: &Value) -> Result<String, PublicError> {
    payload
        .get("copied_ad_id")
        .or_else(|| payload.get("id"))
        .and_then(normalize_numeric_value)
        .ok_or_else(|| ambiguous_result("Meta did not confirm the copied ad ID"))
}

fn confirmed_update(payload: &Value, expected_id: &str, resource: &str) -> Result<(), PublicError> {
    if payload.get("success").and_then(Value::as_bool) == Some(true)
        || payload
            .get("id")
            .and_then(normalize_numeric_value)
            .is_some_and(|id| id == expected_id)
    {
        return Ok(());
    }
    Err(ambiguous_result(format!(
        "Meta did not confirm the {resource} update"
    )))
}

fn ambiguous_result(message: impl Into<String>) -> PublicError {
    ambiguous_mutation_result(
        message,
        "Verify the result in Meta Ads Manager before retrying",
    )
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{
        AdCopyStatus, AdCreateStatus, AdCreativeContent, AdUpdateStatus, CarouselCard,
        CarouselMedia, CloneAdInput, CreateAdCreativeInput, CreateAdInput, CreativeCallToAction,
        CreativeCallToActionType, CreativeImageSource, PixelTracking, UpdateAdCreativeInput,
        UpdateAdInput, build_clone_ad_request, build_create_ad_request,
        build_create_creative_request, build_update_ad_request, build_update_creative_request,
    };

    fn form_value<'a>(form: &'a [(String, String)], key: &str) -> Option<&'a str> {
        form.iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn schemas_are_closed_and_publish_safe_defaults() {
        assert!(
            serde_json::from_value::<CreateAdInput>(json!({
                "ad_account_id": "1",
                "name": "Ad",
                "ad_set_id": "2",
                "ad_creative_id": "3",
                "unexpected": true
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<CreateAdCreativeInput>(json!({
                "ad_account_id": "1",
                "name": "Creative",
                "content": {
                    "kind": "existing_post",
                    "object_story_id": "2_3",
                    "unexpected": true
                }
            }))
            .is_err()
        );

        let create = serde_json::from_value::<CreateAdInput>(json!({
            "ad_account_id": "1",
            "name": "Ad",
            "ad_set_id": "2",
            "ad_creative_id": "3"
        }))
        .unwrap();
        assert!(matches!(create.status, AdCreateStatus::Paused));
        let clone = serde_json::from_value::<CloneAdInput>(json!({
            "ad_account_id": "1",
            "ad_id": "4"
        }))
        .unwrap();
        assert!(matches!(clone.status, AdCopyStatus::Paused));

        let schema = serde_json::to_value(rmcp::schemars::schema_for!(CreateAdInput)).unwrap();
        assert_eq!(
            schema.pointer("/properties/status/default"),
            Some(&json!("PAUSED"))
        );
    }

    #[test]
    fn builds_exact_paused_ad_create_form() {
        let request = build_create_ad_request(&CreateAdInput {
            ad_account_id: "123".to_owned(),
            name: " Launch ".to_owned(),
            ad_set_id: "456".to_owned(),
            ad_creative_id: "789".to_owned(),
            status: AdCreateStatus::default(),
            bid_amount: Some(250),
            conversion_domain: Some("WWW.Example.com.".to_owned()),
            pixel_tracking: Some(PixelTracking {
                pixel_id: "999".to_owned(),
            }),
        })
        .unwrap();
        assert_eq!(request.endpoint, "act_123/ads");
        assert_eq!(
            request.form,
            vec![
                ("name".to_owned(), "Launch".to_owned()),
                ("adset_id".to_owned(), "456".to_owned()),
                (
                    "creative".to_owned(),
                    "{\"creative_id\":\"789\"}".to_owned(),
                ),
                ("status".to_owned(), "PAUSED".to_owned()),
                ("bid_amount".to_owned(), "250".to_owned()),
                ("conversion_domain".to_owned(), "www.example.com".to_owned(),),
                (
                    "tracking_specs".to_owned(),
                    "[{\"action.type\":[\"offsite_conversion\"],\"fb_pixel\":[\"999\"]}]"
                        .to_owned(),
                ),
            ]
        );
    }

    #[test]
    fn builds_exact_ad_update_and_rejects_empty_updates() {
        let request = build_update_ad_request(&UpdateAdInput {
            ad_account_id: "act_123".to_owned(),
            ad_id: "100".to_owned(),
            name: Some("Revised".to_owned()),
            status: Some(AdUpdateStatus::Paused),
            bid_amount: None,
            conversion_domain: None,
            ad_creative_id: Some("200".to_owned()),
            pixel_tracking: None,
        })
        .unwrap();
        assert_eq!(request.endpoint, "100");
        assert_eq!(
            request.form,
            vec![
                ("name".to_owned(), "Revised".to_owned()),
                ("status".to_owned(), "PAUSED".to_owned()),
                (
                    "creative".to_owned(),
                    "{\"creative_id\":\"200\"}".to_owned(),
                ),
            ]
        );
        assert!(
            build_update_ad_request(&UpdateAdInput {
                ad_account_id: "act_123".to_owned(),
                ad_id: "100".to_owned(),
                name: None,
                status: None,
                bid_amount: None,
                conversion_domain: None,
                ad_creative_id: None,
                pixel_tracking: None,
            })
            .is_err()
        );
    }

    #[test]
    fn ad_copy_uses_only_documented_v26_parameters() {
        let request = build_clone_ad_request(&CloneAdInput {
            ad_account_id: "act_123".to_owned(),
            ad_id: "300".to_owned(),
            target_ad_set_id: Some("400".to_owned()),
            status: AdCopyStatus::default(),
            name_suffix: Some(" - Copy".to_owned()),
        })
        .unwrap();
        assert_eq!(request.endpoint, "300/copies");
        assert_eq!(
            request.form,
            vec![
                ("status_option".to_owned(), "PAUSED".to_owned()),
                ("adset_id".to_owned(), "400".to_owned()),
                (
                    "rename_options".to_owned(),
                    "{\"rename_strategy\":\"ONLY_TOP_LEVEL_RENAME\",\"rename_suffix\":\" - Copy\"}"
                        .to_owned(),
                ),
            ]
        );
        assert!(form_value(&request.form, "deep_copy").is_none());
        assert!(form_value(&request.form, "creative_parameters").is_none());
    }

    #[test]
    fn builds_exact_simple_image_creative_spec() {
        let request = build_create_creative_request(&CreateAdCreativeInput {
            ad_account_id: "act_1".to_owned(),
            name: "Image creative".to_owned(),
            content: AdCreativeContent::Image {
                page_id: "2".to_owned(),
                instagram_user_id: Some("3".to_owned()),
                image: CreativeImageSource::Hash {
                    hash: "abc_123".to_owned(),
                },
                link_url: "https://example.com/offer".to_owned(),
                primary_text: Some("Primary".to_owned()),
                headline: Some("Headline".to_owned()),
                description: Some("Description".to_owned()),
                call_to_action: Some(CreativeCallToAction {
                    button: CreativeCallToActionType::LearnMore,
                    lead_form_id: Some("4".to_owned()),
                }),
            },
            url_tags: Some("utm_source=meta".to_owned()),
        })
        .unwrap();
        assert_eq!(
            request,
            super::MutationRequest {
                endpoint: "act_1/adcreatives".to_owned(),
                form: vec![
                    ("name".to_owned(), "Image creative".to_owned()),
                    (
                        "object_story_spec".to_owned(),
                        "{\"page_id\":\"2\",\"instagram_user_id\":\"3\",\"link_data\":{\"link\":\"https://example.com/offer\",\"image_hash\":\"abc_123\",\"message\":\"Primary\",\"name\":\"Headline\",\"description\":\"Description\",\"call_to_action\":{\"type\":\"LEARN_MORE\",\"value\":{\"link\":\"https://example.com/offer\",\"lead_gen_form_id\":\"4\"}}}}"
                            .to_owned(),
                    ),
                    ("url_tags".to_owned(), "utm_source=meta".to_owned()),
                ],
            }
        );
    }

    #[test]
    fn builds_exact_video_creative_and_preserves_copy_line_breaks() {
        let request = build_create_creative_request(&CreateAdCreativeInput {
            ad_account_id: "1".to_owned(),
            name: "Video creative".to_owned(),
            content: AdCreativeContent::Video {
                page_id: "2".to_owned(),
                instagram_user_id: None,
                video_id: "3".to_owned(),
                thumbnail_url: Some("https://cdn.example/video.jpg".to_owned()),
                link_url: "https://example.com/watch".to_owned(),
                primary_text: Some("Watch\nnow".to_owned()),
                headline: Some("Watch".to_owned()),
                description: None,
                call_to_action: CreativeCallToAction {
                    button: CreativeCallToActionType::LearnMore,
                    lead_form_id: None,
                },
            },
            url_tags: None,
        })
        .unwrap();
        assert_eq!(
            request,
            super::MutationRequest {
                endpoint: "act_1/adcreatives".to_owned(),
                form: vec![
                    ("name".to_owned(), "Video creative".to_owned()),
                    (
                        "object_story_spec".to_owned(),
                        "{\"page_id\":\"2\",\"video_data\":{\"video_id\":\"3\",\"image_url\":\"https://cdn.example/video.jpg\",\"message\":\"Watch\\nnow\",\"title\":\"Watch\",\"call_to_action\":{\"type\":\"LEARN_MORE\",\"value\":{\"link\":\"https://example.com/watch\"}}}}"
                            .to_owned(),
                    ),
                ],
            }
        );
    }

    #[test]
    fn builds_exact_instagram_source_and_catalog_creatives() {
        let instagram = build_create_creative_request(&CreateAdCreativeInput {
            ad_account_id: "9".to_owned(),
            name: "Authorized partner media".to_owned(),
            content: AdCreativeContent::InstagramMedia {
                source_instagram_media_id: "42".to_owned(),
            },
            url_tags: None,
        })
        .unwrap();
        assert_eq!(
            instagram,
            super::MutationRequest {
                endpoint: "act_9/adcreatives".to_owned(),
                form: vec![
                    ("name".to_owned(), "Authorized partner media".to_owned()),
                    ("source_instagram_media_id".to_owned(), "42".to_owned()),
                ],
            }
        );

        let catalog = build_create_creative_request(&CreateAdCreativeInput {
            ad_account_id: "9".to_owned(),
            name: "Catalog".to_owned(),
            content: AdCreativeContent::Catalog {
                page_id: "10".to_owned(),
                instagram_user_id: Some("11".to_owned()),
                product_set_id: "12".to_owned(),
                link_url: "https://shop.example/products".to_owned(),
                primary_text: Some("Explore the collection".to_owned()),
                headline: Some("Shop now".to_owned()),
                description: Some("Available today".to_owned()),
                call_to_action: Some(CreativeCallToActionType::ShopNow),
            },
            url_tags: None,
        })
        .unwrap();
        assert_eq!(
            catalog,
            super::MutationRequest {
                endpoint: "act_9/adcreatives".to_owned(),
                form: vec![
                    ("name".to_owned(), "Catalog".to_owned()),
                    ("product_set_id".to_owned(), "12".to_owned()),
                    (
                        "object_story_spec".to_owned(),
                        "{\"page_id\":\"10\",\"instagram_user_id\":\"11\",\"template_data\":{\"link\":\"https://shop.example/products\",\"message\":\"Explore the collection\",\"name\":\"Shop now\",\"description\":\"Available today\",\"call_to_action\":{\"type\":\"SHOP_NOW\"}}}"
                            .to_owned(),
                    ),
                ],
            }
        );
    }

    #[test]
    fn rejects_credentials_in_public_urls_and_tracking_tags() {
        for query in [
            "access_token=synthetic-secret",
            "%61ccess_token=synthetic-secret",
            "provider_token=synthetic-secret",
            "utm_source=meta#%61ccess_token=synthetic-secret",
        ] {
            assert!(
                super::https_url(&format!("https://example.test/?{query}"), "link_url").is_err()
            );
            assert!(super::bounded_url_tags(query).is_err());
        }
        assert!(super::bounded_url_tags("utm_source=meta&campaign={{campaign.name}}").is_ok());
    }

    #[test]
    fn flexible_and_carousel_specs_are_bounded_and_typed() {
        let flexible = build_create_creative_request(&CreateAdCreativeInput {
            ad_account_id: "1".to_owned(),
            name: "Flexible".to_owned(),
            content: AdCreativeContent::FlexibleImage {
                page_id: "2".to_owned(),
                instagram_user_id: None,
                link_url: "https://example.com/".to_owned(),
                images: vec![CreativeImageSource::Url {
                    url: "https://cdn.example.com/image.jpg".to_owned(),
                }],
                primary_texts: vec!["One".to_owned(), "Two".to_owned()],
                headlines: vec!["Headline".to_owned()],
                descriptions: vec![],
                call_to_action: Some(CreativeCallToActionType::ShopNow),
            },
            url_tags: None,
        })
        .unwrap();
        let feed: Value =
            serde_json::from_str(form_value(&flexible.form, "asset_feed_spec").unwrap()).unwrap();
        assert_eq!(
            feed["images"],
            json!([{"url": "https://cdn.example.com/image.jpg"}])
        );
        assert_eq!(feed["bodies"], json!([{"text": "One"}, {"text": "Two"}]));
        assert_eq!(feed["ad_formats"], json!(["SINGLE_IMAGE"]));

        let invalid_carousel = CreateAdCreativeInput {
            ad_account_id: "1".to_owned(),
            name: "Carousel".to_owned(),
            content: AdCreativeContent::Carousel {
                page_id: "2".to_owned(),
                instagram_user_id: None,
                link_url: "https://example.com/".to_owned(),
                primary_text: None,
                call_to_action: None,
                cards: vec![CarouselCard {
                    link_url: "https://example.com/one".to_owned(),
                    headline: "One".to_owned(),
                    description: None,
                    media: CarouselMedia::Video {
                        video_id: "3".to_owned(),
                    },
                    call_to_action: None,
                }],
            },
            url_tags: None,
        };
        assert!(build_create_creative_request(&invalid_carousel).is_err());
    }

    #[test]
    fn creative_update_is_name_only() {
        let request = build_update_creative_request(&UpdateAdCreativeInput {
            ad_account_id: "act_123".to_owned(),
            ad_creative_id: "700".to_owned(),
            name: "Renamed".to_owned(),
        })
        .unwrap();
        assert_eq!(request.endpoint, "700");
        assert_eq!(
            request.form,
            vec![("name".to_owned(), "Renamed".to_owned())]
        );
    }

    #[test]
    fn status_values_match_meta_write_contracts() {
        assert_eq!(AdCreateStatus::Paused.as_str(), "PAUSED");
        assert_eq!(
            AdCopyStatus::InheritedFromSource.as_str(),
            "INHERITED_FROM_SOURCE"
        );
        assert_eq!(AdUpdateStatus::Archived.as_str(), "ARCHIVED");
    }
}
