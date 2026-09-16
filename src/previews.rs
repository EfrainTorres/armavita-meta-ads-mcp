use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::numeric_owned as normalize_numeric_id,
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

/// A compact, intentionally curated subset of Meta v26's large preview-format enum.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AdPreviewFormat {
    DesktopFeedStandard,
    MobileFeedStandard,
    InstagramStandard,
    FacebookStoryMobile,
    InstagramStory,
    FacebookReelsMobile,
    InstagramReels,
    RightColumnStandard,
    MarketplaceMobile,
    AudienceNetworkRewardedVideo,
}

impl AdPreviewFormat {
    const fn as_graph_value(self) -> &'static str {
        match self {
            Self::DesktopFeedStandard => "DESKTOP_FEED_STANDARD",
            Self::MobileFeedStandard => "MOBILE_FEED_STANDARD",
            Self::InstagramStandard => "INSTAGRAM_STANDARD",
            Self::FacebookStoryMobile => "FACEBOOK_STORY_MOBILE",
            Self::InstagramStory => "INSTAGRAM_STORY",
            Self::FacebookReelsMobile => "FACEBOOK_REELS_MOBILE",
            Self::InstagramReels => "INSTAGRAM_REELS",
            Self::RightColumnStandard => "RIGHT_COLUMN_STANDARD",
            Self::MarketplaceMobile => "MARKETPLACE_MOBILE",
            Self::AudienceNetworkRewardedVideo => "AUDIENCE_NETWORK_REWARDED_VIDEO",
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
