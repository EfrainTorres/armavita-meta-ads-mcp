use std::{
    fmt,
    sync::{
        Arc, PoisonError, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use reqwest::{
    Method, StatusCode, Url,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, RETRY_AFTER},
    multipart::{Form, Part},
    redirect::Policy,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::{io::AsyncReadExt, time::Instant};
use tokio_util::io::ReaderStream;

use crate::{
    config::{
        AppSecret, MAX_RESPONSE_BYTES, MAX_RETRIES, MetaConfig, REQUEST_TIMEOUT, TokenOrigin,
    },
    error::{ApiErrorDetails, GraphError, MetaErrorDetails, RateLimitDetails, StartupError},
};

const USER_AGENT: &str = concat!("armavita-meta-ads-mcp/", env!("CARGO_PKG_VERSION"));
const MAX_ENDPOINT_BYTES: usize = 512;
const MAX_QUERY_PAIRS: usize = 64;
const MAX_QUERY_BYTES: usize = 128 * 1024;
const MAX_QUERY_KEY_BYTES: usize = 64;
const TOTAL_GET_TIMEOUT: Duration = Duration::from_secs(90);
const TOTAL_MUTATION_TIMEOUT: Duration = Duration::from_secs(30);
const TOTAL_UPLOAD_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const MAX_MUTATION_PAIRS: usize = 64;
const MAX_MUTATION_BODY_BYTES: usize = 128 * 1024;
const MAX_UPLOAD_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_UPLOAD_FILE_NAME_BYTES: usize = 255;
const MAX_UPLOAD_TEXT_PAIRS: usize = 64;
const MAX_UPLOAD_TEXT_BYTES: usize = 16 * 1024;
const MEDIA_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MEDIA_ACCEPT: &str = "image/jpeg,image/png,image/webp,image/gif";
const REDACTED: &str = "[REDACTED]";
const SENSITIVE_KEYS: [&str; 5] = [
    "access_token",
    "app_secret",
    "appsecret_proof",
    "client_secret",
    "token",
];
/// Longest single pause a GET retry may take. A longer wait demanded by Meta
/// is returned to the caller as `retry_after_seconds` instead of being slept.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(30);
/// Meta's regain estimates can be hours; never promise more than a day.
const MAX_REPORTED_WAIT_SECONDS: u64 = 24 * 60 * 60;
const MAX_USAGE_HEADER_BYTES: usize = 8 * 1024;
const MIN_ACCESS_TOKEN_CHARS: usize = 20;
const MAX_PROVIDER_TOKEN_CHARS: usize = 64;

type TokenDigest = [u8; 32];

#[derive(Clone)]
pub(crate) struct GraphClient {
    client: reqwest::Client,
    media_client: reqwest::Client,
    api_base: String,
    // Page-scoped clones reuse the pool and keep credentials out of results/Debug.
    credential: Credential,
    // Opt-in: sign every request with `appsecret_proof` (HMAC of its token).
    appsecret: Option<AppSecret>,
}

/// Bearer credential used for a request.
///
/// The user credential is shared by every clone. When Meta rejects it (codes
/// 102/190) it is set aside and the next request re-reads `META_ACCESS_TOKEN`
/// and the OAuth token cache, so a refreshed token works without a restart.
/// A Page credential belongs to one scoped clone and never touches the user's.
#[derive(Clone)]
enum Credential {
    User(Arc<UserCredential>),
    Page {
        authorization: HeaderValue,
        appsecret_proof: Option<String>,
        revoked: Arc<AtomicBool>,
    },
}

struct UserCredential {
    state: RwLock<UserCredentialState>,
    source: Arc<dyn CredentialSource>,
    appsecret: Option<AppSecret>,
}

#[derive(Default)]
struct UserCredentialState {
    active: Option<ActiveCredential>,
    /// Digest of the last token Meta rejected; it is never installed again.
    rejected: Option<TokenDigest>,
}

struct ActiveCredential {
    authorization: HeaderValue,
    appsecret_proof: Option<String>,
    digest: TokenDigest,
    origin: Option<TokenOrigin>,
}

/// The credential a single request carried, so a rejection only retires the
/// token that was actually refused.
struct UsedCredential {
    authorization: HeaderValue,
    appsecret_proof: Option<String>,
    user_digest: Option<TokenDigest>,
}

/// HMAC-SHA256 (RFC 2104) over `message`, as lowercase hex. Implemented on the
/// existing `sha2` dependency; checked against RFC 4231 in the tests.
fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> String {
    const BLOCK: usize = 64;
    let mut block = [0_u8; BLOCK];
    if key.len() > BLOCK {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block.map(|byte| byte ^ 0x36));
    inner.update(message);
    let mut outer = Sha256::new();
    outer.update(block.map(|byte| byte ^ 0x5c));
    outer.update(inner.finalize());
    outer
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// Meta's `appsecret_proof`: HMAC-SHA256 of the access token keyed by the app secret.
fn appsecret_proof(secret: Option<&AppSecret>, token: &str) -> Option<String> {
    secret.map(|secret| hmac_sha256_hex(secret.expose().as_bytes(), token.as_bytes()))
}

/// Where a rejected user token is looked up again.
pub(crate) trait CredentialSource: Send + Sync {
    fn environment_token(&self) -> Option<String>;
    fn cached_token(&self) -> Option<String>;
    /// Remove the OAuth cache only while it still holds the rejected token.
    fn discard_cached(&self, rejected: &TokenDigest);
}

/// Production source: the process environment, then the OAuth token cache.
struct EnvironmentAndCache;

impl CredentialSource for EnvironmentAndCache {
    fn environment_token(&self) -> Option<String> {
        std::env::var("META_ACCESS_TOKEN")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| value.chars().count() >= MIN_ACCESS_TOKEN_CHARS)
    }

    fn cached_token(&self) -> Option<String> {
        crate::auth::load_cached_access_token()
    }

    fn discard_cached(&self, rejected: &TokenDigest) {
        if crate::auth::load_cached_access_token()
            .is_some_and(|token| token_digest(&token) == *rejected)
        {
            crate::auth::invalidate_cached_token();
        }
    }
}

/// Unit tests must never read or delete the developer's real token cache.
#[cfg(test)]
struct NoCredentialSource;

#[cfg(test)]
impl CredentialSource for NoCredentialSource {
    fn environment_token(&self) -> Option<String> {
        None
    }

    fn cached_token(&self) -> Option<String> {
        None
    }

    fn discard_cached(&self, _rejected: &TokenDigest) {}
}

fn default_credential_source() -> Arc<dyn CredentialSource> {
    #[cfg(not(test))]
    {
        Arc::new(EnvironmentAndCache)
    }
    #[cfg(test)]
    {
        let _ = EnvironmentAndCache;
        Arc::new(NoCredentialSource)
    }
}

fn token_digest(token: &str) -> TokenDigest {
    Sha256::digest(token.as_bytes()).into()
}

fn bearer_header(token: &str) -> Option<HeaderValue> {
    let mut authorization = HeaderValue::from_str(&format!("Bearer {token}")).ok()?;
    authorization.set_sensitive(true);
    Some(authorization)
}

impl UserCredential {
    fn current(&self) -> Result<UsedCredential, GraphError> {
        if let Some(active) = self
            .state
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .active
            .as_ref()
        {
            return Ok(active.used());
        }

        let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(active) = state.active.as_ref() {
            return Ok(active.used());
        }
        // Same precedence as start-up: the environment first, then the cache.
        // The cache is read only when the environment offers nothing new.
        let appsecret = self.appsecret.as_ref();
        if let Some(used) = self
            .source
            .environment_token()
            .and_then(|token| state.install(&token, TokenOrigin::Environment, appsecret))
        {
            return Ok(used);
        }
        if let Some(used) = self
            .source
            .cached_token()
            .and_then(|token| state.install(&token, TokenOrigin::Cache, appsecret))
        {
            return Ok(used);
        }

        Err(if state.rejected.is_some() {
            GraphError::AuthExpired
        } else {
            GraphError::NotAuthenticated
        })
    }

    fn reject(&self, digest: TokenDigest) {
        let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
        let Some(active) = state
            .active
            .as_ref()
            .filter(|active| active.digest == digest)
        else {
            // A concurrent request already replaced the refused token.
            return;
        };
        let from_cache = active.origin == Some(TokenOrigin::Cache);
        state.active = None;
        state.rejected = Some(digest);
        drop(state);
        tracing::warn!("Meta rejected the access token; the next request re-reads it");
        if from_cache {
            self.source.discard_cached(&digest);
        }
    }
}

impl UserCredentialState {
    fn install(
        &mut self,
        token: &str,
        origin: TokenOrigin,
        appsecret: Option<&AppSecret>,
    ) -> Option<UsedCredential> {
        let digest = token_digest(token);
        if self.rejected == Some(digest) {
            return None;
        }
        let authorization = bearer_header(token)?;
        if self.rejected.is_some() {
            tracing::info!("using a refreshed Meta access token after a rejection");
        }
        let active = ActiveCredential {
            authorization,
            appsecret_proof: appsecret_proof(appsecret, token),
            digest,
            origin: Some(origin),
        };
        let used = active.used();
        self.active = Some(active);
        Some(used)
    }
}

impl ActiveCredential {
    fn used(&self) -> UsedCredential {
        UsedCredential {
            authorization: self.authorization.clone(),
            appsecret_proof: self.appsecret_proof.clone(),
            user_digest: Some(self.digest),
        }
    }
}

pub(crate) struct UploadFile {
    pub field: &'static str,
    pub file: tokio::fs::File,
    pub size: u64,
    pub name: String,
    pub mime: &'static str,
}

impl fmt::Debug for GraphClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (authenticated, page_scoped) = match &self.credential {
            Credential::User(user) => (
                user.state
                    .read()
                    .unwrap_or_else(PoisonError::into_inner)
                    .active
                    .is_some(),
                false,
            ),
            Credential::Page { revoked, .. } => (!revoked.load(Ordering::Relaxed), true),
        };
        formatter
            .debug_struct("GraphClient")
            .field("api_base", &self.api_base)
            .field("authenticated", &authenticated)
            .field("page_scoped", &page_scoped)
            .finish_non_exhaustive()
    }
}

impl GraphClient {
    pub(crate) fn new(config: &MetaConfig) -> Result<Self, StartupError> {
        Self::with_credential_source(config, default_credential_source())
    }

    pub(crate) fn with_credential_source(
        config: &MetaConfig,
        source: Arc<dyn CredentialSource>,
    ) -> Result<Self, StartupError> {
        let mut state = UserCredentialState::default();
        if let Some(token) = &config.access_token {
            let authorization =
                bearer_header(token.expose()).ok_or(StartupError::InvalidAccessToken)?;
            state.active = Some(ActiveCredential {
                authorization,
                appsecret_proof: appsecret_proof(config.appsecret_proof.as_ref(), token.expose()),
                digest: token_digest(token.expose()),
                origin: config.token_origin,
            });
        }

        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(10))
            .timeout(REQUEST_TIMEOUT)
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(2)
            // Reqwest otherwise retries some protocol NACKs internally. Keep
            // all retry decisions here so writes are provably one-shot and
            // GET retries remain visible, bounded, and testable.
            .retry(reqwest::retry::never())
            // Following a 307/308 could send a mutation body twice and could
            // move the bearer header outside the configured Graph origin.
            .redirect(Policy::none())
            .build()?;
        // Graph credentials must never accompany a request to a CDN. Keep one
        // uncredentialed, pooled client for bounded media reads rather than
        // constructing a client for every tool call.
        let media_client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(10))
            .timeout(MEDIA_REQUEST_TIMEOUT)
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(2)
            .redirect(Policy::none())
            .https_only(true)
            .build()?;

        Ok(Self {
            client,
            media_client,
            api_base: config.api_base.trim_end_matches('/').to_owned(),
            credential: Credential::User(Arc::new(UserCredential {
                state: RwLock::new(state),
                source,
                appsecret: config.appsecret_proof.clone(),
            })),
            appsecret: config.appsecret_proof.clone(),
        })
    }

    fn current_credential(&self) -> Result<UsedCredential, GraphError> {
        match &self.credential {
            Credential::User(user) => user.current(),
            Credential::Page {
                authorization,
                appsecret_proof,
                revoked,
            } => {
                if revoked.load(Ordering::Acquire) {
                    Err(GraphError::NotAuthenticated)
                } else {
                    Ok(UsedCredential {
                        authorization: authorization.clone(),
                        appsecret_proof: appsecret_proof.clone(),
                        user_digest: None,
                    })
                }
            }
        }
    }

    fn reject_credential(&self, used: &UsedCredential) {
        match (&self.credential, used.user_digest) {
            (Credential::User(user), Some(digest)) => user.reject(digest),
            (Credential::Page { revoked, .. }, None) => revoked.store(true, Ordering::Release),
            _ => {}
        }
    }

    pub(crate) async fn get_json(
        &self,
        endpoint: &str,
        query: &[(String, String)],
    ) -> Result<Value, GraphError> {
        let credential = self.current_credential()?;
        if !valid_endpoint(endpoint) {
            return Err(GraphError::InvalidEndpoint);
        }
        if !valid_query(query) {
            return Err(GraphError::InvalidQuery);
        }

        let url = format!("{}/{}", self.api_base, endpoint.trim_matches('/'));
        tokio::time::timeout(
            TOTAL_GET_TIMEOUT,
            self.get_json_with_retries(&url, query, false, &credential),
        )
        .await
        .unwrap_or_else(|_| {
            Err(GraphError::Transport {
                message: "Meta request timed out".to_owned(),
            })
        })
    }

    /// Resolve a Page token internally; never expose it through a tool response.
    /// Clones share the HTTP pool, but a revoked Page token cannot invalidate the user's token.
    pub(crate) async fn for_page(&self, page_id: &str) -> Result<Self, GraphError> {
        let page_id = crate::meta_ids::numeric(page_id).ok_or(GraphError::InvalidEndpoint)?;
        let credential = self.ensure_mutation_input(page_id)?;
        let url = format!("{}/{}", self.api_base, page_id);
        let query = [("fields".to_owned(), "access_token".to_owned())];
        let mut payload = tokio::time::timeout(
            TOTAL_GET_TIMEOUT,
            self.get_json_with_retries(&url, &query, true, &credential),
        )
        .await
        .map_err(|_| GraphError::Transport {
            message: "Meta Page authorization timed out".into(),
        })??;
        let token = payload
            .get_mut("access_token")
            .map(Value::take)
            .and_then(|value| match value {
                Value::String(token) => Some(token),
                _ => None,
            })
            .filter(|token| !token.is_empty() && token.len() <= 8192)
            .ok_or(GraphError::PageAccessRequired)?;
        let authorization = bearer_header(&token).ok_or(GraphError::PageAccessRequired)?;
        let mut scoped = self.clone();
        scoped.credential = Credential::Page {
            authorization,
            appsecret_proof: appsecret_proof(self.appsecret.as_ref(), &token),
            revoked: Arc::new(AtomicBool::new(false)),
        };
        Ok(scoped)
    }

    fn request(
        &self,
        method: Method,
        url: impl AsRef<str>,
        credential: &UsedCredential,
    ) -> reqwest::RequestBuilder {
        let request = self
            .client
            .request(method, url.as_ref())
            .header(AUTHORIZATION, credential.authorization.clone());
        match &credential.appsecret_proof {
            Some(proof) => request.query(&[("appsecret_proof", proof)]),
            None => request,
        }
    }

    /// Whether requests are signed with `appsecret_proof`.
    pub(crate) fn signs_appsecret_proof(&self) -> bool {
        self.appsecret.is_some()
    }

    /// `GET /debug_token` for the token this client currently uses. The token is
    /// sent only to Meta and never appears in the result.
    pub(crate) async fn debug_current_token(&self) -> Result<Value, GraphError> {
        let credential = self.current_credential()?;
        let token = credential
            .authorization
            .to_str()
            .ok()
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or(GraphError::NotAuthenticated)?
            .to_owned();
        let url = format!("{}/debug_token", self.api_base);
        let query = [("input_token".to_owned(), token)];
        tokio::time::timeout(
            TOTAL_GET_TIMEOUT,
            self.get_json_with_retries(&url, &query, false, &credential),
        )
        .await
        .unwrap_or_else(|_| {
            Err(GraphError::Transport {
                message: "Meta request timed out".to_owned(),
            })
        })
    }

    /// Submit one bounded, form-encoded Graph mutation.
    ///
    /// Mutations are deliberately never retried: the connection can fail after
    /// Meta has committed a write but before the response reaches this process.
    pub(crate) async fn post_form_json(
        &self,
        endpoint: &str,
        form: &[(String, String)],
    ) -> Result<Value, GraphError> {
        let credential = self.ensure_mutation_input(endpoint)?;
        let body = encode_mutation_params(form)?;
        let url = format!("{}/{}", self.api_base, endpoint.trim_matches('/'));

        tokio::time::timeout(TOTAL_MUTATION_TIMEOUT, async {
            let response = self
                .request(Method::POST, url, &credential)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(body)
                .send()
                .await
                .map_err(|error| public_transport_error(&error))?;
            self.parse_mutation_response(response, &credential).await
        })
        .await
        .unwrap_or_else(|_| {
            Err(GraphError::Transport {
                message: "Meta request timed out".to_owned(),
            })
        })
    }

    /// Submit one bounded Graph DELETE with optional query parameters.
    ///
    /// Like POST, DELETE is one-shot even when Meta reports a transient error.
    pub(crate) async fn delete_json(
        &self,
        endpoint: &str,
        query: &[(String, String)],
    ) -> Result<Value, GraphError> {
        let credential = self.ensure_mutation_input(endpoint)?;
        let encoded_query = encode_mutation_params(query)?;
        let mut url = format!("{}/{}", self.api_base, endpoint.trim_matches('/'));
        if !encoded_query.is_empty() {
            url.push('?');
            url.push_str(&encoded_query);
        }

        tokio::time::timeout(TOTAL_MUTATION_TIMEOUT, async {
            let response = self
                .request(Method::DELETE, url, &credential)
                .send()
                .await
                .map_err(|error| public_transport_error(&error))?;
            self.parse_mutation_response(response, &credential).await
        })
        .await
        .unwrap_or_else(|_| {
            Err(GraphError::Transport {
                message: "Meta request timed out".to_owned(),
            })
        })
    }

    /// Stream one already-validated local media file to a Graph mutation.
    ///
    /// The caller owns filesystem policy and content validation. This boundary
    /// independently caps the stream, restricts Meta's two supported upload
    /// part names, rejects credential fields, and remains strictly one-shot.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn post_multipart_file_json(
        &self,
        endpoint: &str,
        field_name: &'static str,
        file: tokio::fs::File,
        file_size: u64,
        file_name: String,
        mime_type: &'static str,
        text_fields: Vec<(String, String)>,
    ) -> Result<Value, GraphError> {
        self.post_multipart_files_json(
            endpoint,
            vec![UploadFile {
                field: field_name,
                file,
                size: file_size,
                name: file_name,
                mime: mime_type,
            }],
            text_fields,
        )
        .await
    }

    pub(crate) async fn post_multipart_files_json(
        &self,
        endpoint: &str,
        files: Vec<UploadFile>,
        text_fields: Vec<(String, String)>,
    ) -> Result<Value, GraphError> {
        let credential = self.ensure_mutation_input(endpoint)?;
        if files.is_empty() || files.len() > 2 || !valid_upload_text_fields(&text_fields) {
            return Err(GraphError::InvalidQuery);
        }
        let mut form = Form::new();
        let mut seen = std::collections::BTreeSet::new();
        let mut total = 0_u64;
        for upload in files {
            total = total.saturating_add(upload.size);
            let valid_type = match upload.field {
                "upload_gated_file" => {
                    matches!(upload.mime, "application/pdf" | "image/jpeg" | "image/png")
                }
                "cover_photo" => matches!(upload.mime, "image/jpeg" | "image/png"),
                "file" => matches!(
                    upload.mime,
                    "text/csv"
                        | "text/tab-separated-values"
                        | "application/xml"
                        | "application/json"
                ),
                "source_zip" => upload.mime == "application/zip",
                _ => valid_upload_mime_type(upload.mime),
            };
            if !matches!(
                upload.field,
                "filename" | "source" | "cover_photo" | "upload_gated_file" | "file" | "source_zip"
            ) || !seen.insert(upload.field)
                || upload.size == 0
                || total > MAX_UPLOAD_FILE_BYTES
                || !valid_upload_file_name(&upload.name)
                || !valid_type
            {
                return Err(GraphError::InvalidQuery);
            }
            let stream = ReaderStream::new(upload.file.take(upload.size));
            let body = reqwest::Body::wrap_stream(stream);
            let part = Part::stream_with_length(body, upload.size)
                .file_name(upload.name)
                .mime_str(upload.mime)
                .map_err(|_| GraphError::InvalidQuery)?;
            form = form.part(upload.field, part);
        }
        for (key, value) in text_fields {
            form = form.text(key, value);
        }
        let url = format!("{}/{}", self.api_base, endpoint.trim_matches('/'));

        tokio::time::timeout(TOTAL_UPLOAD_TIMEOUT, async {
            let response = self
                .request(Method::POST, url, &credential)
                .multipart(form)
                .timeout(TOTAL_UPLOAD_TIMEOUT)
                .send()
                .await
                .map_err(|error| public_transport_error(&error))?;
            self.parse_mutation_response(response, &credential).await
        })
        .await
        .unwrap_or_else(|_| {
            Err(GraphError::Transport {
                message: "Meta upload timed out".to_owned(),
            })
        })
    }

    /// Send an uncredentialed media GET through the shared CDN pool.
    ///
    /// The caller must validate the HTTPS URL and bound the streamed body.
    /// Errors are deliberately opaque so signed URLs and query values cannot
    /// reach logs or model-visible output.
    pub(crate) async fn get_media_response(
        &self,
        url: Url,
    ) -> Result<reqwest::Response, MediaTransportError> {
        self.media_client
            .get(url)
            .header(ACCEPT, MEDIA_ACCEPT)
            .send()
            .await
            .map_err(|_| MediaTransportError)
    }

    async fn get_json_with_retries(
        &self,
        url: &str,
        query: &[(String, String)],
        preserve_page_token: bool,
        credential: &UsedCredential,
    ) -> Result<Value, GraphError> {
        // The caller wraps this loop in TOTAL_GET_TIMEOUT; never start a pause
        // that the outer timeout would cut short.
        let deadline = Instant::now() + TOTAL_GET_TIMEOUT;
        for attempt in 0..=MAX_RETRIES {
            let response = match self
                .request(Method::GET, url, credential)
                .query(query)
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    if retry_get_transport(&error, attempt).await {
                        continue;
                    }
                    return Err(public_transport_error(&error));
                }
            };

            let status = response.status();
            let parsed = match self
                .parse_graph_response(response, preserve_page_token, credential)
                .await
            {
                Ok(parsed) => parsed,
                Err(ResponseReadError::TooLarge) => {
                    return Err(GraphError::ResponseTooLarge {
                        limit: MAX_RESPONSE_BYTES,
                    });
                }
                Err(ResponseReadError::Transport(error)) => {
                    if retry_get_transport(&error, attempt).await {
                        continue;
                    }
                    return Err(public_transport_error(&error));
                }
            };

            if !parsed.retryable {
                return finish_graph_response(parsed, None);
            }
            let rate_limited = parsed.rate_limit_scope().is_some();
            let delay = parsed
                .retry_hint()
                .unwrap_or_else(|| backoff_delay(attempt, rate_limited, parsed.usage.usage_pct));
            let remaining = deadline.saturating_duration_since(Instant::now());
            if attempt >= MAX_RETRIES || delay > MAX_RETRY_WAIT || delay >= remaining {
                // Report how long Meta needs instead of silently giving up.
                let retry_after = (rate_limited || parsed.retry_hint().is_some()).then_some(delay);
                tracing::warn!(
                    status = status.as_u16(),
                    graph_code = parsed.graph_code,
                    attempt = attempt + 1,
                    delay_ms = delay.as_millis(),
                    "Meta request still throttled after the retry budget"
                );
                return finish_graph_response(parsed, retry_after);
            }
            tracing::warn!(
                status = status.as_u16(),
                graph_code = parsed.graph_code,
                graph_subcode = parsed.graph_subcode,
                usage_pct = parsed.usage.usage_pct,
                attempt = attempt + 1,
                delay_ms = delay.as_millis(),
                "retrying Meta request"
            );
            tokio::time::sleep(delay).await;
        }

        unreachable!("bounded retry loop always returns")
    }

    fn ensure_mutation_input(&self, endpoint: &str) -> Result<UsedCredential, GraphError> {
        let credential = self.current_credential()?;
        if !valid_endpoint(endpoint) {
            return Err(GraphError::InvalidEndpoint);
        }
        Ok(credential)
    }

    async fn parse_mutation_response(
        &self,
        response: reqwest::Response,
        credential: &UsedCredential,
    ) -> Result<Value, GraphError> {
        let parsed = self
            .parse_graph_response(response, false, credential)
            .await
            .map_err(|error| match error {
                ResponseReadError::TooLarge => GraphError::ResponseTooLarge {
                    limit: MAX_RESPONSE_BYTES,
                },
                ResponseReadError::Transport(error) => public_transport_error(&error),
            })?;
        // Writes are never retried, but the caller still learns how long Meta
        // asked it to wait before verifying and resubmitting.
        let retry_after = parsed.retry_hint();
        finish_graph_response(parsed, retry_after)
    }

    async fn parse_graph_response(
        &self,
        response: reqwest::Response,
        preserve_page_token: bool,
        credential: &UsedCredential,
    ) -> Result<ParsedGraphResponse, ResponseReadError> {
        let status = response.status();
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(|seconds| Duration::from_secs(seconds.min(MAX_REPORTED_WAIT_SECONDS)));
        let usage = UsageHeaders::from_headers(response.headers());
        let body = read_bounded(response).await?;
        let mut payload = serde_json::from_slice::<Value>(&body).ok();
        if !preserve_page_token
            || !status.is_success()
            || payload.as_ref().is_some_and(|v| v.get("error").is_some())
        {
            payload.iter_mut().for_each(sanitize_payload);
        }

        let graph_code = payload
            .as_ref()
            .and_then(|value| value.pointer("/error/code"))
            .and_then(serde_json::Value::as_i64);
        let graph_subcode = payload
            .as_ref()
            .and_then(|value| value.pointer("/error/error_subcode"))
            .and_then(serde_json::Value::as_i64);
        // Code 10 denies this operation's permissions; it does not invalidate
        // a token that may still authorize other accounts or endpoints.
        let auth_error = matches!(graph_code, Some(102 | 190));
        if auth_error {
            self.reject_credential(credential);
        }

        Ok(ParsedGraphResponse {
            status,
            payload,
            graph_code,
            graph_subcode,
            retryable: !auth_error && retryable_response(status, graph_code),
            retry_after,
            usage,
        })
    }
}

#[derive(Debug)]
pub(crate) struct MediaTransportError;

struct ParsedGraphResponse {
    status: StatusCode,
    payload: Option<Value>,
    graph_code: Option<i64>,
    graph_subcode: Option<i64>,
    retryable: bool,
    retry_after: Option<Duration>,
    usage: UsageHeaders,
}

impl ParsedGraphResponse {
    fn rate_limit_scope(&self) -> Option<&'static str> {
        if self.status.is_success() {
            return None;
        }
        rate_limit_scope(self.graph_code, self.graph_subcode).or_else(|| {
            (self.status == StatusCode::TOO_MANY_REQUESTS)
                .then(|| self.usage.scope.unwrap_or("http"))
        })
    }

    /// The pause Meta itself asked for, if any: an explicit `Retry-After`, or
    /// for a throttled request the regain estimate from its usage headers.
    fn retry_hint(&self) -> Option<Duration> {
        let throttled = self.rate_limit_scope().is_some();
        let usage_wait = throttled
            .then_some(self.usage.regain_seconds)
            .flatten()
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs);
        match (self.retry_after, usage_wait) {
            (Some(header), Some(usage)) => Some(header.max(usage)),
            (header, usage) => header.or(usage),
        }
    }
}

/// Utilisation reported by Meta's throttling headers.
///
/// * `X-Business-Use-Case-Usage`: per business use case `call_count`,
///   `total_time`, `total_cputime` (percent) and
///   `estimated_time_to_regain_access` (minutes).
/// * `X-Ad-Account-Usage`: `acc_id_util_pct` and `reset_time_duration` (seconds).
/// * `X-FB-Ads-Insights-Throttle`: `app_id_util_pct` and `acc_id_util_pct`.
#[derive(Clone, Debug, Default, PartialEq)]
struct UsageHeaders {
    usage_pct: Option<u32>,
    scope: Option<&'static str>,
    business_use_case: Option<String>,
    regain_seconds: Option<u64>,
    access_tier: Option<String>,
}

impl UsageHeaders {
    fn from_headers(headers: &HeaderMap) -> Self {
        let json = |name: &str| {
            headers
                .get(name)
                .filter(|value| value.len() <= MAX_USAGE_HEADER_BYTES)
                .and_then(|value| serde_json::from_slice::<Value>(value.as_bytes()).ok())
        };
        let mut usage = Self::default();

        if let Some(Value::Object(owners)) = json("x-business-use-case-usage") {
            for entry in owners
                .values()
                .filter_map(Value::as_array)
                .flatten()
                .take(64)
            {
                let pct = ["call_count", "total_time", "total_cputime"]
                    .into_iter()
                    .filter_map(|key| usage_percentage(entry.get(key)))
                    .max();
                let regain = entry
                    .get("estimated_time_to_regain_access")
                    .and_then(Value::as_f64)
                    .filter(|minutes| minutes.is_finite() && *minutes > 0.0)
                    .map(|minutes| ((minutes * 60.0).ceil() as u64).min(MAX_REPORTED_WAIT_SECONDS));
                let use_case = entry
                    .get("type")
                    .and_then(Value::as_str)
                    .and_then(|kind| bounded_identifier(kind, false));
                if usage.record(pct, "business_use_case") || regain > usage.regain_seconds {
                    usage.business_use_case = use_case;
                }
                usage.regain_seconds = usage.regain_seconds.max(regain);
                usage.note_tier(entry.get("ads_api_access_tier"));
            }
        }
        if let Some(account) = json("x-ad-account-usage") {
            usage.record(
                usage_percentage(account.get("acc_id_util_pct")),
                "ad_account",
            );
            let reset = account
                .get("reset_time_duration")
                .and_then(Value::as_f64)
                .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
                .map(|seconds| (seconds.ceil() as u64).min(MAX_REPORTED_WAIT_SECONDS));
            usage.regain_seconds = usage.regain_seconds.max(reset);
            usage.note_tier(account.get("ads_api_access_tier"));
        }
        if let Some(insights) = json("x-fb-ads-insights-throttle") {
            let pct = ["app_id_util_pct", "acc_id_util_pct"]
                .into_iter()
                .filter_map(|key| usage_percentage(insights.get(key)))
                .max();
            usage.record(pct, "ads_insights");
            usage.note_tier(insights.get("ads_api_access_tier"));
        }
        usage
    }

    /// Keep the highest utilisation and remember which header reported it.
    fn record(&mut self, pct: Option<u32>, scope: &'static str) -> bool {
        match pct {
            Some(pct) if self.usage_pct.is_none_or(|current| pct > current) => {
                self.usage_pct = Some(pct);
                self.scope = Some(scope);
                true
            }
            _ => false,
        }
    }

    fn note_tier(&mut self, tier: Option<&Value>) {
        if self.access_tier.is_none() {
            self.access_tier = tier
                .and_then(Value::as_str)
                .and_then(|tier| bounded_identifier(tier, false));
        }
    }
}

fn usage_percentage(value: Option<&Value>) -> Option<u32> {
    value
        .and_then(Value::as_f64)
        .filter(|pct| pct.is_finite() && *pct >= 0.0)
        .map(|pct| pct.ceil().min(1000.0) as u32)
}

/// Meta's throttling codes and subcodes, mapped to the limit they enforce.
///
/// 4 application, 17 user (subcode 2446079: ad account), 32 Page,
/// 613 call-rate (subcodes 1487742, 1487632, 4841018, 5044001),
/// 80000-80014 business use case. 4/1504022 and 4/1504039 are the
/// Ads Insights platform throttles.
fn rate_limit_scope(code: Option<i64>, subcode: Option<i64>) -> Option<&'static str> {
    Some(match (code?, subcode) {
        (4, Some(1504022 | 1504039)) => "ads_insights",
        (4, _) => "application",
        (17, Some(2446079)) => "ad_account",
        (17, _) => "user",
        (32, _) => "page",
        (613, _) => "ad_account",
        (80000..=80014, _) => "business_use_case",
        _ => return None,
    })
}

/// Pause before retrying a GET when Meta gave no explicit wait.
///
/// Throttled requests back off harder, scaled by the reported utilisation, so
/// a busy account is not pushed further over its limit.
fn backoff_delay(attempt: u8, rate_limited: bool, usage_pct: Option<u32>) -> Duration {
    let base = retry_delay(attempt);
    if !rate_limited {
        return base;
    }
    let factor = match usage_pct {
        Some(pct) if pct >= 100 => 4,
        Some(pct) if pct >= 90 => 3,
        _ => 2,
    };
    (base * factor).min(MAX_RETRY_WAIT)
}

fn valid_endpoint(endpoint: &str) -> bool {
    if endpoint.len() > MAX_ENDPOINT_BYTES {
        return false;
    }
    let endpoint = endpoint.trim_matches('/');
    !endpoint.is_empty()
        && endpoint.split('/').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        })
}

fn valid_query(query: &[(String, String)]) -> bool {
    if query.len() > MAX_QUERY_PAIRS {
        return false;
    }

    let mut total_bytes = 0_usize;
    for (key, value) in query {
        if !valid_parameter_name(key) {
            return false;
        }
        let Some(next_total) = total_bytes
            .checked_add(key.len())
            .and_then(|length| length.checked_add(value.len()))
        else {
            return false;
        };
        total_bytes = next_total;
    }

    total_bytes <= MAX_QUERY_BYTES
}

fn encode_mutation_params(params: &[(String, String)]) -> Result<String, GraphError> {
    if params.len() > MAX_MUTATION_PAIRS {
        return Err(GraphError::InvalidQuery);
    }

    // Preflight raw input before encoding so hostile values cannot force an
    // unbounded temporary allocation. The encoded representation is checked
    // independently because percent-encoding may expand a byte threefold.
    let mut raw_bytes = 0_usize;
    for (key, value) in params {
        if !valid_parameter_name(key) {
            return Err(GraphError::InvalidQuery);
        }
        raw_bytes = raw_bytes
            .checked_add(key.len())
            .and_then(|length| length.checked_add(value.len()))
            .ok_or(GraphError::InvalidQuery)?;
        if raw_bytes > MAX_MUTATION_BODY_BYTES {
            return Err(GraphError::InvalidQuery);
        }
    }

    // URL's query serializer implements application/x-www-form-urlencoded.
    // We send this exact checked string as the POST body (or DELETE query), so
    // the size bound applies to the bytes that leave the process.
    let mut encoder = Url::parse("https://form.invalid/").map_err(|_| GraphError::InvalidQuery)?;
    {
        let mut serializer = encoder.query_pairs_mut();
        for (key, value) in params {
            serializer.append_pair(key, value);
        }
    }
    let encoded = encoder.query().unwrap_or_default().to_owned();
    if encoded.len() > MAX_MUTATION_BODY_BYTES {
        return Err(GraphError::InvalidQuery);
    }
    Ok(encoded)
}

fn valid_parameter_name(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_QUERY_KEY_BYTES
        && !sensitive_key(key)
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn valid_upload_file_name(file_name: &str) -> bool {
    !file_name.is_empty()
        && file_name.len() <= MAX_UPLOAD_FILE_NAME_BYTES
        && !matches!(file_name, "." | "..")
        && file_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_upload_mime_type(mime_type: &str) -> bool {
    matches!(
        mime_type,
        "image/jpeg" | "image/png" | "video/mp4" | "video/quicktime"
    )
}

fn valid_upload_text_fields(fields: &[(String, String)]) -> bool {
    if fields.len() > MAX_UPLOAD_TEXT_PAIRS {
        return false;
    }

    let mut total_bytes = 0_usize;
    for (key, value) in fields {
        if !valid_parameter_name(key) {
            return false;
        }
        let Some(next_total) = total_bytes
            .checked_add(key.len())
            .and_then(|length| length.checked_add(value.len()))
        else {
            return false;
        };
        total_bytes = next_total;
        if total_bytes > MAX_UPLOAD_TEXT_BYTES {
            return false;
        }
    }
    true
}

enum ResponseReadError {
    TooLarge,
    Transport(reqwest::Error),
}

async fn read_bounded(mut response: reqwest::Response) -> Result<Vec<u8>, ResponseReadError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(ResponseReadError::TooLarge);
    }

    let initial_capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or(8 * 1024)
        .min(MAX_RESPONSE_BYTES);
    let mut body = Vec::with_capacity(initial_capacity);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(ResponseReadError::Transport)?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(ResponseReadError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn finish_graph_response(
    parsed: ParsedGraphResponse,
    retry_after: Option<Duration>,
) -> Result<Value, GraphError> {
    if parsed.status.is_success()
        && !parsed
            .payload
            .as_ref()
            .is_some_and(|payload| payload.get("error").is_some_and(|error| !error.is_null()))
    {
        let mut payload = parsed.payload.ok_or(GraphError::InvalidJson)?;
        // Meta retains the current page's `after` cursor on terminal pages.
        // Every caller exposes it as next_cursor, so require an actual next page.
        if !payload
            .pointer("/paging/next")
            .and_then(Value::as_str)
            .is_some_and(|next| !next.is_empty())
            && let Some(cursors) = payload
                .pointer_mut("/paging/cursors")
                .and_then(Value::as_object_mut)
        {
            cursors.remove("after");
        }
        return Ok(payload);
    }

    let meta = meta_error_details(&parsed);
    let retry_after_seconds = retry_after.map(duration_to_reported_seconds);
    let details = (meta.is_some() || retry_after_seconds.is_some()).then(|| {
        Box::new(ApiErrorDetails {
            meta,
            retry_after_seconds,
        })
    });
    Err(GraphError::Api {
        status: parsed.status.as_u16(),
        code: parsed.graph_code,
        // Provider prose can echo private input in arbitrary formats. Keep
        // only the structured status/code instead of guessing how to redact it.
        message: "request rejected".to_owned(),
        retryable: parsed.retryable,
        details,
    })
}

fn duration_to_reported_seconds(duration: Duration) -> u64 {
    let seconds = duration.as_secs() + u64::from(duration.subsec_nanos() > 0);
    seconds.clamp(1, MAX_REPORTED_WAIT_SECONDS)
}

/// Copy only the non-prose fields of Meta's error object, plus a static hint.
fn meta_error_details(parsed: &ParsedGraphResponse) -> Option<MetaErrorDetails> {
    let error = parsed
        .payload
        .as_ref()
        .and_then(|payload| payload.get("error"))
        .filter(|error| error.is_object());
    let rate_limit = parsed.rate_limit_scope().map(|scope| RateLimitDetails {
        scope: scope.to_owned(),
        usage_pct: parsed.usage.usage_pct,
        business_use_case: parsed.usage.business_use_case.clone(),
        estimated_time_to_regain_access_seconds: parsed.usage.regain_seconds,
        ads_api_access_tier: parsed.usage.access_tier.clone(),
    });
    let details = MetaErrorDetails {
        code: parsed.graph_code,
        error_subcode: parsed.graph_subcode,
        error_type: error
            .and_then(|error| error.get("type"))
            .and_then(Value::as_str)
            .and_then(|kind| bounded_identifier(kind, false)),
        is_transient: error
            .and_then(|error| error.get("is_transient"))
            .and_then(Value::as_bool),
        fbtrace_id: error
            .and_then(|error| error.get("fbtrace_id"))
            .and_then(Value::as_str)
            .and_then(|trace| bounded_identifier(trace, true)),
        hint: meta_error_hint(parsed.graph_code, parsed.graph_subcode),
        rate_limit,
    };
    (error.is_some() || details.rate_limit.is_some()).then_some(details)
}

/// Server-authored guidance for common Marketing API errors, keyed by Meta's
/// code and subcode. It replaces Meta's prose, which is never forwarded.
fn meta_error_hint(code: Option<i64>, subcode: Option<i64>) -> Option<&'static str> {
    Some(match (code?, subcode) {
        (_, Some(1_885_183)) => {
            "The promoted post was created by an app in development mode; use a post created by a live app or an existing Page post"
        }
        (_, Some(1_885_000..=1_885_999)) => {
            "Meta rejected the ad creative; check the object_story_spec (Page, link, image hash or video ID, call to action) and that this ad account may use the Page or Instagram account"
        }
        (100, Some(33)) => {
            "The object does not exist, the token cannot see it, or it does not support this operation; verify the ID and the token's access to it"
        }
        (100, _) => {
            "Meta rejected a parameter; check field names, enum values, and ID formats against the Marketing API reference for the pinned Graph version"
        }
        (102, _) => "The Meta session is invalid; refresh the access token",
        (190, Some(463)) => "The access token has expired; refresh it or log in again",
        (190, Some(460)) => {
            "The access token was invalidated by a password change or security event; log in again"
        }
        (190, Some(458 | 459 | 464 | 467 | 492)) => {
            "The access token is no longer valid for this app or user; log in again or issue a new system user token"
        }
        (190, _) => "Meta rejected the access token; refresh it or log in again",
        (200..=299, _) => {
            "The token lacks a permission or asset role this call needs (for example ads_management, ads_read, business_management, or a Page task); grant it and retry"
        }
        (10, _) => {
            "The app or token is not allowed to perform this operation; check the app's access level and the user's role on the asset"
        }
        (613, Some(1_487_742)) => "Too many calls from this ad account; wait before retrying",
        (613, Some(1_487_632)) => {
            "Ad set budgets can change only 4 times per hour; space out budget changes"
        }
        (613, Some(5_044_001)) => {
            "The ad account exceeded its mutation rate; wait before sending more changes"
        }
        (613, Some(4_841_018)) => "Too many concurrent requests; send fewer requests in parallel",
        (613, None) => {
            "Meta applied an abuse-prevention limit; check for actions sending excessive requests before retrying"
        }
        (613, _) => "Meta throttled this request; wait before retrying",
        (4 | 17 | 32 | 80_000..=80_014, _) => {
            "Meta throttled this request; wait for retry_after_seconds when present and reduce request volume"
        }
        (2635, _) => {
            "This call uses a deprecated Graph API version; update the server to a supported Marketing API version"
        }
        (1 | 2, _) => "Meta reported a temporary problem; retry later",
        _ => return None,
    })
}

/// Accept short machine identifiers only (error types, access tiers, trace
/// IDs); anything else is dropped rather than rewritten.
fn bounded_identifier(value: &str, trace_alphabet: bool) -> Option<String> {
    (!value.is_empty()
        && value.len() <= MAX_PROVIDER_TOKEN_CHARS
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || byte == b'_'
                || (trace_alphabet && matches!(byte, b'-' | b'+' | b'/' | b'='))
        }))
    .then(|| value.to_owned())
}

async fn retry_get_transport(error: &reqwest::Error, attempt: u8) -> bool {
    if attempt >= MAX_RETRIES || !is_retryable_transport(error) {
        return false;
    }

    let delay = retry_delay(attempt);
    tracing::warn!(
        reason = transport_reason(error),
        attempt = attempt + 1,
        delay_ms = delay.as_millis(),
        "retrying Meta GET after a transport failure"
    );
    tokio::time::sleep(delay).await;
    true
}

fn is_retryable_transport(error: &reqwest::Error) -> bool {
    error.is_timeout()
        || error.is_connect()
        || error.is_request()
        || error.is_body()
        || error.is_decode()
}

fn transport_reason(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_body() || error.is_decode() {
        "body"
    } else {
        "request"
    }
}

fn public_transport_error(error: &reqwest::Error) -> GraphError {
    let message = if error.is_timeout() {
        "Meta request timed out"
    } else {
        "Meta network request failed"
    };
    GraphError::Transport {
        message: message.to_owned(),
    }
}

fn retryable_response(status: StatusCode, graph_code: Option<i64>) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || matches!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR
                | StatusCode::BAD_GATEWAY
                | StatusCode::SERVICE_UNAVAILABLE
                | StatusCode::GATEWAY_TIMEOUT
        )
        || matches!(graph_code, Some(4 | 17 | 32 | 613 | 80000..=80014))
}

fn retry_delay(attempt: u8) -> Duration {
    Duration::from_secs(1_u64 << attempt.min(3))
}

fn sanitize_payload(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(sanitize_payload),
        Value::Object(items) => {
            for (key, item) in items {
                if sensitive_key(key) {
                    *item = Value::String(REDACTED.to_owned());
                } else {
                    sanitize_payload(item);
                }
            }
        }
        Value::String(text) => sanitize_text(text),
        _ => {}
    }
}

fn sensitive_key(key: &str) -> bool {
    SENSITIVE_KEYS
        .iter()
        .any(|candidate| key.eq_ignore_ascii_case(candidate))
}

fn contains_sensitive_assignment(text: &str) -> bool {
    find_sensitive_assignment(text, 0).is_some()
}

fn sanitize_text(text: &mut String) {
    if let Ok(mut url) = Url::parse(text)
        && matches!(url.scheme(), "http" | "https")
    {
        let sensitive_query = url.query_pairs().any(|(key, _)| sensitive_key(&key));
        let private_component =
            !url.username().is_empty() || url.password().is_some() || url.fragment().is_some();
        if sensitive_query || private_component {
            if sensitive_query {
                let retained = url
                    .query_pairs()
                    .filter(|(key, _)| !sensitive_key(key))
                    .map(|(key, value)| (key.into_owned(), value.into_owned()))
                    .collect::<Vec<_>>();
                url.query_pairs_mut().clear().extend_pairs(retained);
            }
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_fragment(None);
            *text = url.into();
        }
        return;
    }

    if contains_sensitive_assignment(text) {
        *text = redact_sensitive_assignments(text);
    }
}

fn redact_sensitive_assignments(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    while cursor < text.len() {
        let Some((start, assignment_len)) = find_sensitive_assignment(text, cursor) else {
            result.push_str(&text[cursor..]);
            break;
        };

        result.push_str(&text[cursor..start + assignment_len]);
        result.push_str(REDACTED);
        let value_start = start + assignment_len;
        let value_end = text[value_start..]
            .find(|character: char| {
                character.is_ascii_whitespace()
                    || matches!(
                        character,
                        '&' | '#' | '"' | '\'' | '<' | '>' | ')' | ']' | '}'
                    )
            })
            .map_or(text.len(), |offset| value_start + offset);
        cursor = value_end;
    }
    result
}

fn find_sensitive_assignment(text: &str, from: usize) -> Option<(usize, usize)> {
    let remaining = text.as_bytes().get(from..)?;
    SENSITIVE_KEYS
        .iter()
        .filter_map(|key| {
            let key = key.as_bytes();
            let assignment_len = key.len() + 1;
            remaining
                .windows(assignment_len)
                .position(|window| {
                    window[key.len()] == b'=' && window[..key.len()].eq_ignore_ascii_case(key)
                })
                .map(|offset| (from + offset, assignment_len))
        })
        .min_by_key(|(start, _)| *start)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    use crate::{config::MetaConfig, error::GraphError};

    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use reqwest::header::{HeaderMap, HeaderValue};

    use crate::config::TokenOrigin;

    use super::{
        CredentialSource, GraphClient, MAX_RETRY_WAIT, TokenDigest, UsageHeaders, backoff_delay,
        meta_error_hint, rate_limit_scope, retry_delay, retryable_response, sanitize_payload,
        token_digest, valid_endpoint, valid_query, valid_upload_file_name, valid_upload_mime_type,
        valid_upload_text_fields,
    };

    async fn respond(socket: &mut TcpStream, status: &str, headers: &str, body: &str) {
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    }

    #[derive(Default)]
    struct TestCredentialSource {
        environment: Mutex<Option<String>>,
        cache: Mutex<Option<String>>,
        discarded: Mutex<Vec<TokenDigest>>,
        cache_reads: AtomicUsize,
    }

    impl CredentialSource for TestCredentialSource {
        fn environment_token(&self) -> Option<String> {
            self.environment.lock().unwrap().clone()
        }

        fn cached_token(&self) -> Option<String> {
            self.cache_reads.fetch_add(1, Ordering::SeqCst);
            self.cache.lock().unwrap().clone()
        }

        fn discard_cached(&self, rejected: &TokenDigest) {
            self.discarded.lock().unwrap().push(*rejected);
        }
    }

    async fn read_http_request(socket: &mut TcpStream) -> Vec<u8> {
        let mut request = Vec::with_capacity(2 * 1024);
        let mut buffer = [0_u8; 1024];
        loop {
            let bytes_read = socket.read(&mut buffer).await.unwrap();
            if bytes_read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..bytes_read]);

            let Some(headers_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
            else {
                continue;
            };
            let body_start = headers_end + 4;
            let headers = String::from_utf8_lossy(&request[..headers_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or_default();
            if request.len() >= body_start + content_length {
                break;
            }
        }
        request
    }

    #[test]
    fn validates_relative_graph_endpoints() {
        assert!(valid_endpoint("me/adaccounts"));
        assert!(valid_endpoint("act_123"));
        assert!(!valid_endpoint("https://evil.example"));
        assert!(!valid_endpoint("../secrets"));
        assert!(!valid_endpoint("me//adaccounts"));
        assert!(!valid_endpoint("."));
        assert!(!valid_endpoint(&"/".repeat(super::MAX_ENDPOINT_BYTES + 1)));
    }

    #[test]
    fn removes_tokens_from_nested_urls() {
        let mut payload = json!({
            "paging": {
                "next": "https://graph.facebook.com/v25.0/me/adaccounts?after=x&access_token=secret"
            }
        });
        sanitize_payload(&mut payload);
        let next = payload.pointer("/paging/next").unwrap().as_str().unwrap();
        assert!(!next.contains("access_token"));
        assert!(next.contains("after=x"));

        let mut malformed = json!({
            "access_token": "secret-one",
            "message": "failed access_token=secret-two after validation",
            "paging": "/v26.0/me?access_token=secret-three&after=y"
        });
        sanitize_payload(&mut malformed);
        let serialized = malformed.to_string();
        assert!(!serialized.contains("secret-one"));
        assert!(!serialized.contains("secret-two"));
        assert!(!serialized.contains("secret-three"));
        assert!(serialized.contains("after=y"));

        let mut urls = json!({
            "encoded": "https://example.test/?%61ccess_%74oken=secret-four&after=z",
            "fragment": "https://example.test/#access_token=secret-five",
            "userinfo": "https://user:secret-six@example.test/",
            "signed": "https://example.test/image?signature=preserve%2fexact+bytes"
        });
        sanitize_payload(&mut urls);
        assert!(!urls.to_string().contains("secret-"));
        assert_eq!(
            urls["signed"],
            "https://example.test/image?signature=preserve%2fexact+bytes"
        );
    }

    #[test]
    fn bounds_queries_and_rejects_query_credentials() {
        assert!(valid_query(&[("fields".to_owned(), "id,name".to_owned())]));
        assert!(!valid_query(&[(
            "access_token".to_owned(),
            "secret".to_owned()
        )]));
        assert!(!valid_query(&[(
            "fields".to_owned(),
            "x".repeat(super::MAX_QUERY_BYTES + 1)
        )]));
    }

    #[test]
    fn bounds_multipart_metadata_and_rejects_credentials() {
        assert!(valid_upload_file_name("ad-image_1.jpg"));
        assert!(!valid_upload_file_name("../ad-image.jpg"));
        assert!(!valid_upload_file_name("ad image.jpg"));
        assert!(valid_upload_mime_type("image/jpeg"));
        assert!(valid_upload_mime_type("video/mp4"));
        assert!(!valid_upload_mime_type("application/octet-stream"));
        assert!(valid_upload_text_fields(&[(
            "name".to_owned(),
            "Summer asset".to_owned()
        )]));
        assert!(!valid_upload_text_fields(&[(
            "access_token".to_owned(),
            "secret".to_owned()
        )]));
        assert!(!valid_upload_text_fields(&[(
            "name".to_owned(),
            "x".repeat(super::MAX_UPLOAD_TEXT_BYTES + 1)
        )]));
    }

    #[tokio::test]
    async fn exposes_next_cursors_only_when_meta_has_a_next_page() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for body in [
                json!({"data":[{"id":"1"}],"paging":{"cursors":{"before":"first","after":"last"}}}),
                json!({"data":[],"paging":{"cursors":{"after":"last"},"next":""}}),
                json!({"data":[],"paging":{"cursors":{"after":"next"},"next":"https://example.test/edge?after=next&access_token=private-token"}}),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let _request = read_http_request(&mut socket).await;
                let body = body.to_string();
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

        let terminal = graph.get_json("42/edge", &[]).await.unwrap();
        assert!(terminal.pointer("/paging/cursors/after").is_none());
        assert_eq!(
            terminal.pointer("/paging/cursors/before"),
            Some(&json!("first"))
        );
        let empty_next = graph.get_json("42/edge", &[]).await.unwrap();
        assert!(empty_next.pointer("/paging/cursors/after").is_none());
        let continuing = graph.get_json("42/edge", &[]).await.unwrap();
        assert_eq!(continuing["data"], json!([]));
        assert_eq!(
            continuing.pointer("/paging/cursors/after"),
            Some(&json!("next"))
        );
        assert_eq!(
            continuing.pointer("/paging/next"),
            Some(&json!("https://example.test/edge?after=next"))
        );
        assert!(!continuing.to_string().contains("private-token"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn retries_get_after_a_transient_connect_failure() {
        let port_probe = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = port_probe.local_addr().unwrap();
        drop(port_probe);

        let server = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let listener = TcpListener::bind(address).await.unwrap();
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let bytes_read = socket.read(&mut request).await.unwrap();
            assert!(String::from_utf8_lossy(&request[..bytes_read]).starts_with("GET /me "));
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}",
                )
                .await
                .unwrap();
        });

        let config = MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        );
        let graph = GraphClient::new(&config).unwrap();
        let payload = graph.get_json("me", &[]).await.unwrap();

        assert_eq!(payload, json!({ "ok": true }));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn retries_get_after_an_interrupted_response_body() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for response in [
                &b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":"[..],
                &b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}"
                    [..],
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0_u8; 1024];
                let bytes_read = socket.read(&mut request).await.unwrap();
                assert!(String::from_utf8_lossy(&request[..bytes_read]).starts_with("GET /me "));
                socket.write_all(response).await.unwrap();
            }
        });

        let config = MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        );
        let graph = GraphClient::new(&config).unwrap();
        let payload = graph.get_json("me", &[]).await.unwrap();

        assert_eq!(payload, json!({ "ok": true }));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn transport_errors_never_echo_urls_or_query_values() {
        let config = MetaConfig::for_test(
            "not a valid absolute URL",
            Some("test-access-token-1234567890"),
        );
        let graph = GraphClient::new(&config).unwrap();
        let error = graph
            .get_json(
                "me",
                &[("fields".to_owned(), "private-query-value".to_owned())],
            )
            .await
            .unwrap_err();

        let GraphError::Transport { message } = error else {
            panic!("expected a transport error");
        };
        assert_eq!(message, "Meta network request failed");
    }

    #[tokio::test]
    async fn page_credentials_stay_private_and_revocation_does_not_invalidate_user() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for (path, bearer, body) in [
                (
                    "/42?fields=access_token",
                    "user-token",
                    r#"{"access_token":"private-page-token"}"#,
                ),
                (
                    "/42",
                    "private-page-token",
                    r#"{"error":{"code":190,"message":"expired"}}"#,
                ),
                (
                    "/me",
                    "user-token",
                    r#"{"id":"7","access_token":"private-user-token"}"#,
                ),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_http_request(&mut socket).await;
                let request = String::from_utf8_lossy(&request);
                assert!(request.starts_with(&format!("GET {path} HTTP/1.1")));
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains(&format!("authorization: bearer {bearer}\r\n"))
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("user-token"),
        ))
        .unwrap();
        let page = graph.for_page("42").await.unwrap();
        assert!(!format!("{page:?}").contains("private-page-token"));
        assert!(page.get_json("42", &[]).await.is_err());
        let result = graph.get_json("me", &[]).await.unwrap();
        assert!(!result.to_string().contains("private-user-token"));
        assert_eq!(result["id"], "7");
        server.await.unwrap();
    }

    #[test]
    fn hmac_sha256_matches_rfc_4231() {
        // Test case 1.
        assert_eq!(
            super::hmac_sha256_hex(&[0x0b; 20], b"Hi There"),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        // Test case 2.
        assert_eq!(
            super::hmac_sha256_hex(b"Jefe", b"what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Test case 6: a key longer than the block size.
        assert_eq!(
            super::hmac_sha256_hex(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            ),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[tokio::test]
    async fn signs_user_and_page_requests_with_appsecret_proof_only_when_enabled() {
        let secret = "0123456789abcdef0123456789abcdef";
        let user_proof = super::hmac_sha256_hex(secret.as_bytes(), b"user-token-1234567890");
        let page_proof = super::hmac_sha256_hex(secret.as_bytes(), b"private-page-token");
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let expected = [
            format!("GET /me?appsecret_proof={user_proof} HTTP/1.1"),
            format!("GET /42?appsecret_proof={user_proof}&fields=access_token HTTP/1.1"),
            format!("POST /42/feed?appsecret_proof={page_proof} HTTP/1.1"),
            "GET /me HTTP/1.1".to_owned(),
        ];
        let server = tokio::spawn(async move {
            for (line, body) in expected.iter().zip([
                r#"{"id":"7"}"#,
                r#"{"access_token":"private-page-token"}"#,
                r#"{"id":"9"}"#,
                r#"{"id":"7"}"#,
            ]) {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = String::from_utf8(read_http_request(&mut socket).await).unwrap();
                assert!(request.starts_with(line), "{request}");
                respond(&mut socket, "200 OK", "", body).await;
            }
        });
        let signed = GraphClient::new(
            &MetaConfig::for_test(format!("http://{address}"), Some("user-token-1234567890"))
                .with_appsecret_proof(secret),
        )
        .unwrap();
        assert!(signed.signs_appsecret_proof());
        signed.get_json("me", &[]).await.unwrap();
        let page = signed.for_page("42").await.unwrap();
        page.post_form_json("42/feed", &[]).await.unwrap();

        let plain = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("user-token-1234567890"),
        ))
        .unwrap();
        assert!(!plain.signs_appsecret_proof());
        plain.get_json("me", &[]).await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn debug_token_sends_the_token_only_to_meta() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = String::from_utf8(read_http_request(&mut socket).await).unwrap();
            assert!(
                request.starts_with("GET /debug_token?input_token=user-token-1234567890 HTTP/1.1")
            );
            respond(
                &mut socket,
                "200 OK",
                "",
                r#"{"data":{"app_id":"1","type":"SYSTEM_USER","is_valid":true,"scopes":["ads_read"]}}"#,
            )
            .await;
        });
        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("user-token-1234567890"),
        ))
        .unwrap();
        let payload = graph.debug_current_token().await.unwrap();
        assert_eq!(payload["data"]["type"], "SYSTEM_USER");
        assert!(!payload.to_string().contains("user-token"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn posts_exact_bounded_form_encoding() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut socket).await;
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"id\":\"42\"}",
                )
                .await
                .unwrap();
            request
        });

        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        ))
        .unwrap();
        let payload = graph
            .post_form_json(
                "act_123/campaigns",
                &[
                    ("name".to_owned(), "Summer Sale".to_owned()),
                    ("special_ad_categories".to_owned(), "[]".to_owned()),
                ],
            )
            .await
            .unwrap();

        assert_eq!(payload, json!({ "id": "42" }));
        let request = String::from_utf8(server.await.unwrap()).unwrap();
        assert!(request.starts_with("POST /act_123/campaigns HTTP/1.1\r\n"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("content-type: application/x-www-form-urlencoded")
        );
        assert!(
            request.ends_with("name=Summer+Sale&special_ad_categories=%5B%5D"),
            "unexpected encoded body: {request:?}"
        );
    }

    #[tokio::test]
    async fn streams_one_bounded_multipart_upload() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut socket).await;
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 16\r\nConnection: close\r\n\r\n{\"hash\":\"asset\"}",
                )
                .await
                .unwrap();
            request
        });

        let file_path = std::env::temp_dir().join(format!(
            "armavita-meta-multipart-{}-{}.jpg",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let file_bytes = b"bounded-media-bytes";
        tokio::fs::write(&file_path, file_bytes).await.unwrap();
        let file = tokio::fs::File::open(&file_path).await.unwrap();
        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        ))
        .unwrap();
        let payload = graph
            .post_multipart_file_json(
                "act_123/adimages",
                "filename",
                file,
                file_bytes.len() as u64,
                "ad-image.jpg".to_owned(),
                "image/jpeg",
                Vec::new(),
            )
            .await
            .unwrap();
        tokio::fs::remove_file(file_path).await.unwrap();

        assert_eq!(payload, json!({ "hash": "asset" }));
        let request = server.await.unwrap();
        let rendered = String::from_utf8_lossy(&request);
        assert!(rendered.starts_with("POST /act_123/adimages HTTP/1.1\r\n"));
        assert!(
            rendered
                .to_ascii_lowercase()
                .contains("content-type: multipart/form-data; boundary=")
        );
        assert!(rendered.contains("name=\"filename\""));
        assert!(rendered.contains("filename=\"ad-image.jpg\""));
        assert!(rendered.contains("Content-Type: image/jpeg"));
        assert!(
            request
                .windows(file_bytes.len())
                .any(|part| part == file_bytes)
        );
    }

    #[tokio::test]
    async fn sends_delete_parameters_in_the_query_without_a_body() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut socket).await;
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 16\r\nConnection: close\r\n\r\n{\"success\":true}",
                )
                .await
                .unwrap();
            request
        });

        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        ))
        .unwrap();
        let payload = graph
            .delete_json("campaign_42", &[("archive".to_owned(), "true".to_owned())])
            .await
            .unwrap();

        assert_eq!(payload, json!({ "success": true }));
        let request = String::from_utf8(server.await.unwrap()).unwrap();
        assert!(request.starts_with("DELETE /campaign_42?archive=true HTTP/1.1\r\n"));
        assert!(request.ends_with("\r\n\r\n"));
    }

    #[tokio::test]
    async fn rejects_oversized_invalid_or_credential_mutation_parameters() {
        let graph = GraphClient::new(&MetaConfig::for_test(
            "http://127.0.0.1:1",
            Some("test-access-token-1234567890"),
        ))
        .unwrap();

        for form in [
            vec![("access_token".to_owned(), "secret".to_owned())],
            vec![("nested[value]".to_owned(), "x".to_owned())],
            vec![(
                "name".to_owned(),
                "x".repeat(super::MAX_MUTATION_BODY_BYTES + 1),
            )],
            vec![(
                "name".to_owned(),
                "\0".repeat(super::MAX_MUTATION_BODY_BYTES / 2),
            )],
        ] {
            assert!(matches!(
                graph.post_form_json("me", &form).await,
                Err(GraphError::InvalidQuery)
            ));
        }
    }

    #[tokio::test]
    async fn provider_error_prose_never_reaches_internal_or_public_errors() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _request = read_http_request(&mut socket).await;
            let response = json!({"error": {
                "code": 100,
                "error_subcode": 1_487_390,
                "type": "OAuthException",
                "message": "Rejected private-form-value; https://example.test/#access_token=response-secret; {\"token\":\"nested-secret\"}; person@example.test",
                "error_user_title": "Name private-form-value rejected",
                "error_user_msg": "Contact person@example.test about private-form-value",
                "is_transient": false,
                "fbtrace_id": "AbCdEf123_-+/="
            }}).to_string();
            let headers = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            );
            socket.write_all(headers.as_bytes()).await.unwrap();
            socket.write_all(response.as_bytes()).await.unwrap();
        });

        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        ))
        .unwrap();
        let error = graph
            .post_form_json(
                "me",
                &[("name".to_owned(), "private-form-value".to_owned())],
            )
            .await
            .unwrap_err();
        let rendered = format!("{error:?} {error}");

        assert!(!rendered.contains("private-form-value"));
        assert!(!rendered.contains("response-secret"));
        assert!(!rendered.contains("nested-secret"));
        assert!(!rendered.contains("person@example.test"));
        let public = crate::error::PublicError::from(error);
        assert_eq!(
            public.message,
            "Meta API error 100 (HTTP 400): request rejected"
        );
        assert_eq!(public.code, "META_API_ERROR");
        let meta = public.details.meta.as_ref().unwrap();
        assert_eq!(meta.code, Some(100));
        assert_eq!(meta.error_subcode, Some(1_487_390));
        assert_eq!(meta.error_type.as_deref(), Some("OAuthException"));
        assert_eq!(meta.fbtrace_id.as_deref(), Some("AbCdEf123_-+/="));
        assert_eq!(meta.is_transient, Some(false));
        assert!(meta.hint.unwrap().starts_with("Meta rejected a parameter"));
        assert!(public.details.retry_after_seconds.is_none());
        let serialized = serde_json::to_string(&public).unwrap();
        for private in [
            "private-form-value",
            "response-secret",
            "nested-secret",
            "person@example.test",
        ] {
            assert!(!serialized.contains(private), "{private} leaked");
        }
        server.await.unwrap();
    }

    #[test]
    fn static_hints_cover_common_marketing_api_errors() {
        let hint = |code, subcode| meta_error_hint(Some(code), subcode).unwrap();
        assert!(hint(100, Some(33)).contains("does not exist"));
        assert!(hint(100, Some(1_885_183)).contains("development mode"));
        assert!(hint(100, Some(1_885_650)).contains("ad creative"));
        assert!(hint(190, Some(463)).contains("expired"));
        assert!(hint(190, None).contains("access token"));
        assert!(hint(200, None).contains("permission"));
        assert!(hint(10, None).contains("not allowed"));
        assert!(hint(613, Some(1_487_742)).contains("ad account"));
        assert!(hint(613, Some(1_487_632)).contains("4 times per hour"));
        assert!(hint(613, Some(5_044_001)).contains("mutation rate"));
        assert!(hint(613, Some(4_841_018)).contains("concurrent"));
        assert!(hint(613, None).contains("abuse-prevention"));
        assert!(hint(80_004, None).contains("throttled"));
        assert!(hint(2635, None).contains("deprecated"));
        assert_eq!(meta_error_hint(Some(1_234_567), None), None);
        assert_eq!(meta_error_hint(None, Some(33)), None);
    }

    #[test]
    fn parses_meta_usage_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-business-use-case-usage",
            HeaderValue::from_static(
                r#"{"123":[{"type":"ads_management","call_count":40,"total_cputime":12,"total_time":20,"estimated_time_to_regain_access":0},{"type":"ads_insights","call_count":101,"total_cputime":30,"total_time":40,"estimated_time_to_regain_access":7,"ads_api_access_tier":"standard_access"}]}"#,
            ),
        );
        headers.insert(
            "x-ad-account-usage",
            HeaderValue::from_static(
                r#"{"acc_id_util_pct":64.2,"reset_time_duration":30,"ads_api_access_tier":"standard_access"}"#,
            ),
        );
        headers.insert(
            "x-fb-ads-insights-throttle",
            HeaderValue::from_static(r#"{"app_id_util_pct":12.5,"acc_id_util_pct":99}"#),
        );
        assert_eq!(
            UsageHeaders::from_headers(&headers),
            UsageHeaders {
                usage_pct: Some(101),
                scope: Some("business_use_case"),
                business_use_case: Some("ads_insights".to_owned()),
                regain_seconds: Some(420),
                access_tier: Some("standard_access".to_owned()),
            }
        );

        let mut account_only = HeaderMap::new();
        account_only.insert(
            "x-ad-account-usage",
            HeaderValue::from_static(r#"{"acc_id_util_pct":100,"reset_time_duration":95.5}"#),
        );
        account_only.insert(
            "x-fb-ads-insights-throttle",
            HeaderValue::from_static("not json"),
        );
        let usage = UsageHeaders::from_headers(&account_only);
        assert_eq!(usage.usage_pct, Some(100));
        assert_eq!(usage.scope, Some("ad_account"));
        assert_eq!(usage.regain_seconds, Some(96));
        assert_eq!(
            UsageHeaders::from_headers(&HeaderMap::new()),
            UsageHeaders::default()
        );
    }

    #[test]
    fn recognises_meta_rate_limit_codes() {
        for (code, subcode, scope) in [
            (613, Some(5_044_001), "ad_account"),
            (613, Some(4_841_018), "ad_account"),
            (613, Some(1_487_632), "ad_account"),
            (613, None, "ad_account"),
            (17, Some(2_446_079), "ad_account"),
            (17, None, "user"),
            (4, Some(1_504_022), "ads_insights"),
            (4, None, "application"),
            (32, None, "page"),
            (80000, None, "business_use_case"),
            (80014, Some(2_446_079), "business_use_case"),
        ] {
            assert_eq!(rate_limit_scope(Some(code), subcode), Some(scope), "{code}");
            assert!(retryable_response(
                reqwest::StatusCode::BAD_REQUEST,
                Some(code)
            ));
        }
        for code in [Some(100), Some(190), Some(79_999), Some(80_015), None] {
            assert_eq!(rate_limit_scope(code, None), None);
        }
        assert!(!retryable_response(
            reqwest::StatusCode::BAD_REQUEST,
            Some(80_015)
        ));
    }

    #[test]
    fn throttled_backoff_scales_with_reported_usage() {
        assert_eq!(backoff_delay(0, false, Some(100)).as_secs(), 1);
        assert_eq!(backoff_delay(0, true, None).as_secs(), 2);
        assert_eq!(backoff_delay(1, true, Some(92)).as_secs(), 6);
        assert_eq!(backoff_delay(2, true, Some(100)).as_secs(), 16);
        assert_eq!(backoff_delay(3, true, Some(100)), MAX_RETRY_WAIT);
    }

    #[tokio::test]
    async fn retries_a_throttled_get_after_metas_retry_after() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _request = read_http_request(&mut socket).await;
            respond(
                &mut socket,
                "400 Bad Request",
                "Retry-After: 1\r\nX-Ad-Account-Usage: {\"acc_id_util_pct\":97}\r\n",
                r#"{"error":{"code":613,"error_subcode":1487632,"message":"Calls to this api have exceeded the rate limit."}}"#,
            )
            .await;
            let (mut socket, _) = listener.accept().await.unwrap();
            let _request = read_http_request(&mut socket).await;
            respond(&mut socket, "200 OK", "", r#"{"data":[]}"#).await;
        });
        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        ))
        .unwrap();

        let started = tokio::time::Instant::now();
        let payload = graph.get_json("act_1/campaigns", &[]).await.unwrap();
        assert_eq!(payload, json!({"data": []}));
        assert!(started.elapsed() >= std::time::Duration::from_secs(1));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn returns_retry_after_when_metas_wait_exceeds_the_retry_budget() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _request = read_http_request(&mut socket).await;
            respond(
                &mut socket,
                "400 Bad Request",
                "X-Business-Use-Case-Usage: {\"42\":[{\"type\":\"ads_insights\",\"call_count\":100,\"total_cputime\":35,\"total_time\":61,\"estimated_time_to_regain_access\":5,\"ads_api_access_tier\":\"development_access\"}]}\r\n",
                r#"{"error":{"code":80000,"error_subcode":2446079,"message":"There have been too many calls from this ad-account.","type":"OAuthException","fbtrace_id":"Trace01"}}"#,
            )
            .await;
            tokio::time::timeout(std::time::Duration::from_millis(300), listener.accept())
                .await
                .is_ok()
        });
        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        ))
        .unwrap();

        let started = tokio::time::Instant::now();
        let error = graph.get_json("act_42/insights", &[]).await.unwrap_err();
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        assert!(!server.await.unwrap(), "GET retried despite a long wait");
        let public = crate::error::PublicError::from(error);
        assert_eq!(public.code, "RATE_LIMITED");
        assert!(public.retryable);
        assert_eq!(public.details.retry_after_seconds, Some(300));
        assert!(public.action.as_deref().unwrap().contains("300 seconds"));
        let meta = public.details.meta.as_ref().unwrap();
        assert_eq!(meta.code, Some(80000));
        assert_eq!(meta.error_subcode, Some(2_446_079));
        assert_eq!(meta.fbtrace_id.as_deref(), Some("Trace01"));
        let rate_limit = meta.rate_limit.as_ref().unwrap();
        assert_eq!(rate_limit.scope, "business_use_case");
        assert_eq!(rate_limit.usage_pct, Some(100));
        assert_eq!(
            rate_limit.business_use_case.as_deref(),
            Some("ads_insights")
        );
        assert_eq!(
            rate_limit.estimated_time_to_regain_access_seconds,
            Some(300)
        );
        assert_eq!(
            rate_limit.ads_api_access_tier.as_deref(),
            Some("development_access")
        );
    }

    #[tokio::test]
    async fn throttled_write_reports_the_wait_without_retrying() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _request = read_http_request(&mut socket).await;
            respond(
                &mut socket,
                "400 Bad Request",
                "X-Ad-Account-Usage: {\"acc_id_util_pct\":100,\"reset_time_duration\":120}\r\n",
                r#"{"error":{"code":17,"error_subcode":2446079,"message":"User request limit reached"}}"#,
            )
            .await;
            tokio::time::timeout(std::time::Duration::from_millis(300), listener.accept())
                .await
                .is_ok()
        });
        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        ))
        .unwrap();

        let error = graph
            .post_form_json("act_1/campaigns", &[("name".to_owned(), "x".to_owned())])
            .await
            .unwrap_err();
        assert!(!server.await.unwrap(), "mutation was sent more than once");
        let public = crate::mutation_result::mutation_error_without_blind_retry(
            error,
            "Verify the campaign list before retrying",
        );
        assert_eq!(public.code, "RATE_LIMITED");
        assert!(!public.retryable);
        assert_eq!(public.details.retry_after_seconds, Some(120));
        let rate_limit = public.details.meta.unwrap().rate_limit.unwrap();
        assert_eq!(rate_limit.scope, "ad_account");
        assert_eq!(rate_limit.usage_pct, Some(100));
    }

    #[tokio::test]
    async fn rejected_token_is_replaced_by_a_refreshed_one_without_restart() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for (bearer, status, body) in [
                (
                    "old-token-1234567890abcdef",
                    "401 Unauthorized",
                    r#"{"error":{"code":190,"error_subcode":463,"type":"OAuthException","message":"Error validating access token: Session has expired"}}"#,
                ),
                ("new-token-1234567890abcdef", "200 OK", r#"{"id":"7"}"#),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_http_request(&mut socket).await;
                let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
                assert!(request.contains(&format!("authorization: bearer {bearer}\r\n")));
                respond(&mut socket, status, "", body).await;
            }
            tokio::time::timeout(std::time::Duration::from_millis(200), listener.accept())
                .await
                .is_ok()
        });
        let source = Arc::new(TestCredentialSource::default());
        // A running process keeps seeing the environment it started with.
        *source.environment.lock().unwrap() = Some("old-token-1234567890abcdef".to_owned());
        let graph = GraphClient::with_credential_source(
            &MetaConfig::for_test(
                format!("http://{address}"),
                Some("old-token-1234567890abcdef"),
            ),
            source.clone(),
        )
        .unwrap();

        let expired = crate::error::PublicError::from(graph.get_json("me", &[]).await.unwrap_err());
        assert_eq!(expired.code, "AUTH_EXPIRED");
        assert_eq!(
            expired
                .details
                .meta
                .as_ref()
                .and_then(|meta| meta.error_subcode),
            Some(463)
        );

        // Nothing new to load: fail clearly without calling Meta again.
        let still_expired = graph.post_form_json("me", &[]).await.unwrap_err();
        assert!(matches!(still_expired, GraphError::AuthExpired));
        let public = crate::error::PublicError::from(still_expired);
        assert_eq!(public.code, "AUTH_EXPIRED");
        assert!(public.action.unwrap().contains("re-reads"));
        assert!(source.discarded.lock().unwrap().is_empty());

        // `--login` in another terminal refreshes the cache; no restart needed.
        *source.cache.lock().unwrap() = Some("new-token-1234567890abcdef".to_owned());
        let reads_before = source.cache_reads.load(Ordering::SeqCst);
        assert_eq!(graph.get_json("me", &[]).await.unwrap()["id"], "7");
        assert_eq!(source.cache_reads.load(Ordering::SeqCst), reads_before + 1);
        assert!(!server.await.unwrap(), "an expired token was sent again");
        assert!(format!("{graph:?}").contains("authenticated: true"));
        assert!(!format!("{graph:?}").contains("new-token"));
    }

    #[tokio::test]
    async fn rejecting_a_cached_token_discards_only_that_cache_entry() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _request = read_http_request(&mut socket).await;
            respond(
                &mut socket,
                "400 Bad Request",
                "",
                r#"{"error":{"code":102,"message":"Session key invalid"}}"#,
            )
            .await;
        });
        let source = Arc::new(TestCredentialSource::default());
        let mut config = MetaConfig::for_test(
            format!("http://{address}"),
            Some("cached-token-1234567890abcd"),
        );
        config.token_origin = Some(TokenOrigin::Cache);
        let graph = GraphClient::with_credential_source(&config, source.clone()).unwrap();

        assert!(graph.get_json("me", &[]).await.is_err());
        assert_eq!(
            *source.discarded.lock().unwrap(),
            vec![token_digest("cached-token-1234567890abcd")]
        );
        assert!(matches!(
            graph.get_json("me", &[]).await,
            Err(GraphError::AuthExpired)
        ));
        server.await.unwrap();
    }

    #[test]
    fn a_stale_rejection_never_retires_a_newer_token() {
        let source = Arc::new(TestCredentialSource::default());
        let graph = GraphClient::with_credential_source(
            &MetaConfig::for_test("http://127.0.0.1:9", Some("current-token-1234567890ab")),
            source,
        )
        .unwrap();
        let super::Credential::User(user) = &graph.credential else {
            panic!("expected a user credential");
        };
        user.reject(token_digest("previous-token-1234567890ab"));
        assert!(graph.current_credential().is_ok());
        user.reject(token_digest("current-token-1234567890ab"));
        assert!(matches!(
            graph.current_credential(),
            Err(GraphError::AuthExpired)
        ));
    }

    #[test]
    fn unconfigured_clients_pick_up_a_token_written_later() {
        let source = Arc::new(TestCredentialSource::default());
        let graph = GraphClient::with_credential_source(
            &MetaConfig::for_test("http://127.0.0.1:9", None),
            source.clone(),
        )
        .unwrap();
        assert!(matches!(
            graph.current_credential(),
            Err(GraphError::NotAuthenticated)
        ));
        *source.cache.lock().unwrap() = Some("later-token-1234567890abcd".to_owned());
        assert!(graph.current_credential().is_ok());
    }

    #[tokio::test]
    async fn permission_denial_preserves_authentication_for_other_operations() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for (status, body) in [
                (
                    "403 Forbidden",
                    r#"{"error":{"code":10,"message":"Permission denied"}}"#,
                ),
                ("200 OK", r#"{"data":[]}"#),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let _request = read_http_request(&mut socket).await;
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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

        let error = graph.post_form_json("123", &[]).await.unwrap_err();
        let public = crate::error::PublicError::from(error);
        assert_eq!(public.code, "META_API_ERROR");
        assert!(!public.retryable);
        assert!(public.action.is_none());
        assert_eq!(
            graph.get_json("me/adaccounts", &[]).await.unwrap(),
            json!({"data": []})
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn never_retries_a_mutation_after_a_transient_response() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _request = read_http_request(&mut socket).await;
            let response = b"{\"error\":{\"code\":613,\"message\":\"rate limited\"}}";
            let headers = format!(
                "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            );
            socket.write_all(headers.as_bytes()).await.unwrap();
            socket.write_all(response).await.unwrap();

            tokio::time::timeout(std::time::Duration::from_millis(1_200), listener.accept())
                .await
                .is_ok()
        });

        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        ))
        .unwrap();
        let error = graph
            .post_form_json("me", &[("name".to_owned(), "one-shot".to_owned())])
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            GraphError::Api {
                retryable: true,
                ..
            }
        ));
        assert!(!server.await.unwrap(), "mutation was sent more than once");
    }

    #[tokio::test]
    async fn never_follows_a_mutation_redirect() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _request = read_http_request(&mut socket).await;
            let response = format!(
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{address}/moved\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            socket.write_all(response.as_bytes()).await.unwrap();

            tokio::time::timeout(std::time::Duration::from_millis(300), listener.accept())
                .await
                .is_ok()
        });

        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        ))
        .unwrap();
        let error = graph
            .post_form_json("me", &[("name".to_owned(), "one-shot".to_owned())])
            .await
            .unwrap_err();

        assert!(matches!(error, GraphError::Api { status: 307, .. }));
        assert!(!server.await.unwrap(), "mutation redirect was followed");
    }

    #[test]
    fn retry_policy_is_small_and_capped() {
        assert_eq!(retry_delay(0).as_secs(), 1);
        assert_eq!(retry_delay(3).as_secs(), 8);
        assert_eq!(retry_delay(9).as_secs(), 8);
        assert_eq!(super::TOTAL_GET_TIMEOUT.as_secs(), 90);
        assert!(retryable_response(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            None
        ));
        assert!(retryable_response(
            reqwest::StatusCode::BAD_REQUEST,
            Some(613)
        ));
        assert!(retryable_response(
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
            None
        ));
        assert!(!retryable_response(
            reqwest::StatusCode::NOT_IMPLEMENTED,
            None
        ));
    }
}
