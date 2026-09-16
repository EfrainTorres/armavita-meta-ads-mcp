use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{
        ad_account as normalize_account_id, numeric_owned as normalize_numeric_id,
        numeric_value as meta_numeric_value,
    },
};

// Deliberately omit object_story_spec, asset_feed_spec, and video source bytes/URLs.
// Dedicated write or inspection tools can expose narrowly selected spec fields later.
const CREATIVE_LIST_FIELDS: &str = "id,name,status,object_type,object_id,object_story_id,effective_object_story_id,image_hash,video_id,thumbnail_url";
const CREATIVE_DETAIL_FIELDS: &str = "id,account_id,actor_id,instagram_user_id,name,status,object_type,object_id,object_story_id,effective_object_story_id,image_hash,video_id,thumbnail_url,image_url,link_url,instagram_permalink_url,title,body,call_to_action_type";
const IMAGE_FIELDS: &str = "id,hash,name,status,width,height,created_time,updated_time,url";
const VIDEO_FIELDS: &str = "id,title,description,picture,length,status,created_time,updated_time";
const DEFAULT_PAGE_SIZE: u16 = 25;
const MAX_PAGE_SIZE: u16 = 100;
const MAX_CURSOR_CHARS: usize = 2_048;
const MAX_HASH_CHARS: usize = 128;
const MAX_HASH_FILTERS: usize = 50;
const MAX_NAME_CHARS: usize = 256;
const MAX_BODY_CHARS: usize = 512;
const MAX_URL_CHARS: usize = 2_048;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListAdCreativesInput {
    /// Numeric Meta ad ID.
    pub ad_id: String,
    /// Number of creatives to return, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadAdCreativeInput {
    /// Numeric Meta ad-creative ID.
    pub ad_creative_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListAdImagesInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    pub ad_account_id: String,
    /// Number of images to return, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
    /// Optional image hashes to select. At most 50 bounded hash identifiers.
    pub hashes: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListAdVideosInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    pub ad_account_id: String,
    /// Number of videos to return, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AdCreativeList {
    pub creatives: Vec<AdCreative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Compact creative metadata. Large story and asset-feed specs are intentionally absent.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AdCreative {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instagram_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_story_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_object_story_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_to_action_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instagram_permalink_url: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AdImageList {
    pub images: Vec<AdImage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Ad-library image metadata. The `hash` is the stable reuse identifier.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AdImage {
    pub hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AdVideoList {
    pub videos: Vec<AdVideo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Ad-library video metadata. Source media is intentionally absent.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AdVideo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
}

#[derive(Debug)]
struct ListRequest {
    endpoint: String,
    query: Vec<(String, String)>,
    page_size: u16,
}

#[derive(Debug)]
struct Page {
    size: u16,
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawList<T> {
    data: Vec<T>,
    paging: Option<RawPaging>,
}

#[derive(Debug, Deserialize)]
struct RawPaging {
    cursors: Option<RawCursors>,
}

#[derive(Debug, Deserialize)]
struct RawCursors {
    after: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawCreative {
    id: Option<Value>,
    account_id: Option<Value>,
    actor_id: Option<Value>,
    instagram_user_id: Option<Value>,
    name: Option<String>,
    status: Option<Value>,
    object_type: Option<String>,
    object_id: Option<Value>,
    object_story_id: Option<String>,
    effective_object_story_id: Option<String>,
    image_hash: Option<String>,
    video_id: Option<Value>,
    title: Option<String>,
    body: Option<String>,
    call_to_action_type: Option<String>,
    thumbnail_url: Option<String>,
    image_url: Option<String>,
    link_url: Option<String>,
    instagram_permalink_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawImage {
    id: Option<String>,
    hash: Option<String>,
    name: Option<String>,
    status: Option<Value>,
    width: Option<Value>,
    height: Option<Value>,
    created_time: Option<String>,
    updated_time: Option<String>,
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawVideo {
    id: Option<Value>,
    title: Option<String>,
    description: Option<String>,
    picture: Option<String>,
    length: Option<Value>,
    status: Option<Value>,
    created_time: Option<String>,
    updated_time: Option<String>,
}

pub(crate) async fn list_ad_creatives(
    graph: &GraphClient,
    input: ListAdCreativesInput,
) -> ToolResponse<AdCreativeList> {
    let request = match build_creative_list_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    match fetch_list(graph, request, "creative", normalize_creative).await {
        Ok((creatives, next_cursor)) => ToolResponse::success(AdCreativeList {
            creatives,
            next_cursor,
        }),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn read_ad_creative(
    graph: &GraphClient,
    input: ReadAdCreativeInput,
) -> ToolResponse<AdCreative> {
    let Some(creative_id) = normalize_numeric_id(&input.ad_creative_id) else {
        return ToolResponse::error(PublicError::invalid_input(
            "ad_creative_id must be a numeric Meta creative ID",
            "Use the ID returned by list_ad_creatives",
        ));
    };
    let query = vec![("fields".to_owned(), CREATIVE_DETAIL_FIELDS.to_owned())];
    let payload = match graph.get_json(&creative_id, &query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    let raw = match serde_json::from_value::<RawCreative>(payload) {
        Ok(raw) => raw,
        Err(_) => {
            return ToolResponse::error(PublicError::invalid_upstream(
                "Meta returned unexpected creative metadata",
            ));
        }
    };

    match normalize_creative(raw) {
        Some(creative) => ToolResponse::success(creative),
        None => ToolResponse::error(PublicError::invalid_upstream(
            "Meta omitted a valid creative ID",
        )),
    }
}

pub(crate) async fn list_ad_images(
    graph: &GraphClient,
    input: ListAdImagesInput,
) -> ToolResponse<AdImageList> {
    let request = match build_image_list_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    match fetch_list(graph, request, "image", normalize_image).await {
        Ok((images, next_cursor)) => ToolResponse::success(AdImageList {
            images,
            next_cursor,
        }),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn list_ad_videos(
    graph: &GraphClient,
    input: ListAdVideosInput,
) -> ToolResponse<AdVideoList> {
    let request = match build_video_list_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    match fetch_list(graph, request, "video", normalize_video).await {
        Ok((videos, next_cursor)) => ToolResponse::success(AdVideoList {
            videos,
            next_cursor,
        }),
        Err(error) => ToolResponse::error(error),
    }
}

fn build_creative_list_request(input: &ListAdCreativesInput) -> Result<ListRequest, PublicError> {
    let ad_id = normalize_numeric_id(&input.ad_id).ok_or_else(|| {
        PublicError::invalid_input(
            "ad_id must be a numeric Meta ad ID",
            "Use an ID returned by list_ads",
        )
    })?;
    let page = validate_page(input.page_size, input.page_cursor.as_deref())?;
    Ok(edge_request(
        format!("{ad_id}/adcreatives"),
        CREATIVE_LIST_FIELDS,
        page,
    ))
}

fn build_image_list_request(input: &ListAdImagesInput) -> Result<ListRequest, PublicError> {
    let account_id = normalize_account_id(&input.ad_account_id).ok_or_else(|| {
        PublicError::invalid_input(
            "ad_account_id must be a numeric Meta account ID",
            "Use an ID such as `act_123456789` or `123456789`",
        )
    })?;
    let page = validate_page(input.page_size, input.page_cursor.as_deref())?;
    let mut request = edge_request(format!("{account_id}/adimages"), IMAGE_FIELDS, page);
    if let Some(hashes) = normalize_hash_filters(input.hashes.as_deref())? {
        let encoded = serde_json::to_string(&hashes).map_err(|_| {
            PublicError::invalid_input(
                "hashes could not be encoded",
                "Use image hashes returned by list_ad_images",
            )
        })?;
        request.query.push(("hashes".to_owned(), encoded));
    }
    Ok(request)
}

fn build_video_list_request(input: &ListAdVideosInput) -> Result<ListRequest, PublicError> {
    let account_id = normalize_account_id(&input.ad_account_id).ok_or_else(|| {
        PublicError::invalid_input(
            "ad_account_id must be a numeric Meta account ID",
            "Use an ID such as `act_123456789` or `123456789`",
        )
    })?;
    let page = validate_page(input.page_size, input.page_cursor.as_deref())?;
    Ok(edge_request(
        format!("{account_id}/advideos"),
        VIDEO_FIELDS,
        page,
    ))
}

fn edge_request(endpoint: String, fields: &str, page: Page) -> ListRequest {
    let mut query = vec![
        ("fields".to_owned(), fields.to_owned()),
        ("limit".to_owned(), page.size.to_string()),
    ];
    if let Some(cursor) = page.cursor {
        query.push(("after".to_owned(), cursor));
    }
    ListRequest {
        endpoint,
        query,
        page_size: page.size,
    }
}

fn validate_page(page_size: Option<u16>, page_cursor: Option<&str>) -> Result<Page, PublicError> {
    let size = page_size.unwrap_or(DEFAULT_PAGE_SIZE);
    if !(1..=MAX_PAGE_SIZE).contains(&size) {
        return Err(PublicError::invalid_input(
            "page_size must be between 1 and 100",
            "Choose a bounded page_size and paginate with next_cursor",
        ));
    }
    let cursor = page_cursor
        .filter(|cursor| !cursor.is_empty())
        .map(str::to_owned);
    if cursor
        .as_ref()
        .is_some_and(|cursor| cursor.chars().count() > MAX_CURSOR_CHARS)
    {
        return Err(PublicError::invalid_input(
            "page_cursor is too long",
            "Use next_cursor exactly as returned by the previous response",
        ));
    }
    Ok(Page { size, cursor })
}

fn normalize_hash_filters(raw: Option<&[String]>) -> Result<Option<Vec<String>>, PublicError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.len() > MAX_HASH_FILTERS {
        return Err(PublicError::invalid_input(
            "hashes may contain at most 50 items",
            "Split the image hashes across bounded requests",
        ));
    }
    let mut hashes = Vec::with_capacity(raw.len());
    for hash in raw {
        let Some(hash) = normalize_hash(hash) else {
            return Err(PublicError::invalid_input(
                "hashes contains an invalid image hash",
                "Use hash values returned by list_ad_images",
            ));
        };
        if !hashes.contains(&hash) {
            hashes.push(hash);
        }
    }
    Ok((!hashes.is_empty()).then_some(hashes))
}

async fn fetch_list<T: DeserializeOwned, O>(
    graph: &GraphClient,
    request: ListRequest,
    resource: &str,
    normalize: fn(T) -> Option<O>,
) -> Result<(Vec<O>, Option<String>), PublicError> {
    let payload = graph
        .get_json(&request.endpoint, &request.query)
        .await
        .map_err(PublicError::from)?;
    let raw = serde_json::from_value::<RawList<T>>(payload).map_err(|_| {
        PublicError::invalid_upstream(format!("Meta returned an unexpected {resource} list"))
    })?;
    if raw.data.len() > usize::from(request.page_size) {
        return Err(PublicError::invalid_upstream(format!(
            "Meta returned more {resource} records than requested"
        )));
    }
    let next_cursor = extract_next_cursor(raw.paging, resource)?;
    let items = raw.data.into_iter().filter_map(normalize).collect();
    Ok((items, next_cursor))
}

fn extract_next_cursor(
    paging: Option<RawPaging>,
    resource: &str,
) -> Result<Option<String>, PublicError> {
    let cursor = paging
        .and_then(|paging| paging.cursors)
        .and_then(|cursors| cursors.after)
        .filter(|cursor| !cursor.is_empty());
    if cursor
        .as_ref()
        .is_some_and(|cursor| cursor.chars().count() > MAX_CURSOR_CHARS)
    {
        return Err(PublicError::invalid_upstream(format!(
            "Meta returned an oversized {resource} cursor"
        )));
    }
    Ok(cursor)
}

fn normalize_creative(raw: RawCreative) -> Option<AdCreative> {
    Some(AdCreative {
        id: normalize_numeric_value(raw.id)?,
        account_id: normalize_numeric_value(raw.account_id),
        actor_id: normalize_numeric_value(raw.actor_id),
        instagram_user_id: normalize_numeric_value(raw.instagram_user_id),
        name: bounded_text(raw.name, MAX_NAME_CHARS),
        status: scalar_text(raw.status, 64),
        object_type: bounded_text(raw.object_type, 64),
        object_id: normalize_numeric_value(raw.object_id),
        object_story_id: bounded_identifier(raw.object_story_id),
        effective_object_story_id: bounded_identifier(raw.effective_object_story_id),
        image_hash: raw.image_hash.and_then(|hash| normalize_hash(&hash)),
        video_id: normalize_numeric_value(raw.video_id),
        title: bounded_text(raw.title, MAX_NAME_CHARS),
        body: bounded_text(raw.body, MAX_BODY_CHARS),
        call_to_action_type: bounded_text(raw.call_to_action_type, 64),
        thumbnail_url: bounded_url(raw.thumbnail_url),
        image_url: bounded_url(raw.image_url),
        link_url: bounded_url(raw.link_url),
        instagram_permalink_url: bounded_url(raw.instagram_permalink_url),
    })
}

fn normalize_image(raw: RawImage) -> Option<AdImage> {
    Some(AdImage {
        hash: normalize_hash(raw.hash.as_deref()?)?,
        id: bounded_identifier(raw.id),
        name: bounded_text(raw.name, MAX_NAME_CHARS),
        status: scalar_text(raw.status, 64),
        width: unsigned_value(raw.width),
        height: unsigned_value(raw.height),
        created_time: bounded_text(raw.created_time, 64),
        updated_time: bounded_text(raw.updated_time, 64),
        url: bounded_url(raw.url),
    })
}

fn normalize_video(raw: RawVideo) -> Option<AdVideo> {
    Some(AdVideo {
        id: normalize_numeric_value(raw.id)?,
        title: bounded_text(raw.title, MAX_NAME_CHARS),
        description: bounded_text(raw.description, MAX_BODY_CHARS),
        status: video_status(raw.status),
        length_seconds: nonnegative_number(raw.length),
        created_time: bounded_text(raw.created_time, 64),
        updated_time: bounded_text(raw.updated_time, 64),
        thumbnail_url: bounded_url(raw.picture),
    })
}

fn normalize_numeric_value(raw: Option<Value>) -> Option<String> {
    meta_numeric_value(&raw?)
}

fn normalize_hash(raw: &str) -> Option<String> {
    let hash = raw.trim();
    if hash.is_empty()
        || hash.len() > MAX_HASH_CHARS
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return None;
    }
    Some(hash.to_owned())
}

fn bounded_identifier(raw: Option<String>) -> Option<String> {
    let value = raw?.trim().to_owned();
    if value.is_empty()
        || value.len() > MAX_HASH_CHARS
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return None;
    }
    Some(value)
}

fn bounded_text(raw: Option<String>, max_chars: usize) -> Option<String> {
    let value = raw?.trim().to_owned();
    if value.is_empty() {
        return None;
    }
    if value.chars().count() <= max_chars {
        return Some(value);
    }
    let mut bounded = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    bounded.push('…');
    Some(bounded)
}

fn bounded_url(raw: Option<String>) -> Option<String> {
    let value = raw?.trim().to_owned();
    if value.is_empty() || value.chars().count() > MAX_URL_CHARS {
        return None;
    }
    let mut url = reqwest::Url::parse(&value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    url.set_fragment(None);
    Some(url.into())
}

fn scalar_text(raw: Option<Value>, max_chars: usize) -> Option<String> {
    let value = match raw? {
        Value::String(value) => value,
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        _ => return None,
    };
    bounded_text(Some(value), max_chars)
}

fn video_status(raw: Option<Value>) -> Option<String> {
    let value = raw?;
    if let Value::Object(status) = &value {
        return status
            .get("video_status")
            .or_else(|| status.get("status"))
            .cloned()
            .and_then(|value| scalar_text(Some(value), 64));
    }
    scalar_text(Some(value), 64)
}

fn unsigned_value(raw: Option<Value>) -> Option<u32> {
    match raw? {
        Value::Number(value) => u32::try_from(value.as_u64()?).ok(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

fn nonnegative_number(raw: Option<Value>) -> Option<f64> {
    let number = match raw? {
        Value::Number(value) => value.as_f64()?,
        Value::String(value) => value.parse().ok()?,
        _ => return None,
    };
    (number.is_finite() && number >= 0.0).then_some(number)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        CREATIVE_DETAIL_FIELDS, CREATIVE_LIST_FIELDS, ListAdCreativesInput, ListAdImagesInput,
        ListAdVideosInput, MAX_BODY_CHARS, MAX_CURSOR_CHARS, MAX_HASH_FILTERS, RawCreative,
        RawImage, RawVideo, build_creative_list_request, build_image_list_request,
        build_video_list_request, normalize_creative, normalize_image, normalize_video,
    };

    #[test]
    fn builds_bounded_requests_without_large_specs_or_video_sources() {
        let creative = build_creative_list_request(&ListAdCreativesInput {
            ad_id: "123".to_owned(),
            page_size: Some(10),
            page_cursor: Some("opaque+/= cursor".to_owned()),
        })
        .unwrap();
        assert_eq!(creative.endpoint, "123/adcreatives");
        assert!(
            creative
                .query
                .contains(&("limit".to_owned(), "10".to_owned()))
        );
        assert!(
            creative
                .query
                .contains(&("after".to_owned(), "opaque+/= cursor".to_owned()))
        );
        assert!(!CREATIVE_LIST_FIELDS.contains("object_story_spec"));
        assert!(!CREATIVE_LIST_FIELDS.contains("asset_feed_spec"));
        assert!(!CREATIVE_DETAIL_FIELDS.contains("object_story_spec"));
        assert!(!CREATIVE_DETAIL_FIELDS.contains("asset_feed_spec"));

        let video = build_video_list_request(&ListAdVideosInput {
            ad_account_id: "act_456".to_owned(),
            page_size: None,
            page_cursor: None,
        })
        .unwrap();
        assert_eq!(video.endpoint, "act_456/advideos");
        let fields = video
            .query
            .iter()
            .find(|(key, _)| key == "fields")
            .map(|(_, value)| value.as_str())
            .unwrap();
        assert!(!fields.split(',').any(|field| field == "source"));
    }

    #[test]
    fn validates_ids_pages_cursors_and_hash_filters() {
        assert!(
            build_creative_list_request(&ListAdCreativesInput {
                ad_id: "../123".to_owned(),
                page_size: None,
                page_cursor: None,
            })
            .is_err()
        );
        assert!(
            build_video_list_request(&ListAdVideosInput {
                ad_account_id: "123".to_owned(),
                page_size: Some(101),
                page_cursor: None,
            })
            .is_err()
        );
        assert!(
            build_video_list_request(&ListAdVideosInput {
                ad_account_id: "123".to_owned(),
                page_size: None,
                page_cursor: Some("x".repeat(MAX_CURSOR_CHARS + 1)),
            })
            .is_err()
        );

        let images = build_image_list_request(&ListAdImagesInput {
            ad_account_id: "456".to_owned(),
            page_size: Some(5),
            page_cursor: None,
            hashes: Some(vec!["abc123".to_owned(), "abc123".to_owned()]),
        })
        .unwrap();
        assert_eq!(images.endpoint, "act_456/adimages");
        assert!(
            images
                .query
                .contains(&("hashes".to_owned(), "[\"abc123\"]".to_owned()))
        );

        assert!(
            build_image_list_request(&ListAdImagesInput {
                ad_account_id: "456".to_owned(),
                page_size: None,
                page_cursor: None,
                hashes: Some(vec!["ok".to_owned(); MAX_HASH_FILTERS + 1]),
            })
            .is_err()
        );
    }

    #[test]
    fn normalizes_only_compact_creative_metadata() {
        let raw: RawCreative = serde_json::from_value(json!({
            "id": "1001",
            "account_id": 2002,
            "name": "  Launch creative  ",
            "body": "x".repeat(MAX_BODY_CHARS + 50),
            "object_story_id": "3003_4004",
            "image_hash": "abcd-1234",
            "thumbnail_url": "https://cdn.example/preview.jpg#fragment",
            "object_story_spec": {"link_data": {"message": "must not escape"}},
            "asset_feed_spec": {"bodies": [{"text": "must not escape"}]}
        }))
        .unwrap();

        let creative = normalize_creative(raw).unwrap();
        assert_eq!(creative.id, "1001");
        assert_eq!(creative.account_id.as_deref(), Some("2002"));
        assert_eq!(creative.name.as_deref(), Some("Launch creative"));
        assert_eq!(
            creative.body.as_ref().unwrap().chars().count(),
            MAX_BODY_CHARS
        );
        assert_eq!(
            creative.thumbnail_url.as_deref(),
            Some("https://cdn.example/preview.jpg")
        );
        let encoded = serde_json::to_string(&creative).unwrap();
        assert!(!encoded.contains("object_story_spec"));
        assert!(!encoded.contains("asset_feed_spec"));
    }

    #[test]
    fn normalizes_image_and_video_metadata_without_media_sources() {
        let image: RawImage = serde_json::from_value(json!({
            "hash": "abc123",
            "name": " Square ",
            "status": 1,
            "width": "1080",
            "height": 1080,
            "url": "data:image/png;base64,secret"
        }))
        .unwrap();
        let image = normalize_image(image).unwrap();
        assert_eq!(image.hash, "abc123");
        assert_eq!(image.width, Some(1080));
        assert_eq!(image.status.as_deref(), Some("1"));
        assert!(image.url.is_none());

        let video: RawVideo = serde_json::from_value(json!({
            "id": "987",
            "title": " Demo ",
            "length": "12.5",
            "status": {"video_status": "ready", "processing_phase": {"status": "complete"}},
            "picture": "https://cdn.example/thumb.jpg",
            "source": "data:video/mp4;base64,secret"
        }))
        .unwrap();
        let video = normalize_video(video).unwrap();
        assert_eq!(video.id, "987");
        assert_eq!(video.status.as_deref(), Some("ready"));
        assert_eq!(video.length_seconds, Some(12.5));
        let encoded = serde_json::to_string(&video).unwrap();
        assert!(!encoded.contains("source"));
        assert!(!encoded.contains("base64"));
    }
}
