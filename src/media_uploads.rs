// Copyright (C) 2025 ArmaVita LLC
// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    io::SeekFrom,
    path::{Component, Path, PathBuf},
};

use reqwest::Url;
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::{
    error::{GraphError, PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{
        ad_account as normalize_account_id, ad_account_digits, numeric_value as value_numeric_id,
    },
    mutation_result::{ambiguous_mutation_result, mutation_error_without_blind_retry},
};

const MAX_HASH_CHARS: usize = 128;
const MAX_RELATIVE_PATH_CHARS: usize = 1_024;
const MAX_RELATIVE_PATH_BYTES: usize = 4 * 1_024;
const MAX_SOURCE_URL_CHARS: usize = 8_192;
const MAX_NAME_CHARS: usize = 255;
const MAX_DESCRIPTION_CHARS: usize = 5_000;
const MAX_LOCAL_IMAGE_BYTES: u64 = 15 * 1024 * 1024;
const MAX_LOCAL_VIDEO_BYTES: u64 = 512 * 1024 * 1024;
const MEDIA_HEADER_BYTES: usize = 16;

/// Add one image to an ad account. Local files are relative to the configured
/// `META_MEDIA_ROOT`; cross-account copies require access to both accounts.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UploadAdImageAssetInput {
    /// Destination Meta ad-account ID, with or without `act_`.
    #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
    pub ad_account_id: String,
    /// A bounded local image or an existing image in another accessible account.
    pub source: AdImageAssetSource,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AdImageAssetSource {
    /// Stream a JPEG or PNG below 15 MiB from `META_MEDIA_ROOT`.
    LocalFile {
        /// Relative path below `META_MEDIA_ROOT`; absolute paths and traversal are rejected.
        #[schemars(length(min = 1, max = 1_024))]
        relative_path: String,
    },
    /// Copy an existing ad image without downloading or re-encoding it.
    ExistingAccountImage {
        /// Source ad-account ID, with or without `act_`.
        #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
        source_ad_account_id: String,
        /// Image hash returned by `list_ad_images` on the source account.
        #[schemars(length(min = 1, max = 128), regex(pattern = "^[A-Za-z0-9_-]{1,128}$"))]
        image_hash: String,
    },
}

/// Add one video to an ad account. Use a local MP4/MOV below 512 MiB or let
/// Meta fetch a larger video from a credential-free HTTPS URL.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UploadAdVideoAssetInput {
    /// Destination Meta ad-account ID, with or without `act_`.
    #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
    pub ad_account_id: String,
    pub source: AdVideoAssetSource,
    /// Optional internal video name, up to 255 characters.
    #[schemars(length(min = 1, max = 255))]
    pub name: Option<String>,
    /// Optional video title, up to 255 characters.
    #[schemars(length(min = 1, max = 255))]
    pub title: Option<String>,
    /// Optional video description, up to 5,000 characters.
    #[schemars(length(min = 1, max = 5_000))]
    pub description: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AdVideoAssetSource {
    /// Stream an MP4 or MOV below 512 MiB from `META_MEDIA_ROOT`.
    LocalFile {
        /// Relative path below `META_MEDIA_ROOT`; absolute paths and traversal are rejected.
        #[schemars(length(min = 1, max = 1_024))]
        relative_path: String,
    },
    /// Ask Meta to fetch a video from a public HTTPS URL.
    HttpsUrl {
        /// Credential-free HTTPS URL. Query parameters are allowed for signed CDN URLs.
        #[schemars(length(min = 1, max = 8_192), url)]
        url: String,
    },
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UploadedAdImageAsset {
    pub ad_image_hash: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UploadedAdVideoAsset {
    pub ad_video_id: String,
}

#[derive(Debug, PartialEq, Eq)]
struct MutationRequest {
    endpoint: String,
    form: Vec<(String, String)>,
}

struct OpenedMedia {
    file: tokio::fs::File,
    size: u64,
    file_name: &'static str,
    mime_type: &'static str,
}

#[derive(Clone, Copy)]
enum LocalMediaKind {
    Image,
    Video,
}

#[derive(Clone, Copy)]
struct DetectedMedia {
    file_name: &'static str,
    mime_type: &'static str,
}

#[derive(Serialize)]
struct CopyImageFrom<'a> {
    source_account_id: &'a str,
    hash: &'a str,
}

pub(crate) async fn upload_ad_image_asset(
    graph: &GraphClient,
    media_root: Option<&Path>,
    input: UploadAdImageAssetInput,
) -> ToolResponse<UploadedAdImageAsset> {
    let destination = match normalize_account_id(&input.ad_account_id) {
        Some(account_id) => account_id,
        None => {
            return ToolResponse::error(PublicError::invalid_input(
                "ad_account_id must be a numeric Meta ad-account ID",
                "Use an ID such as `act_123456789` or `123456789`",
            ));
        }
    };
    let endpoint = format!("{destination}/adimages");

    let payload = match input.source {
        AdImageAssetSource::LocalFile { relative_path } => {
            let media = match open_local_media(
                media_root,
                &relative_path,
                LocalMediaKind::Image,
                MAX_LOCAL_IMAGE_BYTES,
            )
            .await
            {
                Ok(media) => media,
                Err(error) => return ToolResponse::error(error),
            };
            match graph
                .post_multipart_file_json(
                    &endpoint,
                    "filename",
                    media.file,
                    media.size,
                    media.file_name.to_owned(),
                    media.mime_type,
                    Vec::new(),
                )
                .await
            {
                Ok(payload) => payload,
                Err(error) => {
                    return ToolResponse::error(mutation_error_without_blind_retry(
                        error,
                        "List ad images before uploading the file again",
                    ));
                }
            }
        }
        AdImageAssetSource::ExistingAccountImage {
            source_ad_account_id,
            image_hash,
        } => {
            let request =
                match build_image_copy_request(&destination, &source_ad_account_id, &image_hash) {
                    Ok(request) => request,
                    Err(error) => return ToolResponse::error(error),
                };
            match graph.post_form_json(&request.endpoint, &request.form).await {
                Ok(payload) => payload,
                Err(error) => {
                    return ToolResponse::error(mutation_error_without_blind_retry(
                        error,
                        "List destination-account ad images before copying again",
                    ));
                }
            }
        }
    };

    match uploaded_image_hash(&payload) {
        Some(ad_image_hash) => ToolResponse::success(UploadedAdImageAsset { ad_image_hash }),
        None => ToolResponse::error(ambiguous_upload_result(
            "Meta did not return exactly one uploaded image hash",
            "List ad images before attempting another upload",
        )),
    }
}

pub(crate) async fn upload_ad_video_asset(
    graph: &GraphClient,
    media_root: Option<&Path>,
    input: UploadAdVideoAssetInput,
) -> ToolResponse<UploadedAdVideoAsset> {
    let destination = match normalize_account_id(&input.ad_account_id) {
        Some(account_id) => account_id,
        None => {
            return ToolResponse::error(PublicError::invalid_input(
                "ad_account_id must be a numeric Meta ad-account ID",
                "Use an ID such as `act_123456789` or `123456789`",
            ));
        }
    };
    let metadata = match video_metadata_form(input.name, input.title, input.description) {
        Ok(metadata) => metadata,
        Err(error) => return ToolResponse::error(error),
    };
    let endpoint = format!("{destination}/advideos");

    let payload = match input.source {
        AdVideoAssetSource::LocalFile { relative_path } => {
            let media = match open_local_media(
                media_root,
                &relative_path,
                LocalMediaKind::Video,
                MAX_LOCAL_VIDEO_BYTES,
            )
            .await
            {
                Ok(media) => media,
                Err(error) => return ToolResponse::error(error),
            };
            match graph
                .post_multipart_file_json(
                    &endpoint,
                    "source",
                    media.file,
                    media.size,
                    media.file_name.to_owned(),
                    media.mime_type,
                    metadata,
                )
                .await
            {
                Ok(payload) => payload,
                Err(error) => {
                    return ToolResponse::error(mutation_error_without_blind_retry(
                        error,
                        "List ad videos before uploading the file again",
                    ));
                }
            }
        }
        AdVideoAssetSource::HttpsUrl { url } => {
            let request = match build_video_url_request(&endpoint, &url, metadata) {
                Ok(request) => request,
                Err(error) => return ToolResponse::error(error),
            };
            match graph.post_form_json(&request.endpoint, &request.form).await {
                Ok(payload) => payload,
                Err(error) => return ToolResponse::error(remote_video_error(error)),
            }
        }
    };

    match uploaded_video_id(&payload) {
        Some(ad_video_id) => ToolResponse::success(UploadedAdVideoAsset { ad_video_id }),
        None => ToolResponse::error(ambiguous_upload_result(
            "Meta did not return the uploaded video ID",
            "List ad videos before attempting another upload",
        )),
    }
}

fn build_image_copy_request(
    destination: &str,
    source_raw: &str,
    hash_raw: &str,
) -> Result<MutationRequest, PublicError> {
    let source = normalize_account_digits(source_raw).ok_or_else(|| {
        PublicError::invalid_input(
            "source_ad_account_id must be a numeric Meta ad-account ID",
            "Use the account ID returned by list_ad_accounts",
        )
    })?;
    let hash = normalize_image_hash(hash_raw).ok_or_else(|| {
        PublicError::invalid_input(
            "image_hash is invalid",
            "Use a hash returned by list_ad_images on the source account",
        )
    })?;
    let copy_from = serde_json::to_string(&CopyImageFrom {
        source_account_id: &source,
        hash: &hash,
    })
    .map_err(|_| PublicError::invalid_upstream("Could not encode the image-copy request"))?;

    Ok(MutationRequest {
        endpoint: format!("{destination}/adimages"),
        form: vec![("copy_from".to_owned(), copy_from)],
    })
}

fn build_video_url_request(
    endpoint: &str,
    raw_url: &str,
    mut metadata: Vec<(String, String)>,
) -> Result<MutationRequest, PublicError> {
    let url = normalize_remote_video_url(raw_url)?;
    metadata.insert(0, ("file_url".to_owned(), url));
    Ok(MutationRequest {
        endpoint: endpoint.to_owned(),
        form: metadata,
    })
}

fn video_metadata_form(
    name: Option<String>,
    title: Option<String>,
    description: Option<String>,
) -> Result<Vec<(String, String)>, PublicError> {
    let mut form = Vec::with_capacity(3);
    push_optional_text(&mut form, "name", name, MAX_NAME_CHARS)?;
    push_optional_text(&mut form, "title", title, MAX_NAME_CHARS)?;
    push_optional_text(&mut form, "description", description, MAX_DESCRIPTION_CHARS)?;
    Ok(form)
}

fn push_optional_text(
    form: &mut Vec<(String, String)>,
    field: &'static str,
    value: Option<String>,
    max_chars: usize,
) -> Result<(), PublicError> {
    let Some(value) = value else {
        return Ok(());
    };
    let value = value.trim();
    if value.is_empty() || value.chars().count() > max_chars {
        return Err(PublicError::invalid_input(
            format!("{field} must contain 1 through {max_chars} characters"),
            format!("Shorten {field} or omit it"),
        ));
    }
    form.push((field.to_owned(), value.to_owned()));
    Ok(())
}

async fn open_local_media(
    media_root: Option<&Path>,
    relative_path: &str,
    kind: LocalMediaKind,
    max_bytes: u64,
) -> Result<OpenedMedia, PublicError> {
    let root = media_root.ok_or_else(|| PublicError {
        code: "MEDIA_ROOT_REQUIRED".to_owned(),
        message: "Local media uploads are not configured".to_owned(),
        retryable: false,
        action: Some("Set META_MEDIA_ROOT to the one directory allowed for uploads".to_owned()),
    })?;
    if !root.is_absolute() {
        return Err(PublicError::invalid_input(
            "The configured media root is invalid",
            "Set META_MEDIA_ROOT to an existing absolute directory",
        ));
    }
    let relative = validate_relative_path(relative_path)?;
    let canonical = tokio::fs::canonicalize(root.join(relative))
        .await
        .map_err(|_| unreadable_local_file())?;
    if !canonical.starts_with(root) {
        return Err(PublicError::invalid_input(
            "relative_path resolves outside META_MEDIA_ROOT",
            "Choose a file contained by the configured media root",
        ));
    }

    // Reject FIFOs and devices before opening: a FIFO read can block forever.
    // The post-open check below still validates the object actually opened.
    let metadata = tokio::fs::metadata(&canonical)
        .await
        .map_err(|_| unreadable_local_file())?;
    if !metadata.is_file() {
        return Err(PublicError::invalid_input(
            "relative_path is not a regular file",
            "Choose one regular image or video file",
        ));
    }
    let mut file = tokio::fs::File::open(&canonical)
        .await
        .map_err(|_| unreadable_local_file())?;
    let metadata = file.metadata().await.map_err(|_| unreadable_local_file())?;
    if !metadata.is_file() {
        return Err(PublicError::invalid_input(
            "relative_path is not a regular file",
            "Choose one regular image or video file",
        ));
    }
    let size = metadata.len();
    if size == 0 || size > max_bytes {
        return Err(PublicError::invalid_input(
            format!(
                "Local {} must contain 1 byte through {} MiB",
                media_label(kind),
                max_bytes / (1024 * 1024)
            ),
            larger_media_action(kind),
        ));
    }

    let mut header = [0_u8; MEDIA_HEADER_BYTES];
    let mut read = 0_usize;
    while read < header.len() {
        let count = file
            .read(&mut header[read..])
            .await
            .map_err(|_| unreadable_local_file())?;
        if count == 0 {
            break;
        }
        read += count;
    }
    let detected = detect_media(kind, &header[..read]).ok_or_else(|| {
        PublicError::invalid_input(
            format!(
                "Local {} has an unsupported file signature",
                media_label(kind)
            ),
            supported_media_action(kind),
        )
    })?;
    file.seek(SeekFrom::Start(0))
        .await
        .map_err(|_| unreadable_local_file())?;

    Ok(OpenedMedia {
        file,
        size,
        file_name: detected.file_name,
        mime_type: detected.mime_type,
    })
}

fn validate_relative_path(raw: &str) -> Result<PathBuf, PublicError> {
    if raw.is_empty()
        || raw.chars().count() > MAX_RELATIVE_PATH_CHARS
        || raw.len() > MAX_RELATIVE_PATH_BYTES
    {
        return Err(PublicError::invalid_input(
            "relative_path must contain 1 through 1,024 characters",
            "Use a short path relative to META_MEDIA_ROOT",
        ));
    }
    let path = Path::new(raw);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(PublicError::invalid_input(
            "relative_path must not be absolute or contain traversal",
            "Use a path such as `campaign/image.jpg` below META_MEDIA_ROOT",
        ));
    }
    Ok(path.to_owned())
}

fn detect_media(kind: LocalMediaKind, header: &[u8]) -> Option<DetectedMedia> {
    match kind {
        LocalMediaKind::Image => detect_image(header),
        LocalMediaKind::Video => detect_video(header),
    }
}

fn detect_image(header: &[u8]) -> Option<DetectedMedia> {
    if header.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some(DetectedMedia {
            file_name: "upload.jpg",
            mime_type: "image/jpeg",
        });
    }
    if header.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(DetectedMedia {
            file_name: "upload.png",
            mime_type: "image/png",
        });
    }
    None
}

fn detect_video(header: &[u8]) -> Option<DetectedMedia> {
    (header.len() >= 12 && &header[4..8] == b"ftyp").then(|| {
        let quicktime = &header[8..12] == b"qt  ";
        DetectedMedia {
            file_name: if quicktime {
                "upload.mov"
            } else {
                "upload.mp4"
            },
            mime_type: if quicktime {
                "video/quicktime"
            } else {
                "video/mp4"
            },
        }
    })
}

fn normalize_remote_video_url(raw: &str) -> Result<String, PublicError> {
    let raw = raw.trim();
    if raw.is_empty() || raw.chars().count() > MAX_SOURCE_URL_CHARS {
        return Err(unsafe_video_url());
    }
    let url = Url::parse(raw).map_err(|_| unsafe_video_url())?;
    let domain = url.domain().ok_or_else(unsafe_video_url)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.port().is_some_and(|port| port != 443)
        || domain.eq_ignore_ascii_case("localhost")
        || domain.ends_with(".localhost")
        || domain.ends_with(".local")
        || domain.ends_with(".internal")
    {
        return Err(unsafe_video_url());
    }
    Ok(url.into())
}

fn uploaded_image_hash(payload: &Value) -> Option<String> {
    if let Some(hash) = payload.get("hash").and_then(Value::as_str) {
        return normalize_image_hash(hash);
    }
    let images = payload.get("images")?.as_object()?;
    if images.len() != 1 {
        return None;
    }
    let (key, image) = images.iter().next()?;
    image
        .get("hash")
        .and_then(Value::as_str)
        .and_then(normalize_image_hash)
        .or_else(|| normalize_image_hash(key))
}

fn uploaded_video_id(payload: &Value) -> Option<String> {
    ["id", "video_id"]
        .iter()
        .find_map(|key| payload.get(*key))
        .and_then(value_numeric_id)
}

fn normalize_account_digits(raw: &str) -> Option<String> {
    ad_account_digits(raw).map(str::to_owned)
}

fn normalize_image_hash(raw: &str) -> Option<String> {
    let hash = raw.trim();
    (!hash.is_empty()
        && hash.len() <= MAX_HASH_CHARS
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')))
    .then(|| hash.to_owned())
}

fn media_label(kind: LocalMediaKind) -> &'static str {
    match kind {
        LocalMediaKind::Image => "image",
        LocalMediaKind::Video => "video",
    }
}

fn larger_media_action(kind: LocalMediaKind) -> String {
    match kind {
        LocalMediaKind::Image => "Choose a nonempty image no larger than 15 MiB".to_owned(),
        LocalMediaKind::Video => {
            "Choose a nonempty video no larger than 512 MiB or use an HTTPS URL".to_owned()
        }
    }
}

fn supported_media_action(kind: LocalMediaKind) -> String {
    match kind {
        LocalMediaKind::Image => "Use a JPEG or PNG image".to_owned(),
        LocalMediaKind::Video => "Use an MP4 or MOV video, or provide an HTTPS URL".to_owned(),
    }
}

fn unreadable_local_file() -> PublicError {
    PublicError::invalid_input(
        "The local media file could not be opened",
        "Check that the relative path names a readable file below META_MEDIA_ROOT",
    )
}

fn unsafe_video_url() -> PublicError {
    PublicError::invalid_input(
        "url must be a credential-free public HTTPS URL",
        "Use a public HTTPS CDN URL without embedded credentials or a fragment",
    )
}

fn remote_video_error(error: GraphError) -> PublicError {
    match error {
        GraphError::NotAuthenticated
        | GraphError::InvalidEndpoint
        | GraphError::InvalidQuery
        | GraphError::ResponseTooLarge { .. }
        | GraphError::InvalidJson => mutation_error_without_blind_retry(
            error,
            "List ad videos before asking Meta to fetch the URL again",
        ),
        GraphError::Api {
            code: Some(102 | 190),
            ..
        } => PublicError::from(error),
        GraphError::Transport { .. } | GraphError::Api { .. } => {
            let mut error = mutation_error_without_blind_retry(
                error,
                "List ad videos, then verify the URL is public and supported before retrying",
            );
            error.code = "VIDEO_UPLOAD_UNCONFIRMED".to_owned();
            error.message = "Meta did not confirm the remote-video upload".to_owned();
            error.retryable = false;
            error.action = Some(
                "List ad videos, then verify the URL is public and supported before retrying"
                    .to_owned(),
            );
            error
        }
    }
}

fn ambiguous_upload_result(message: &str, action: &str) -> PublicError {
    ambiguous_mutation_result(message, action)
}

#[cfg(test)]
mod tests {
    use rmcp::schemars::schema_for;
    use serde_json::json;

    use super::{
        AdImageAssetSource, UploadAdImageAssetInput, build_image_copy_request,
        build_video_url_request, detect_image, detect_video, uploaded_image_hash,
        uploaded_video_id, validate_relative_path, video_metadata_form,
    };

    #[test]
    fn builds_exact_cross_account_image_copy() {
        let request = build_image_copy_request("act_456", "act_123", "abc_123-def").unwrap();
        assert_eq!(request.endpoint, "act_456/adimages");
        assert_eq!(
            request.form,
            [(
                "copy_from".to_owned(),
                r#"{"source_account_id":"123","hash":"abc_123-def"}"#.to_owned()
            )]
        );
    }

    #[test]
    fn builds_bounded_https_video_request() {
        let metadata =
            video_metadata_form(Some(" Promo ".to_owned()), Some("Launch".to_owned()), None)
                .unwrap();
        let request = build_video_url_request(
            "act_42/advideos",
            "https://cdn.example/video.mp4?signature=abc",
            metadata,
        )
        .unwrap();
        assert_eq!(request.endpoint, "act_42/advideos");
        assert_eq!(request.form[0].0, "file_url");
        assert_eq!(request.form[1], ("name".to_owned(), "Promo".to_owned()));
        assert!(
            build_video_url_request(
                "act_42/advideos",
                "https://user:secret@example.com/video.mp4",
                Vec::new(),
            )
            .is_err()
        );
        assert!(
            build_video_url_request("act_42/advideos", "https://127.0.0.1/video.mp4", Vec::new(),)
                .is_err()
        );
    }

    #[test]
    fn remote_video_permission_errors_do_not_echo_signed_urls() {
        let error = super::remote_video_error(crate::error::GraphError::Api {
            status: 403,
            code: Some(10),
            message: "Rejected https://cdn.example/video.mp4?signature=private".to_owned(),
            retryable: false,
        });
        assert_eq!(error.code, "VIDEO_UPLOAD_UNCONFIRMED");
        assert!(!error.message.contains("signature"));
        assert!(!error.retryable);
    }

    #[test]
    fn accepts_only_contained_relative_path_syntax() {
        assert_eq!(
            validate_relative_path("campaign/image.png").unwrap(),
            std::path::PathBuf::from("campaign/image.png")
        );
        assert!(validate_relative_path("../secret.png").is_err());
        assert!(validate_relative_path("/tmp/image.png").is_err());
        assert!(validate_relative_path("./image.png").is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn opens_contained_file_and_rejects_escaping_symlink() {
        use std::{
            fs,
            os::unix::fs::symlink,
            time::{SystemTime, UNIX_EPOCH},
        };

        use super::{LocalMediaKind, MAX_LOCAL_IMAGE_BYTES, open_local_media};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "armavita-meta-media-test-{}-{nonce}",
            std::process::id()
        ));
        let root = base.join("root");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("image.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        fs::write(base.join("outside.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        symlink(base.join("outside.png"), root.join("escape.png")).unwrap();
        let canonical_root = fs::canonicalize(&root).unwrap();

        let opened = open_local_media(
            Some(&canonical_root),
            "image.png",
            LocalMediaKind::Image,
            MAX_LOCAL_IMAGE_BYTES,
        )
        .await
        .unwrap();
        assert_eq!(opened.size, 8);
        assert_eq!(opened.mime_type, "image/png");
        assert!(
            open_local_media(
                Some(&canonical_root),
                "escape.png",
                LocalMediaKind::Image,
                MAX_LOCAL_IMAGE_BYTES,
            )
            .await
            .is_err()
        );

        let fifo = root.join("image.fifo");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            open_local_media(
                Some(&canonical_root),
                "image.fifo",
                LocalMediaKind::Image,
                MAX_LOCAL_IMAGE_BYTES,
            ),
        )
        .await;
        if result.is_err() {
            // Release a regressed blocking open before the runtime shuts down.
            std::thread::spawn(move || fs::OpenOptions::new().write(true).open(fifo))
                .join()
                .unwrap()
                .unwrap();
        }
        assert!(
            matches!(result, Ok(Err(error)) if error.message == "relative_path is not a regular file")
        );

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn recognizes_supported_magic_and_compact_results() {
        assert_eq!(
            detect_image(b"\x89PNG\r\n\x1a\nrest").unwrap().mime_type,
            "image/png"
        );
        assert!(detect_image(b"RIFFxxxxWEBPrest").is_none());
        assert_eq!(
            detect_video(b"\x00\x00\x00\x18ftypqt  rest")
                .unwrap()
                .mime_type,
            "video/quicktime"
        );
        assert_eq!(
            uploaded_image_hash(&json!({"images":{"upload.png":{"hash":"abc123"}}})),
            Some("abc123".to_owned())
        );
        assert_eq!(
            uploaded_video_id(&json!({"id":"987"})),
            Some("987".to_owned())
        );
        assert!(uploaded_image_hash(&json!({"images":{"a":{},"b":{}}})).is_none());
    }

    #[test]
    fn image_input_schema_is_strict_and_bounded() {
        let schema = serde_json::to_value(schema_for!(UploadAdImageAssetInput)).unwrap();
        let serialized = schema.to_string();
        assert!(serialized.contains("additionalProperties"));
        assert!(serialized.contains("maxLength"));

        let parsed = serde_json::from_value::<UploadAdImageAssetInput>(json!({
            "ad_account_id":"1",
            "source":{"kind":"local_file","relative_path":"image.png","extra":true}
        }));
        assert!(parsed.is_err());

        let source = AdImageAssetSource::ExistingAccountImage {
            source_ad_account_id: "2".to_owned(),
            image_hash: "abc".to_owned(),
        };
        assert!(matches!(
            source,
            AdImageAssetSource::ExistingAccountImage { .. }
        ));
    }
}
