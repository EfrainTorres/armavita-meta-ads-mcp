use std::{
    io::Write,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD, write::EncoderStringWriter};
use reqwest::{Url, header::CONTENT_TYPE};
use rmcp::{
    model::{CallToolResult, ContentBlock},
    schemars::{self, JsonSchema},
};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::numeric_owned as normalize_numeric_id,
};

// Field expansion keeps the normal path to one Graph lookup plus, when a
// stable hash exists, one ad-image-library lookup for the full CDN URL.
const AD_IMAGE_FIELDS: &str =
    "account_id,creative{id,image_hash,image_url,thumbnail_url,object_story_spec,asset_feed_spec}";
const AD_LIBRARY_FIELDS: &str = "hash,url,url_128";
const MAX_HASH_CHARS: usize = 128;
const MAX_SOURCE_URL_CHARS: usize = 8_192;
const MAX_SOURCE_CANDIDATES: usize = 4;
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const DNS_TIMEOUT: Duration = Duration::from_secs(5);
const TOTAL_MEDIA_TIMEOUT: Duration = Duration::from_secs(35);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadAdImageInput {
    /// Numeric Meta ad ID.
    pub ad_id: String,
}

#[derive(Debug, Deserialize)]
struct RawAdReference {
    account_id: Option<Value>,
    creative: Option<RawCreativeReference>,
}

#[derive(Debug, Deserialize)]
struct RawCreativeReference {
    id: Option<Value>,
    image_hash: Option<String>,
    image_url: Option<String>,
    thumbnail_url: Option<String>,
    object_story_spec: Option<Value>,
    asset_feed_spec: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct RawImagePage {
    #[serde(default)]
    data: Vec<RawAdImage>,
}

#[derive(Debug, Deserialize)]
struct RawAdImage {
    hash: Option<String>,
    url: Option<String>,
    url_128: Option<String>,
}

/// Return an ad's primary image as native MCP image content.
///
/// Binary data is base64-encoded only at the protocol boundary and is never
/// repeated in structured JSON. Failures contain no source URL or query data.
pub(crate) async fn read_ad_image(graph: &GraphClient, input: ReadAdImageInput) -> CallToolResult {
    let candidates = match resolve_image_candidates(graph, &input.ad_id).await {
        Ok(candidates) => candidates,
        Err(error) => return error_result(error),
    };

    let mut saw_allowed_source = false;
    let mut last_error = None;
    for candidate in candidates {
        let url = match validate_media_url(&candidate) {
            Ok(url) => url,
            Err(error) => {
                last_error = Some(error);
                continue;
            }
        };
        saw_allowed_source = true;
        if let Err(error) = validate_public_dns(&url).await {
            last_error = Some(error);
            continue;
        }

        match tokio::time::timeout(TOTAL_MEDIA_TIMEOUT, download_image(graph, url)).await {
            Ok(Ok(content)) => return CallToolResult::success(vec![content]),
            Ok(Err(error)) => last_error = Some(error),
            Err(_) => last_error = Some(MediaReadError::Network),
        }
    }

    let error = if !saw_allowed_source {
        PublicError {
            code: "UNSAFE_MEDIA_SOURCE".to_owned(),
            message: "Meta did not return an image on an approved Meta CDN".to_owned(),
            retryable: false,
            action: Some("Store the creative image in the Meta ad image library".to_owned()),
        }
    } else {
        media_public_error(last_error.unwrap_or(MediaReadError::Network))
    };
    error_result(error)
}

async fn resolve_image_candidates(
    graph: &GraphClient,
    raw_ad_id: &str,
) -> Result<Vec<String>, PublicError> {
    let ad_id = normalize_numeric_id(raw_ad_id).ok_or_else(|| {
        PublicError::invalid_input(
            "ad_id must be a numeric Meta ad ID",
            "Use the ID returned by list_ads",
        )
    })?;
    let payload = graph
        .get_json(&ad_id, &[("fields".to_owned(), AD_IMAGE_FIELDS.to_owned())])
        .await
        .map_err(PublicError::from)?;
    let reference = serde_json::from_value::<RawAdReference>(payload)
        .map_err(|_| PublicError::invalid_upstream("Meta returned malformed ad image metadata"))?;

    let account_id = reference
        .account_id
        .as_ref()
        .and_then(value_to_numeric_id)
        .ok_or_else(|| PublicError::invalid_upstream("Meta omitted the ad account ID"))?;
    let creative = reference
        .creative
        .ok_or_else(|| PublicError::invalid_upstream("Meta omitted the ad creative"))?;
    if creative.id.as_ref().and_then(value_to_numeric_id).is_none() {
        return Err(PublicError::invalid_upstream(
            "Meta omitted a valid numeric creative ID",
        ));
    }

    let mut candidates = Vec::with_capacity(MAX_SOURCE_CANDIDATES);
    let mut library_error = None;
    if let Some(hash) = first_image_hash(&creative) {
        match resolve_library_url(graph, &account_id, &hash).await {
            Ok(urls) => urls
                .into_iter()
                .for_each(|url| push_candidate(&mut candidates, url)),
            Err(error) => library_error = Some(error),
        }
    }
    append_creative_urls(&creative, &mut candidates);

    if candidates.is_empty() {
        Err(library_error.unwrap_or_else(|| {
            PublicError::invalid_upstream("Meta returned no usable image source for this ad")
        }))
    } else {
        Ok(candidates)
    }
}

async fn resolve_library_url(
    graph: &GraphClient,
    account_id: &str,
    hash: &str,
) -> Result<Vec<String>, PublicError> {
    let hashes = serde_json::to_string(&[hash])
        .map_err(|_| PublicError::invalid_upstream("Could not encode Meta image hash lookup"))?;
    let payload = graph
        .get_json(
            &format!("act_{account_id}/adimages"),
            &[
                ("fields".to_owned(), AD_LIBRARY_FIELDS.to_owned()),
                ("hashes".to_owned(), hashes),
                ("limit".to_owned(), "1".to_owned()),
            ],
        )
        .await
        .map_err(PublicError::from)?;
    let page = serde_json::from_value::<RawImagePage>(payload).map_err(|_| {
        PublicError::invalid_upstream("Meta returned malformed ad image library metadata")
    })?;
    let image = page
        .data
        .into_iter()
        .find(|image| image.hash.as_deref() == Some(hash))
        .ok_or_else(|| {
            PublicError::invalid_upstream("Meta did not return the requested ad image hash")
        })?;

    Ok(image.url.into_iter().chain(image.url_128).collect())
}

fn first_image_hash(creative: &RawCreativeReference) -> Option<String> {
    let direct = creative
        .image_hash
        .as_deref()
        .filter(|hash| valid_hash(hash));
    if let Some(hash) = direct {
        return Some(hash.to_owned());
    }

    if let Some(story) = &creative.object_story_spec {
        for pointer in [
            "/link_data/image_hash",
            "/link_data/ad_image_hash",
            "/photo_data/image_hash",
            "/video_data/image_hash",
            "/template_data/image_hash",
        ] {
            if let Some(hash) = story
                .pointer(pointer)
                .and_then(Value::as_str)
                .filter(|hash| valid_hash(hash))
            {
                return Some(hash.to_owned());
            }
        }
        if let Some(children) = story
            .pointer("/link_data/child_attachments")
            .and_then(Value::as_array)
        {
            for child in children.iter().take(MAX_SOURCE_CANDIDATES) {
                if let Some(hash) = child
                    .get("image_hash")
                    .and_then(Value::as_str)
                    .filter(|hash| valid_hash(hash))
                {
                    return Some(hash.to_owned());
                }
            }
        }
    }

    creative
        .asset_feed_spec
        .as_ref()
        .and_then(|spec| spec.get("images"))
        .and_then(Value::as_array)
        .and_then(|images| {
            images.iter().take(MAX_SOURCE_CANDIDATES).find_map(|image| {
                image
                    .get("hash")
                    .and_then(Value::as_str)
                    .filter(|hash| valid_hash(hash))
                    .map(str::to_owned)
            })
        })
}

fn append_creative_urls(creative: &RawCreativeReference, candidates: &mut Vec<String>) {
    if let Some(url) = &creative.image_url {
        push_candidate(candidates, url.clone());
    }
    // Keep Meta's own thumbnail ahead of deeply nested fallbacks so an
    // advertiser-hosted image_url cannot crowd out the safe CDN source.
    if let Some(url) = &creative.thumbnail_url {
        push_candidate(candidates, url.clone());
    }
    if let Some(story) = &creative.object_story_spec {
        for pointer in [
            "/link_data/picture",
            "/link_data/image_url",
            "/video_data/image_url",
            "/photo_data/url",
        ] {
            if let Some(url) = story.pointer(pointer).and_then(Value::as_str) {
                push_candidate(candidates, url.to_owned());
            }
        }
    }
    if let Some(images) = creative
        .asset_feed_spec
        .as_ref()
        .and_then(|spec| spec.get("images"))
        .and_then(Value::as_array)
    {
        for image in images.iter().take(MAX_SOURCE_CANDIDATES) {
            if let Some(url) = image.get("url").and_then(Value::as_str) {
                push_candidate(candidates, url.to_owned());
            }
        }
    }
}

fn push_candidate(candidates: &mut Vec<String>, candidate: String) {
    if candidates.len() < MAX_SOURCE_CANDIDATES
        && !candidate.is_empty()
        && candidate.len() <= MAX_SOURCE_URL_CHARS
        && !candidates.contains(&candidate)
    {
        candidates.push(candidate);
    }
}

fn value_to_numeric_id(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => {
            let value = value.strip_prefix("act_").unwrap_or(value);
            normalize_numeric_id(value)
        }
        Value::Number(value) if value.is_u64() => normalize_numeric_id(&value.to_string()),
        _ => None,
    }
}

fn valid_hash(hash: &str) -> bool {
    !hash.is_empty()
        && hash.len() <= MAX_HASH_CHARS
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn validate_media_url(raw: &str) -> Result<Url, MediaReadError> {
    if raw.len() > MAX_SOURCE_URL_CHARS {
        return Err(MediaReadError::UnsafeSource);
    }
    let url = Url::parse(raw).map_err(|_| MediaReadError::UnsafeSource)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return Err(MediaReadError::UnsafeSource);
    }
    let host = url
        .host_str()
        .filter(|host| approved_cdn_host(host))
        .ok_or(MediaReadError::UnsafeSource)?;
    if host.parse::<IpAddr>().is_ok() {
        return Err(MediaReadError::UnsafeSource);
    }
    Ok(url)
}

fn approved_cdn_host(host: &str) -> bool {
    host.ends_with(".fbcdn.net")
        || host.ends_with(".cdninstagram.com")
        || matches!(host, "lookaside.fbsbx.com" | "platform-lookaside.fbsbx.com")
}

async fn validate_public_dns(url: &Url) -> Result<(), MediaReadError> {
    let host = url.host_str().ok_or(MediaReadError::UnsafeSource)?;
    let addresses = tokio::time::timeout(DNS_TIMEOUT, tokio::net::lookup_host((host, 443)))
        .await
        .map_err(|_| MediaReadError::Network)?
        .map_err(|_| MediaReadError::Network)?;
    let mut found = false;
    for address in addresses {
        found = true;
        if !is_public_ip(address.ip()) {
            return Err(MediaReadError::UnsafeSource);
        }
    }
    found.then_some(()).ok_or(MediaReadError::Network)
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 88 && c == 99)
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(ipv4) = ip.to_ipv4() {
        return is_public_ipv4(ipv4);
    }
    let segments = ip.segments();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] & 0xffc0) == 0xfec0
        || (segments[0] == 0x2001 && segments[1] == 0x0db8))
}

async fn download_image(graph: &GraphClient, url: Url) -> Result<ContentBlock, MediaReadError> {
    let mut response = graph
        .get_media_response(url)
        .await
        .map_err(|_| MediaReadError::Network)?;
    if !response.status().is_success() {
        return Err(MediaReadError::HttpStatus);
    }
    let mime = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(normalize_image_mime)
        .ok_or(MediaReadError::UnsupportedMime)?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_IMAGE_BYTES as u64)
    {
        return Err(MediaReadError::TooLarge);
    }

    let capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .map(encoded_capacity)
        .unwrap_or(0);
    let output = String::with_capacity(capacity);
    let mut encoder = EncoderStringWriter::from_consumer(output, &STANDARD);
    let mut prefix = Vec::with_capacity(12);
    let mut bytes_read = 0_usize;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| MediaReadError::Network)?
    {
        bytes_read = checked_image_size(bytes_read, chunk.len())?;
        if prefix.len() < 12 {
            let take = (12 - prefix.len()).min(chunk.len());
            prefix.extend_from_slice(&chunk[..take]);
        }
        encoder
            .write_all(&chunk)
            .map_err(|_| MediaReadError::Network)?;
    }
    if bytes_read == 0 {
        return Err(MediaReadError::Empty);
    }
    if !signature_matches(mime, &prefix) {
        return Err(MediaReadError::InvalidSignature);
    }

    Ok(ContentBlock::image(encoder.into_inner(), mime))
}

fn normalize_image_mime(raw: &str) -> Option<&'static str> {
    let mime = raw.split(';').next()?.trim();
    if mime.eq_ignore_ascii_case("image/jpeg") || mime.eq_ignore_ascii_case("image/jpg") {
        Some("image/jpeg")
    } else if mime.eq_ignore_ascii_case("image/png") {
        Some("image/png")
    } else if mime.eq_ignore_ascii_case("image/webp") {
        Some("image/webp")
    } else if mime.eq_ignore_ascii_case("image/gif") {
        Some("image/gif")
    } else {
        None
    }
}

fn signature_matches(mime: &str, bytes: &[u8]) -> bool {
    match mime {
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/webp" => bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP",
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        _ => false,
    }
}

fn checked_image_size(current: usize, chunk: usize) -> Result<usize, MediaReadError> {
    current
        .checked_add(chunk)
        .filter(|size| *size <= MAX_IMAGE_BYTES)
        .ok_or(MediaReadError::TooLarge)
}

fn encoded_capacity(raw_bytes: usize) -> usize {
    (raw_bytes.saturating_add(2) / 3).saturating_mul(4)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MediaReadError {
    UnsafeSource,
    Network,
    HttpStatus,
    UnsupportedMime,
    TooLarge,
    Empty,
    InvalidSignature,
}

fn media_public_error(error: MediaReadError) -> PublicError {
    match error {
        MediaReadError::UnsafeSource => PublicError {
            code: "UNSAFE_MEDIA_SOURCE".to_owned(),
            message: "Meta returned an image source that failed network safety validation"
                .to_owned(),
            retryable: false,
            action: Some("Store the creative image in the Meta ad image library".to_owned()),
        },
        MediaReadError::UnsupportedMime | MediaReadError::InvalidSignature => PublicError {
            code: "UNSUPPORTED_MEDIA".to_owned(),
            message: "Meta returned an unsupported or mismatched image format".to_owned(),
            retryable: false,
            action: Some("Use a JPEG, PNG, WebP, or GIF creative image".to_owned()),
        },
        MediaReadError::TooLarge => PublicError {
            code: "MEDIA_TOO_LARGE".to_owned(),
            message: format!("The ad image exceeds the {MAX_IMAGE_BYTES} byte safety limit"),
            retryable: false,
            action: Some("Use a smaller creative image".to_owned()),
        },
        MediaReadError::Network | MediaReadError::HttpStatus | MediaReadError::Empty => {
            PublicError {
                code: "MEDIA_UNAVAILABLE".to_owned(),
                message: "The Meta CDN image could not be retrieved".to_owned(),
                retryable: true,
                action: Some("Retry once; the signed Meta image URL may have expired".to_owned()),
            }
        }
    }
}

fn error_result(error: PublicError) -> CallToolResult {
    let response = ToolResponse::<()>::error(error);
    let value = serde_json::to_value(response).unwrap_or_else(|_| {
        serde_json::json!({
            "status": "error",
            "error": {
                "code": "INTERNAL_ERROR",
                "message": "Could not serialize the media error",
                "retryable": false
            }
        })
    });
    CallToolResult::structured_error(value)
}

#[cfg(test)]
mod tests {
    use std::{io::Write, net::IpAddr};

    use base64::{engine::general_purpose::STANDARD, write::EncoderStringWriter};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use crate::{MetaConfig, graph::GraphClient};

    use super::{
        MAX_IMAGE_BYTES, MediaReadError, checked_image_size, encoded_capacity, is_public_ip,
        normalize_image_mime, resolve_image_candidates, signature_matches, validate_media_url,
    };

    #[test]
    fn accepts_only_bounded_https_meta_cdn_urls() {
        assert!(
            validate_media_url("https://scontent-ord5-3.xx.fbcdn.net/v/image.jpg?oh=signed")
                .is_ok()
        );
        assert!(
            validate_media_url("https://scontent-lax3-2.cdninstagram.com/v/image.webp").is_ok()
        );
        assert!(
            validate_media_url("https://platform-lookaside.fbsbx.com/platform/image.png").is_ok()
        );
        for rejected in [
            "http://scontent.xx.fbcdn.net/image.jpg",
            "https://fbcdn.net.evil.example/image.jpg",
            "https://user:pass@scontent.xx.fbcdn.net/image.jpg",
            "https://127.0.0.1/image.jpg",
            "https://scontent.xx.fbcdn.net:444/image.jpg",
        ] {
            assert_eq!(
                validate_media_url(rejected).unwrap_err(),
                MediaReadError::UnsafeSource
            );
        }
    }

    #[test]
    fn rejects_private_and_special_dns_targets() {
        for rejected in [
            "0.0.0.0",
            "10.0.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.1.1",
            "172.16.0.1",
            "192.168.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
        ] {
            assert!(!is_public_ip(rejected.parse::<IpAddr>().unwrap()));
        }
        assert!(is_public_ip("8.8.8.8".parse().unwrap()));
        assert!(is_public_ip("2606:4700:4700::1111".parse().unwrap()));
    }

    #[test]
    fn validates_mime_signature_and_size() {
        assert_eq!(
            normalize_image_mime("image/jpeg; charset=binary"),
            Some("image/jpeg")
        );
        assert_eq!(normalize_image_mime("IMAGE/PNG"), Some("image/png"));
        assert_eq!(normalize_image_mime("image/svg+xml"), None);
        assert!(signature_matches("image/jpeg", &[0xff, 0xd8, 0xff, 0x00]));
        assert!(signature_matches("image/png", b"\x89PNG\r\n\x1a\nrest"));
        assert!(signature_matches("image/webp", b"RIFF0000WEBPrest"));
        assert!(!signature_matches("image/png", b"<svg></svg>"));
        assert_eq!(
            checked_image_size(MAX_IMAGE_BYTES - 1, 1),
            Ok(MAX_IMAGE_BYTES)
        );
        assert_eq!(
            checked_image_size(MAX_IMAGE_BYTES, 1),
            Err(MediaReadError::TooLarge)
        );
    }

    #[test]
    fn streams_standard_base64_across_chunk_boundaries() {
        let mut encoder = EncoderStringWriter::from_consumer(
            String::with_capacity(encoded_capacity(6)),
            &STANDARD,
        );
        encoder.write_all(b"f").unwrap();
        encoder.write_all(b"oo").unwrap();
        encoder.write_all(b"ba").unwrap();
        encoder.write_all(b"r").unwrap();
        assert_eq!(encoder.into_inner(), "Zm9vYmFy");

        let mut one = EncoderStringWriter::from_consumer(String::with_capacity(4), &STANDARD);
        one.write_all(b"f").unwrap();
        assert_eq!(one.into_inner(), "Zg==");
        let mut two = EncoderStringWriter::from_consumer(String::with_capacity(4), &STANDARD);
        two.write_all(b"fo").unwrap();
        assert_eq!(two.into_inner(), "Zm8=");
    }

    #[tokio::test]
    async fn resolves_nested_creative_hash_through_ad_image_library() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for (expected_path, body) in [
                (
                    "/123?",
                    r#"{"account_id":"42","creative":{"id":"77","image_hash":"abc123"}}"#,
                ),
                (
                    "/act_42/adimages?",
                    r#"{"data":[{"hash":"abc123","url":"https://scontent.xx.fbcdn.net/image.jpg"}]}"#,
                ),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0_u8; 8_192];
                let bytes_read = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..bytes_read]);
                assert!(request.starts_with(&format!("GET {expected_path}")));
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });

        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        ))
        .unwrap();
        let candidates = resolve_image_candidates(&graph, "123").await.unwrap();
        assert_eq!(
            candidates.first().map(String::as_str),
            Some("https://scontent.xx.fbcdn.net/image.jpg")
        );
        server.await.unwrap();
    }
}
