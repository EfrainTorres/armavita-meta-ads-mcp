use rmcp::{
    Json,
    handler::server::wrapper::Parameters,
    schemars::{self, JsonSchema},
    tool, tool_router,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    graph_tools,
    meta_ids::numeric_owned as normalize_numeric_id,
    server::MetaAdsServer,
};

const MAX_LOCALE_CHARS: usize = 32;
const MAX_DIMENSION: u16 = 4_096;
const MAX_PREVIEWS: usize = 8;
const MAX_BODY_CHARS: usize = 20_000;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListAdPreviewsInput {
    /// Numeric Meta ad ID.
    pub ad_id: String,
    /// Common current placement format. Defaults to `desktop_feed_standard`.
    pub ad_format: Option<AdPreviewFormat>,
    /// Optional Meta locale such as `en_US`.
    pub locale: Option<String>,
    /// Optional preview width in pixels, from 1 through 4096.
    pub width: Option<u16>,
    /// Optional preview height in pixels, from 1 through 4096.
    pub height: Option<u16>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GenerateCreativePreviewsInput {
    pub ad_account_id: String,
    /// Existing creative as {"id":"..."}, or the bounded v26 creative specification to preview.
    pub creative: Map<String, Value>,
    pub ad_format: AdPreviewFormat,
    /// Optional v26 preview parameters: locale, dimensions, product IDs, dynamic specs and render options.
    pub options: Option<Map<String, Value>>,
}

#[tool_router(router = creative_previews_router, vis = "pub(crate)")]
impl MetaAdsServer {
    #[tool(
        name = "generate_creative_previews",
        description = "Preview an existing creative or a draft specification before creating an ad. Supports Facebook, Instagram and Threads formats; creates no ad.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn generate_creative_previews(
        &self,
        Parameters(input): Parameters<GenerateCreativePreviewsInput>,
    ) -> Result<Json<ToolResponse<AdPreviewList>>, Json<ToolResponse<AdPreviewList>>> {
        let prepared = build_creative_preview_request(&input);
        let response = match prepared {
            Ok((endpoint, params)) => match self.graph.get_json(&endpoint, &params).await {
                Ok(payload) => {
                    graph_tools::response(normalize_page(payload, input.ad_format, false))
                }
                Err(error) => ToolResponse::error(error),
            },
            Err(error) => ToolResponse::error(error),
        };
        response.into_mcp_result()
    }
}

fn build_creative_preview_request(
    input: &GenerateCreativePreviewsInput,
) -> Result<(String, graph_tools::Params), PublicError> {
    let account = graph_tools::account(&input.ad_account_id)?;
    if input.creative.is_empty() {
        return Err(PublicError::invalid_input(
            "creative cannot be empty",
            "Provide an existing creative ID or a draft specification",
        ));
    }
    let mut params = vec![("ad_format".into(), input.ad_format.as_graph_value().into())];
    let endpoint = if let Some(id) = input.creative.get("id") {
        if input.creative.len() != 1
            || !id.is_string()
            || input
                .options
                .as_ref()
                .is_some_and(|options| options.contains_key("message"))
        {
            return Err(PublicError::invalid_input(
                "Existing creative previews accept only its ID and preview options",
                "Use {\"id\":\"CREATIVE_ID\"} without draft fields or message",
            ));
        }
        format!(
            "{}/previews",
            graph_tools::id(id.as_str().unwrap_or_default(), "creative.id")?
        )
    } else {
        params.push((
            "creative".into(),
            graph_tools::json(&Value::Object(input.creative.clone()), "creative")?,
        ));
        format!("{account}/generatepreviews")
    };
    if let Some(options) = &input.options {
        for field in ["width", "height"] {
            if let Some(value) = options.get(field)
                && value.as_u64().is_none_or(|v| !(1..=4096).contains(&v))
            {
                return Err(PublicError::invalid_input(
                    "Preview dimensions must be 1–4096 pixels",
                    "Use numeric width and height",
                ));
            }
        }
        params.extend(graph_tools::form_fields(
            options,
            &[
                "creative_feature",
                "dynamic_asset_label",
                "dynamic_creative_spec",
                "dynamic_customization",
                "end_date",
                "height",
                "locale",
                "message",
                "place_page_id",
                "post",
                "product_item_ids",
                "render_type",
                "start_date",
                "width",
            ],
        )?);
    }
    Ok((endpoint, params))
}

/// Meta v26 preview formats. Availability depends on the creative and placement.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AdPreviewFormat {
    AudienceNetworkInstreamVideo,
    AudienceNetworkInstreamVideoMobile,
    AudienceNetworkOutstreamVideo,
    AudienceNetworkRewardedVideo,
    BizDiscoFeedMobile,
    DesktopFeedStandard,
    FacebookIfuReelsMobile,
    FacebookProfileFeedDesktop,
    FacebookProfileFeedMobile,
    FacebookProfileReelsMobile,
    FacebookReelsBanner,
    FacebookReelsBannerDesktop,
    FacebookReelsBannerFeedAndroid,
    FacebookReelsBannerFeedAndroidLarge,
    FacebookReelsBannerFullscreenIos,
    FacebookReelsBannerFullscreenMobile,
    FacebookReelsMobile,
    FacebookReelsPostloop,
    FacebookReelsPostloopFeed,
    FacebookReelsSimilarProductsMobile,
    FacebookReelsSticker,
    FacebookStoryMobile,
    FacebookStoryStickerMobile,
    InstagramExploreContextual,
    InstagramExploreGridHome,
    InstagramExploreImmersive,
    InstagramFeedWeb,
    InstagramFeedWebMSite,
    InstagramLeadGenMultiSubmitAds,
    InstagramProfileFeed,
    InstagramProfileReels,
    InstagramReels,
    InstagramReelsOverlay,
    InstagramReelsWeb,
    InstagramReelsWebMSite,
    InstagramSearchChain,
    InstagramSearchGrid,
    InstagramStandard,
    InstagramStory,
    InstagramStoryEffectTray,
    InstagramStoryWeb,
    InstagramStoryWebMSite,
    InstantArticleRecirculationAd,
    InstantArticleStandard,
    InstreamBannerDesktop,
    InstreamBannerFeedIos,
    InstreamBannerFullscreenIos,
    InstreamBannerFullscreenMobile,
    InstreamBannerImmersiveMobile,
    InstreamBannerMobile,
    InstreamVideoDesktop,
    InstreamVideoFullscreenIos,
    InstreamVideoFullscreenMobile,
    InstreamVideoImage,
    InstreamVideoImmersiveMobile,
    InstreamVideoMobile,
    JobBrowserDesktop,
    JobBrowserMobile,
    MarketplaceMobile,
    MessengerMobileInboxMedia,
    MessengerMobileStoryMedia,
    MobileBanner,
    MobileFeedBasic,
    MobileFeedStandard,
    MobileFullwidth,
    MobileInterstitial,
    MobileMediumRectangle,
    MobileNative,
    RightColumnStandard,
    SuggestedVideoDesktop,
    SuggestedVideoFullscreenMobile,
    SuggestedVideoImmersiveMobile,
    SuggestedVideoMobile,
    WatchFeedHome,
    WatchFeedMobile,
    ThreadsStream,
}

impl AdPreviewFormat {
    const fn as_graph_value(self) -> &'static str {
        match self {
            Self::AudienceNetworkInstreamVideo => "AUDIENCE_NETWORK_INSTREAM_VIDEO",
            Self::AudienceNetworkInstreamVideoMobile => "AUDIENCE_NETWORK_INSTREAM_VIDEO_MOBILE",
            Self::AudienceNetworkOutstreamVideo => "AUDIENCE_NETWORK_OUTSTREAM_VIDEO",
            Self::AudienceNetworkRewardedVideo => "AUDIENCE_NETWORK_REWARDED_VIDEO",
            Self::BizDiscoFeedMobile => "BIZ_DISCO_FEED_MOBILE",
            Self::DesktopFeedStandard => "DESKTOP_FEED_STANDARD",
            Self::FacebookIfuReelsMobile => "FACEBOOK_IFU_REELS_MOBILE",
            Self::FacebookProfileFeedDesktop => "FACEBOOK_PROFILE_FEED_DESKTOP",
            Self::FacebookProfileFeedMobile => "FACEBOOK_PROFILE_FEED_MOBILE",
            Self::FacebookProfileReelsMobile => "FACEBOOK_PROFILE_REELS_MOBILE",
            Self::FacebookReelsBanner => "FACEBOOK_REELS_BANNER",
            Self::FacebookReelsBannerDesktop => "FACEBOOK_REELS_BANNER_DESKTOP",
            Self::FacebookReelsBannerFeedAndroid => "FACEBOOK_REELS_BANNER_FEED_ANDROID",
            Self::FacebookReelsBannerFeedAndroidLarge => "FACEBOOK_REELS_BANNER_FEED_ANDROID_LARGE",
            Self::FacebookReelsBannerFullscreenIos => "FACEBOOK_REELS_BANNER_FULLSCREEN_IOS",
            Self::FacebookReelsBannerFullscreenMobile => "FACEBOOK_REELS_BANNER_FULLSCREEN_MOBILE",
            Self::FacebookReelsMobile => "FACEBOOK_REELS_MOBILE",
            Self::FacebookReelsPostloop => "FACEBOOK_REELS_POSTLOOP",
            Self::FacebookReelsPostloopFeed => "FACEBOOK_REELS_POSTLOOP_FEED",
            Self::FacebookReelsSimilarProductsMobile => "FACEBOOK_REELS_SIMILAR_PRODUCTS_MOBILE",
            Self::FacebookReelsSticker => "FACEBOOK_REELS_STICKER",
            Self::FacebookStoryMobile => "FACEBOOK_STORY_MOBILE",
            Self::FacebookStoryStickerMobile => "FACEBOOK_STORY_STICKER_MOBILE",
            Self::InstagramExploreContextual => "INSTAGRAM_EXPLORE_CONTEXTUAL",
            Self::InstagramExploreGridHome => "INSTAGRAM_EXPLORE_GRID_HOME",
            Self::InstagramExploreImmersive => "INSTAGRAM_EXPLORE_IMMERSIVE",
            Self::InstagramFeedWeb => "INSTAGRAM_FEED_WEB",
            Self::InstagramFeedWebMSite => "INSTAGRAM_FEED_WEB_M_SITE",
            Self::InstagramLeadGenMultiSubmitAds => "INSTAGRAM_LEAD_GEN_MULTI_SUBMIT_ADS",
            Self::InstagramProfileFeed => "INSTAGRAM_PROFILE_FEED",
            Self::InstagramProfileReels => "INSTAGRAM_PROFILE_REELS",
            Self::InstagramReels => "INSTAGRAM_REELS",
            Self::InstagramReelsOverlay => "INSTAGRAM_REELS_OVERLAY",
            Self::InstagramReelsWeb => "INSTAGRAM_REELS_WEB",
            Self::InstagramReelsWebMSite => "INSTAGRAM_REELS_WEB_M_SITE",
            Self::InstagramSearchChain => "INSTAGRAM_SEARCH_CHAIN",
            Self::InstagramSearchGrid => "INSTAGRAM_SEARCH_GRID",
            Self::InstagramStandard => "INSTAGRAM_STANDARD",
            Self::InstagramStory => "INSTAGRAM_STORY",
            Self::InstagramStoryEffectTray => "INSTAGRAM_STORY_EFFECT_TRAY",
            Self::InstagramStoryWeb => "INSTAGRAM_STORY_WEB",
            Self::InstagramStoryWebMSite => "INSTAGRAM_STORY_WEB_M_SITE",
            Self::InstantArticleRecirculationAd => "INSTANT_ARTICLE_RECIRCULATION_AD",
            Self::InstantArticleStandard => "INSTANT_ARTICLE_STANDARD",
            Self::InstreamBannerDesktop => "INSTREAM_BANNER_DESKTOP",
            Self::InstreamBannerFeedIos => "INSTREAM_BANNER_FEED_IOS",
            Self::InstreamBannerFullscreenIos => "INSTREAM_BANNER_FULLSCREEN_IOS",
            Self::InstreamBannerFullscreenMobile => "INSTREAM_BANNER_FULLSCREEN_MOBILE",
            Self::InstreamBannerImmersiveMobile => "INSTREAM_BANNER_IMMERSIVE_MOBILE",
            Self::InstreamBannerMobile => "INSTREAM_BANNER_MOBILE",
            Self::InstreamVideoDesktop => "INSTREAM_VIDEO_DESKTOP",
            Self::InstreamVideoFullscreenIos => "INSTREAM_VIDEO_FULLSCREEN_IOS",
            Self::InstreamVideoFullscreenMobile => "INSTREAM_VIDEO_FULLSCREEN_MOBILE",
            Self::InstreamVideoImage => "INSTREAM_VIDEO_IMAGE",
            Self::InstreamVideoImmersiveMobile => "INSTREAM_VIDEO_IMMERSIVE_MOBILE",
            Self::InstreamVideoMobile => "INSTREAM_VIDEO_MOBILE",
            Self::JobBrowserDesktop => "JOB_BROWSER_DESKTOP",
            Self::JobBrowserMobile => "JOB_BROWSER_MOBILE",
            Self::MarketplaceMobile => "MARKETPLACE_MOBILE",
            Self::MessengerMobileInboxMedia => "MESSENGER_MOBILE_INBOX_MEDIA",
            Self::MessengerMobileStoryMedia => "MESSENGER_MOBILE_STORY_MEDIA",
            Self::MobileBanner => "MOBILE_BANNER",
            Self::MobileFeedBasic => "MOBILE_FEED_BASIC",
            Self::MobileFeedStandard => "MOBILE_FEED_STANDARD",
            Self::MobileFullwidth => "MOBILE_FULLWIDTH",
            Self::MobileInterstitial => "MOBILE_INTERSTITIAL",
            Self::MobileMediumRectangle => "MOBILE_MEDIUM_RECTANGLE",
            Self::MobileNative => "MOBILE_NATIVE",
            Self::RightColumnStandard => "RIGHT_COLUMN_STANDARD",
            Self::SuggestedVideoDesktop => "SUGGESTED_VIDEO_DESKTOP",
            Self::SuggestedVideoFullscreenMobile => "SUGGESTED_VIDEO_FULLSCREEN_MOBILE",
            Self::SuggestedVideoImmersiveMobile => "SUGGESTED_VIDEO_IMMERSIVE_MOBILE",
            Self::SuggestedVideoMobile => "SUGGESTED_VIDEO_MOBILE",
            Self::WatchFeedHome => "WATCH_FEED_HOME",
            Self::WatchFeedMobile => "WATCH_FEED_MOBILE",
            Self::ThreadsStream => "THREADS_STREAM",
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AdPreviewList {
    pub ad_format: AdPreviewFormat,
    pub auto_selected: bool,
    pub previews: Vec<AdPreview>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AdPreview {
    /// Meta-rendered preview HTML. It is bounded and may be truncated.
    pub body: String,
    #[serde(skip_serializing_if = "is_false")]
    pub body_truncated: bool,
}

#[derive(Debug)]
struct PreviewRequest {
    endpoint: String,
    query: Vec<(String, String)>,
    ad_format: AdPreviewFormat,
    auto_selected: bool,
}

#[derive(Debug, Deserialize)]
struct RawPreviewPage {
    #[serde(default)]
    data: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct RawPreview {
    body: Option<String>,
}

pub(crate) async fn list_ad_previews(
    graph: &GraphClient,
    input: ListAdPreviewsInput,
) -> ToolResponse<AdPreviewList> {
    let request = match build_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.get_json(&request.endpoint, &request.query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    match normalize_page(payload, request.ad_format, request.auto_selected) {
        Ok(previews) => ToolResponse::success(previews),
        Err(error) => ToolResponse::error(error),
    }
}

fn build_request(input: &ListAdPreviewsInput) -> Result<PreviewRequest, PublicError> {
    let ad_id = normalize_numeric_id(&input.ad_id).ok_or_else(|| {
        PublicError::invalid_input(
            "ad_id must be a numeric Meta ad ID",
            "Use an ID returned by list_ads",
        )
    })?;
    let locale = input.locale.as_deref().map(validate_locale).transpose()?;
    validate_dimension(input.width, "width")?;
    validate_dimension(input.height, "height")?;

    let auto_selected = input.ad_format.is_none();
    let ad_format = input
        .ad_format
        .unwrap_or(AdPreviewFormat::DesktopFeedStandard);
    let mut query = vec![
        ("fields".to_owned(), "body".to_owned()),
        (
            "ad_format".to_owned(),
            ad_format.as_graph_value().to_owned(),
        ),
    ];
    if let Some(locale) = locale {
        query.push(("locale".to_owned(), locale.to_owned()));
    }
    if let Some(width) = input.width {
        query.push(("width".to_owned(), width.to_string()));
    }
    if let Some(height) = input.height {
        query.push(("height".to_owned(), height.to_string()));
    }
    Ok(PreviewRequest {
        endpoint: format!("{ad_id}/previews"),
        query,
        ad_format,
        auto_selected,
    })
}

fn validate_locale(raw: &str) -> Result<&str, PublicError> {
    let value = raw.trim();
    if value.is_empty()
        || value.chars().count() > MAX_LOCALE_CHARS
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(PublicError::invalid_input(
            "locale must be a short Meta locale code",
            "Use a value such as `en_US`",
        ));
    }
    Ok(value)
}

fn validate_dimension(value: Option<u16>, field: &str) -> Result<(), PublicError> {
    if value.is_some_and(|value| !(1..=MAX_DIMENSION).contains(&value)) {
        return Err(PublicError::invalid_input(
            format!("{field} must be between 1 and 4096"),
            "Use a bounded preview dimension or omit it",
        ));
    }
    Ok(())
}

fn normalize_page(
    payload: Value,
    ad_format: AdPreviewFormat,
    auto_selected: bool,
) -> Result<AdPreviewList, PublicError> {
    let raw = serde_json::from_value::<RawPreviewPage>(payload)
        .map_err(|_| PublicError::invalid_upstream("Meta returned an unexpected preview list"))?;
    if raw.data.len() > MAX_PREVIEWS {
        return Err(PublicError::invalid_upstream(
            "Meta returned too many ad previews",
        ));
    }
    let mut previews = Vec::with_capacity(raw.data.len());
    for value in raw.data {
        let raw = serde_json::from_value::<RawPreview>(value).map_err(|_| {
            PublicError::invalid_upstream("Meta returned malformed ad-preview metadata")
        })?;
        let body = raw.body.ok_or_else(|| {
            PublicError::invalid_upstream("Meta omitted the rendered ad-preview body")
        })?;
        if contains_sensitive_parameter(&body) {
            return Err(PublicError::invalid_upstream(
                "Meta returned a preview containing credential-like parameters",
            ));
        }
        let (body, body_truncated) = bounded_body(body);
        previews.push(AdPreview {
            body,
            body_truncated,
        });
    }
    Ok(AdPreviewList {
        ad_format,
        auto_selected,
        previews,
    })
}

fn bounded_body(value: String) -> (String, bool) {
    if value.chars().count() <= MAX_BODY_CHARS {
        return (value, false);
    }
    let mut body = value
        .chars()
        .take(MAX_BODY_CHARS.saturating_sub(1))
        .collect::<String>();
    body.push('…');
    (body, true)
}

fn contains_sensitive_parameter(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.contains("access_token") || value.contains("appsecret_proof")
}

const fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        AdPreviewFormat, ListAdPreviewsInput, MAX_BODY_CHARS, build_request, normalize_page,
    };

    fn input() -> ListAdPreviewsInput {
        ListAdPreviewsInput {
            ad_id: "123".to_owned(),
            ad_format: None,
            locale: Some("en_US".to_owned()),
            width: Some(1080),
            height: Some(1080),
        }
    }

    #[test]
    fn drafts_and_existing_creatives_use_exact_threads_preview_contracts() {
        let mut input: super::GenerateCreativePreviewsInput = serde_json::from_value(json!({
            "ad_account_id":"123", "creative":{"object_story_spec":{"page_id":"42","link_data":{"message":"A new ad"}}}, "ad_format":"threads_stream"
        })).unwrap();
        let (path, params) = super::build_creative_preview_request(&input).unwrap();
        assert_eq!(path, "act_123/generatepreviews");
        assert!(params.contains(&("ad_format".into(), "THREADS_STREAM".into())));
        input.creative = serde_json::from_value(json!({"id":"555"})).unwrap();
        assert_eq!(
            super::build_creative_preview_request(&input).unwrap().0,
            "555/previews"
        );
        input.creative =
            serde_json::from_value(json!({"object_story_spec":{"access_token":"private"}}))
                .unwrap();
        assert!(super::build_creative_preview_request(&input).is_err());
    }

    #[test]
    fn builds_one_deterministic_current_preview_request() {
        let request = build_request(&input()).unwrap();
        assert_eq!(request.endpoint, "123/previews");
        assert!(request.auto_selected);
        assert!(
            request
                .query
                .contains(&("ad_format".to_owned(), "DESKTOP_FEED_STANDARD".to_owned()))
        );
        assert!(
            request
                .query
                .contains(&("locale".to_owned(), "en_US".to_owned()))
        );
        assert!(!request.query.iter().any(|(key, _)| key == "access_token"));
    }

    #[test]
    fn rejects_invalid_ids_locales_and_dimensions() {
        let mut invalid = input();
        invalid.ad_id = "123/previews".to_owned();
        assert!(build_request(&invalid).is_err());

        let mut invalid = input();
        invalid.locale = Some("en/US".to_owned());
        assert!(build_request(&invalid).is_err());

        let mut invalid = input();
        invalid.width = Some(0);
        assert!(build_request(&invalid).is_err());
    }

    #[test]
    fn bounds_preview_html_and_fails_closed_on_credential_markers() {
        let page = normalize_page(
            json!({"data": [{"body": "x".repeat(MAX_BODY_CHARS + 1)}]}),
            AdPreviewFormat::MobileFeedStandard,
            false,
        )
        .unwrap();
        assert!(page.previews[0].body_truncated);
        assert_eq!(page.previews[0].body.chars().count(), MAX_BODY_CHARS);

        assert!(
            normalize_page(
                json!({"data": [{"body": "<iframe src='?access_token=secret'>"}]}),
                AdPreviewFormat::DesktopFeedStandard,
                true,
            )
            .is_err()
        );
    }
}
