use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use reqwest::{
    StatusCode, Url,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, RETRY_AFTER},
    multipart::{Form, Part},
    redirect::Policy,
};
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio_util::io::ReaderStream;

use crate::{
    auth::invalidate_cached_token,
    config::{MAX_RESPONSE_BYTES, MAX_RETRIES, MetaConfig, REQUEST_TIMEOUT, TokenOrigin},
    error::{GraphError, StartupError},
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
const MAX_UPLOAD_TEXT_PAIRS: usize = 16;
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

#[derive(Clone)]
pub(crate) struct GraphClient {
    client: reqwest::Client,
    media_client: reqwest::Client,
    api_base: String,
    authenticated: Arc<AtomicBool>,
    cache_backed: bool,
}

impl fmt::Debug for GraphClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GraphClient")
            .field("api_base", &self.api_base)
            .field("authenticated", &self.authenticated.load(Ordering::Relaxed))
            .field("cache_backed", &self.cache_backed)
            .finish_non_exhaustive()
    }
}

impl GraphClient {
    pub(crate) fn new(config: &MetaConfig) -> Result<Self, StartupError> {
        let mut headers = HeaderMap::new();
        if let Some(token) = &config.access_token {
            let mut authorization = HeaderValue::from_str(&format!("Bearer {}", token.expose()))
                .map_err(|_| StartupError::InvalidAccessToken)?;
            authorization.set_sensitive(true);
            headers.insert(AUTHORIZATION, authorization);
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
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
            authenticated: Arc::new(AtomicBool::new(config.access_token.is_some())),
            cache_backed: config.token_origin == Some(TokenOrigin::Cache),
        })
    }

    pub(crate) async fn get_json(
        &self,
        endpoint: &str,
        query: &[(String, String)],
    ) -> Result<Value, GraphError> {
        if !self.authenticated.load(Ordering::Acquire) {
            return Err(GraphError::NotAuthenticated);
        }
        if !valid_endpoint(endpoint) {
            return Err(GraphError::InvalidEndpoint);
        }
        if !valid_query(query) {
            return Err(GraphError::InvalidQuery);
        }

        let url = format!("{}/{}", self.api_base, endpoint.trim_matches('/'));
        tokio::time::timeout(TOTAL_GET_TIMEOUT, self.get_json_with_retries(&url, query))
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
        self.ensure_mutation_input(endpoint)?;
        let body = encode_mutation_params(form)?;
        let url = format!("{}/{}", self.api_base, endpoint.trim_matches('/'));

        tokio::time::timeout(TOTAL_MUTATION_TIMEOUT, async {
            let response = self
                .client
                .post(url)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(body)
                .send()
                .await
                .map_err(|error| public_transport_error(&error))?;
            self.parse_mutation_response(response).await
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
        self.ensure_mutation_input(endpoint)?;
        let encoded_query = encode_mutation_params(query)?;
        let mut url = format!("{}/{}", self.api_base, endpoint.trim_matches('/'));
        if !encoded_query.is_empty() {
            url.push('?');
            url.push_str(&encoded_query);
        }

        tokio::time::timeout(TOTAL_MUTATION_TIMEOUT, async {
            let response = self
                .client
                .delete(url)
                .send()
                .await
                .map_err(|error| public_transport_error(&error))?;
            self.parse_mutation_response(response).await
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
        self.ensure_mutation_input(endpoint)?;
        if !matches!(field_name, "filename" | "source")
            || file_size == 0
            || file_size > MAX_UPLOAD_FILE_BYTES
            || !valid_upload_file_name(&file_name)
            || !valid_upload_mime_type(mime_type)
            || !valid_upload_text_fields(&text_fields)
        {
            return Err(GraphError::InvalidQuery);
        }

        let stream = ReaderStream::new(file.take(file_size));
        let body = reqwest::Body::wrap_stream(stream);
        let part = Part::stream_with_length(body, file_size)
            .file_name(file_name)
            .mime_str(mime_type)
            .map_err(|_| GraphError::InvalidQuery)?;
        let mut form = Form::new().part(field_name, part);
        for (key, value) in text_fields {
            form = form.text(key, value);
        }
        let url = format!("{}/{}", self.api_base, endpoint.trim_matches('/'));

        tokio::time::timeout(TOTAL_UPLOAD_TIMEOUT, async {
            let response = self
                .client
                .post(url)
                .multipart(form)
                .timeout(TOTAL_UPLOAD_TIMEOUT)
                .send()
                .await
                .map_err(|error| public_transport_error(&error))?;
            self.parse_mutation_response(response).await
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
    ) -> Result<Value, GraphError> {
        for attempt in 0..=MAX_RETRIES {
            let response = match self.client.get(url).query(query).send().await {
                Ok(response) => response,
                Err(error) => {
                    if retry_get_transport(&error, attempt).await {
                        continue;
                    }
                    return Err(public_transport_error(&error));
                }
            };

            let status = response.status();
            let retry_after = response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .map(|seconds| Duration::from_secs(seconds.min(30)));
            let parsed = match self.parse_graph_response(response).await {
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

            if parsed.retryable && attempt < MAX_RETRIES {
                let delay = retry_after.unwrap_or_else(|| retry_delay(attempt));
                tracing::warn!(
                    status = status.as_u16(),
                    graph_code = parsed.graph_code,
                    attempt = attempt + 1,
                    delay_ms = delay.as_millis(),
                    "retrying Meta request"
                );
                tokio::time::sleep(delay).await;
                continue;
            }

            return finish_graph_response(parsed);
        }

        unreachable!("bounded retry loop always returns")
    }

    fn ensure_mutation_input(&self, endpoint: &str) -> Result<(), GraphError> {
        if !self.authenticated.load(Ordering::Acquire) {
            return Err(GraphError::NotAuthenticated);
        }
        if !valid_endpoint(endpoint) {
            return Err(GraphError::InvalidEndpoint);
        }
        Ok(())
    }

    async fn parse_mutation_response(
        &self,
        response: reqwest::Response,
    ) -> Result<Value, GraphError> {
        let parsed = self
            .parse_graph_response(response)
            .await
            .map_err(|error| match error {
                ResponseReadError::TooLarge => GraphError::ResponseTooLarge {
                    limit: MAX_RESPONSE_BYTES,
                },
                ResponseReadError::Transport(error) => public_transport_error(&error),
            })?;
        finish_graph_response(parsed)
    }

    async fn parse_graph_response(
        &self,
        response: reqwest::Response,
    ) -> Result<ParsedGraphResponse, ResponseReadError> {
        let status = response.status();
        let body = read_bounded(response).await?;
        let mut payload = serde_json::from_slice::<Value>(&body).ok();
        payload.iter_mut().for_each(sanitize_payload);

        let graph_code = payload
            .as_ref()
            .and_then(|value| value.pointer("/error/code"))
            .and_then(serde_json::Value::as_i64);
        // Code 10 denies this operation's permissions; it does not invalidate
        // a token that may still authorize other accounts or endpoints.
        let auth_error = matches!(graph_code, Some(102 | 190));
        if auth_error {
            self.authenticated.store(false, Ordering::Release);
            if self.cache_backed {
                invalidate_cached_token();
            }
        }

        Ok(ParsedGraphResponse {
            status,
            payload,
            graph_code,
            retryable: !auth_error && retryable_response(status, graph_code),
        })
    }
}

#[derive(Debug)]
pub(crate) struct MediaTransportError;

struct ParsedGraphResponse {
    status: StatusCode,
    payload: Option<Value>,
    graph_code: Option<i64>,
    retryable: bool,
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

fn finish_graph_response(parsed: ParsedGraphResponse) -> Result<Value, GraphError> {
    if parsed.status.is_success() {
        return parsed.payload.ok_or(GraphError::InvalidJson);
    }

    Err(GraphError::Api {
        status: parsed.status.as_u16(),
        code: parsed.graph_code,
        // Provider prose can echo private input in arbitrary formats. Keep
        // only the structured status/code instead of guessing how to redact it.
        message: "request rejected".to_owned(),
        retryable: parsed.retryable,
    })
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
        || matches!(graph_code, Some(4 | 17 | 32 | 613))
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

    use super::{
        GraphClient, retry_delay, retryable_response, sanitize_payload, valid_endpoint,
        valid_query, valid_upload_file_name, valid_upload_mime_type, valid_upload_text_fields,
    };

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
            let response = json!({"error": {"code": 100, "message":
                "Rejected private-form-value; https://example.test/#access_token=response-secret; {\"token\":\"nested-secret\"}; person@example.test"
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
        server.await.unwrap();
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
