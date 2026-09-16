use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::{Instant, timeout},
};

use crate::config::DEFAULT_GRAPH_API_VERSION;

const OAUTH_SCOPE: &str = "ads_management,ads_read,business_management,pages_show_list,pages_read_engagement,instagram_basic,threads_business_basic";
const CALLBACK_PORT_START: u16 = 8_080;
const CALLBACK_PORT_ATTEMPTS: u16 = 10;
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);
const CALLBACK_READ_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CALLBACK_BYTES: usize = 8 * 1024;
const MAX_CALLBACK_CONNECTIONS: usize = 16;
const MAX_TOKEN_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_CACHE_BYTES: u64 = 32 * 1024;
const MIN_TOKEN_CHARS: usize = 20;
const TOKEN_EXPIRY_SKEW: u64 = 60;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("META_APP_ID must be a numeric Meta app ID")]
    InvalidAppId,
    #[error("META_APP_SECRET is required for OAuth login")]
    MissingAppSecret,
    #[error("OAuth callback server is disabled by META_ADS_DISABLE_CALLBACK_SERVER")]
    CallbackDisabled,
    #[error("no callback port is available in 8080-8089")]
    CallbackPortUnavailable,
    #[error("OAuth login timed out")]
    CallbackTimeout,
    #[error("OAuth callback was invalid")]
    InvalidCallback,
    #[error("OAuth state verification failed")]
    StateMismatch,
    #[error("Meta authorization was denied: {0}")]
    AuthorizationDenied(String),
    #[error("secure random state generation failed")]
    RandomState,
    #[error("Meta token exchange failed")]
    TokenExchange,
    #[error("local OAuth I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("OAuth network request failed")]
    Http,
}

#[derive(Debug)]
pub struct LoginOutcome {
    pub used_long_lived_token: bool,
    pub meta_user_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct TokenInfo {
    #[serde(alias = "access_token")]
    meta_access_token: String,
    expires_in: Option<u64>,
    #[serde(default, alias = "user_id")]
    meta_user_id: Option<String>,
    created_at: u64,
}

impl TokenInfo {
    fn is_valid_at(&self, now: u64) -> bool {
        if self.meta_access_token.chars().count() < MIN_TOKEN_CHARS {
            return false;
        }
        self.expires_in.is_none_or(|lifetime| {
            now.saturating_add(TOKEN_EXPIRY_SKEW) < self.created_at.saturating_add(lifetime)
        })
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    #[serde(alias = "meta_access_token")]
    access_token: String,
    expires_in: Option<u64>,
    #[serde(default, alias = "meta_user_id")]
    user_id: Option<String>,
}

struct PendingCallback {
    stream: TcpStream,
    code: String,
}

struct CallbackValues {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

pub async fn login(app_id_override: Option<&str>) -> Result<LoginOutcome, AuthError> {
    if callback_disabled() {
        return Err(AuthError::CallbackDisabled);
    }

    let app_id = app_id_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| env::var("META_APP_ID").ok())
        .ok_or(AuthError::InvalidAppId)?;
    if !valid_app_id(&app_id) {
        return Err(AuthError::InvalidAppId);
    }
    let app_secret = env::var("META_APP_SECRET")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or(AuthError::MissingAppSecret)?;

    let api_version = DEFAULT_GRAPH_API_VERSION;
    let (listener, port) = bind_callback().await?;
    let redirect_uri = format!("http://localhost:{port}/callback");
    let state = generate_state()?;
    let authorization_url = build_authorization_url(api_version, &app_id, &redirect_uri, &state)?;

    println!(
        "Open this Meta authorization URL if your browser does not open:\n{authorization_url}"
    );
    if let Err(error) = open_browser(&authorization_url) {
        tracing::warn!(%error, "could not open the browser automatically");
    }

    let mut callback = wait_for_callback(listener, &state).await?;
    let client = reqwest::Client::builder()
        .user_agent(concat!("armavita-meta-ads-mcp/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| AuthError::Http)?;

    let short = match exchange_code(
        &client,
        api_version,
        &app_id,
        &app_secret,
        &redirect_uri,
        &callback.code,
    )
    .await
    {
        Ok(token) => token,
        Err(error) => {
            let _ = send_html(
                &mut callback.stream,
                StatusCode::BAD_GATEWAY,
                "Authorization failed",
                "Meta did not accept the authorization code. Return to the terminal and retry.",
            )
            .await;
            return Err(error);
        }
    };

    let long = exchange_long_lived(
        &client,
        api_version,
        &app_id,
        &app_secret,
        &short.access_token,
    )
    .await;
    let (token, used_long_lived_token) = match long {
        Ok(mut token) => {
            if token.user_id.is_none() {
                token.user_id.clone_from(&short.user_id);
            }
            (token, true)
        }
        Err(error) => {
            tracing::warn!(%error, "long-lived exchange failed; caching the short-lived token");
            (short, false)
        }
    };

    let token_info = TokenInfo {
        meta_access_token: token.access_token,
        expires_in: token.expires_in,
        meta_user_id: token.user_id,
        created_at: unix_timestamp(),
    };
    persist_token(&token_info)?;
    send_html(
        &mut callback.stream,
        StatusCode::OK,
        "Authorization complete",
        "The token is stored locally with private permissions. You can close this window.",
    )
    .await?;

    Ok(LoginOutcome {
        used_long_lived_token,
        meta_user_id: token_info.meta_user_id,
    })
}

pub(crate) fn load_cached_access_token() -> Option<String> {
    let path = cache_path(true).ok()?;
    let metadata = fs::symlink_metadata(&path).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return None;
    }
    tighten_file_permissions(&path).ok()?;
    if metadata.len() > MAX_CACHE_BYTES {
        let _ = fs::remove_file(path);
        return None;
    }

    let payload = fs::read(&path).ok()?;
    let token = serde_json::from_slice::<TokenInfo>(&payload).ok();
    match token {
        Some(token) if token.is_valid_at(unix_timestamp()) => Some(token.meta_access_token),
        _ => {
            let _ = fs::remove_file(path);
            None
        }
    }
}

pub(crate) fn invalidate_cached_token() {
    if let Ok(path) = cache_path(false) {
        let _ = fs::remove_file(path);
    }
}

async fn bind_callback() -> Result<(TcpListener, u16), AuthError> {
    for port in CALLBACK_PORT_START..CALLBACK_PORT_START + CALLBACK_PORT_ATTEMPTS {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)).await {
            return Ok((listener, port));
        }
    }
    Err(AuthError::CallbackPortUnavailable)
}

async fn wait_for_callback(
    listener: TcpListener,
    expected_state: &str,
) -> Result<PendingCallback, AuthError> {
    let deadline = Instant::now() + CALLBACK_TIMEOUT;
    for _ in 0..MAX_CALLBACK_CONNECTIONS {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(AuthError::CallbackTimeout);
        }
        let (mut stream, _) = timeout(remaining, listener.accept())
            .await
            .map_err(|_| AuthError::CallbackTimeout)??;

        let request_target = match read_request_target(&mut stream).await {
            Ok(target) => target,
            Err(_) => {
                let _ = send_html(
                    &mut stream,
                    StatusCode::BAD_REQUEST,
                    "Invalid request",
                    "This local listener accepts only the Meta OAuth callback.",
                )
                .await;
                continue;
            }
        };
        let values = match parse_callback_target(&request_target) {
            Ok(values) => values,
            Err(_) => {
                let _ = send_html(
                    &mut stream,
                    StatusCode::NOT_FOUND,
                    "Not found",
                    "This local listener exposes no token or account data.",
                )
                .await;
                continue;
            }
        };

        if let Some(error) = values.error {
            let detail = values.error_description.unwrap_or(error);
            let detail = sanitize_message(&detail);
            send_html(
                &mut stream,
                StatusCode::BAD_REQUEST,
                "Authorization denied",
                &detail,
            )
            .await?;
            return Err(AuthError::AuthorizationDenied(detail));
        }
        if !constant_time_eq(
            values.state.as_deref().unwrap_or_default().as_bytes(),
            expected_state.as_bytes(),
        ) {
            send_html(
                &mut stream,
                StatusCode::BAD_REQUEST,
                "Authorization failed",
                "State verification failed. Return to the terminal and retry login.",
            )
            .await?;
            return Err(AuthError::StateMismatch);
        }
        let code = values
            .code
            .filter(|value| !value.is_empty() && value.len() <= 4_096)
            .ok_or(AuthError::InvalidCallback)?;
        return Ok(PendingCallback { stream, code });
    }

    Err(AuthError::InvalidCallback)
}

async fn read_request_target(stream: &mut TcpStream) -> Result<String, AuthError> {
    let mut request = Vec::with_capacity(1_024);
    let mut buffer = [0_u8; 1_024];
    loop {
        let read = timeout(CALLBACK_READ_TIMEOUT, stream.read(&mut buffer))
            .await
            .map_err(|_| AuthError::InvalidCallback)??;
        if read == 0 || request.len().saturating_add(read) > MAX_CALLBACK_BYTES {
            return Err(AuthError::InvalidCallback);
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }

    let request = std::str::from_utf8(&request).map_err(|_| AuthError::InvalidCallback)?;
    let mut parts = request
        .lines()
        .next()
        .ok_or(AuthError::InvalidCallback)?
        .split_ascii_whitespace();
    if parts.next() != Some("GET") {
        return Err(AuthError::InvalidCallback);
    }
    let target = parts.next().ok_or(AuthError::InvalidCallback)?;
    if !matches!(parts.next(), Some("HTTP/1.0" | "HTTP/1.1"))
        || parts.next().is_some()
        || target.len() > MAX_CALLBACK_BYTES
    {
        return Err(AuthError::InvalidCallback);
    }
    Ok(target.to_owned())
}

fn parse_callback_target(target: &str) -> Result<CallbackValues, AuthError> {
    if !target.starts_with('/') {
        return Err(AuthError::InvalidCallback);
    }
    let url =
        Url::parse(&format!("http://localhost{target}")).map_err(|_| AuthError::InvalidCallback)?;
    if url.path() != "/callback" {
        return Err(AuthError::InvalidCallback);
    }

    let mut values = CallbackValues {
        code: None,
        state: None,
        error: None,
        error_description: None,
    };
    for (key, value) in url.query_pairs() {
        let slot = match key.as_ref() {
            "code" => &mut values.code,
            "state" => &mut values.state,
            "error" => &mut values.error,
            "error_description" => &mut values.error_description,
            _ => continue,
        };
        if slot.replace(value.into_owned()).is_some() {
            return Err(AuthError::InvalidCallback);
        }
    }
    Ok(values)
}

fn build_authorization_url(
    api_version: &str,
    app_id: &str,
    redirect_uri: &str,
    state: &str,
) -> Result<String, AuthError> {
    let mut url = Url::parse(&format!(
        "https://www.facebook.com/{api_version}/dialog/oauth"
    ))
    .map_err(|_| AuthError::InvalidCallback)?;
    let config_id = env::var("META_LOGIN_CONFIG_ID")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let scope = env::var("META_AUTH_SCOPE").unwrap_or_else(|_| OAUTH_SCOPE.to_owned());
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("client_id", app_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("state", state);
        if let Some(config_id) = config_id {
            query.append_pair("config_id", &config_id);
        } else {
            query.append_pair("scope", &scope);
        }
    }
    Ok(url.into())
}

async fn exchange_code(
    client: &reqwest::Client,
    api_version: &str,
    app_id: &str,
    app_secret: &str,
    redirect_uri: &str,
    code: &str,
) -> Result<TokenResponse, AuthError> {
    let url = format!("https://graph.facebook.com/{api_version}/oauth/access_token");
    let response = client
        .get(url)
        .query(&[
            ("client_id", app_id),
            ("redirect_uri", redirect_uri),
            ("client_secret", app_secret),
            ("code", code),
        ])
        .send()
        .await
        .map_err(|_| AuthError::Http)?;
    read_token_response(response).await
}

async fn exchange_long_lived(
    client: &reqwest::Client,
    api_version: &str,
    app_id: &str,
    app_secret: &str,
    short_lived_token: &str,
) -> Result<TokenResponse, AuthError> {
    let url = format!("https://graph.facebook.com/{api_version}/oauth/access_token");
    let response = client
        .get(url)
        .query(&[
            ("grant_type", "fb_exchange_token"),
            ("client_id", app_id),
            ("client_secret", app_secret),
            ("fb_exchange_token", short_lived_token),
        ])
        .send()
        .await
        .map_err(|_| AuthError::Http)?;
    read_token_response(response).await
}

async fn read_token_response<T: DeserializeOwned>(
    mut response: reqwest::Response,
) -> Result<T, AuthError> {
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > MAX_TOKEN_RESPONSE_BYTES as u64)
    {
        return Err(AuthError::TokenExchange);
    }

    let mut body = Vec::with_capacity(
        response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or(1_024)
            .min(MAX_TOKEN_RESPONSE_BYTES),
    );
    while let Some(chunk) = response.chunk().await.map_err(|_| AuthError::Http)? {
        if body.len().saturating_add(chunk.len()) > MAX_TOKEN_RESPONSE_BYTES {
            return Err(AuthError::TokenExchange);
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| AuthError::TokenExchange)
}

async fn send_html(
    stream: &mut TcpStream,
    status: StatusCode,
    title: &str,
    message: &str,
) -> Result<(), AuthError> {
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>{}</title><h1>{}</h1><p>{}</p>",
        html_escape(title),
        html_escape(title),
        html_escape(message)
    );
    let reason = status.canonical_reason().unwrap_or("Response");
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{}",
        status.as_u16(),
        reason,
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

fn persist_token(token: &TokenInfo) -> Result<(), AuthError> {
    let path = cache_path(true)?;
    let parent = path.parent().ok_or_else(|| {
        AuthError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "token cache has no parent directory",
        ))
    })?;
    let mut random = [0_u8; 8];
    getrandom::fill(&mut random).map_err(|_| AuthError::RandomState)?;
    let suffix = hex(&random);
    let temporary = parent.join(format!(".token_cache-{suffix}.tmp"));

    let write_result = (|| -> Result<(), AuthError> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        serde_json::to_writer(&mut file, token).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        replace_file(&temporary, &path)?;
        tighten_file_permissions(&path)?;
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

fn cache_path(create: bool) -> io::Result<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Application Support"))
    } else {
        env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".config"))
            })
    }
    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no user config directory"))?;
    let directory = base.join("armavita-meta-ads-mcp");
    if create {
        fs::create_dir_all(&directory)?;
        if fs::symlink_metadata(&directory)?.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "token cache directory cannot be a symlink",
            ));
        }
        tighten_directory_permissions(&directory)?;
    }
    Ok(directory.join("token_cache.json"))
}

#[cfg(unix)]
fn tighten_directory_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn tighten_directory_permissions(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn tighten_file_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn tighten_file_permissions(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(target_os = "windows")]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(source, destination)
}

fn generate_state() -> Result<String, AuthError> {
    let mut bytes = [0_u8; 24];
    getrandom::fill(&mut bytes).map_err(|_| AuthError::RandomState)?;
    Ok(hex(&bytes))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(char::from(DIGITS[usize::from(byte >> 4)]));
        value.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    value
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len() && bool::from(left.ct_eq(right))
}

fn valid_app_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn callback_disabled() -> bool {
    env::var("META_ADS_DISABLE_CALLBACK_SERVER")
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
}

fn sanitize_message(value: &str) -> String {
    let mut value = value
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect::<String>();
    if value.is_empty() {
        value.push_str("authorization was denied");
    }
    value
}

fn html_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn open_browser(url: &str) -> io::Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg(url);
        command
    } else if cfg!(target_os = "windows") {
        let mut command = Command::new("rundll32");
        command.args(["url.dll,FileProtocolHandler", url]);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };
    command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        OAUTH_SCOPE, TokenInfo, constant_time_eq, html_escape, parse_callback_target,
        sanitize_message, valid_app_id,
    };

    #[test]
    fn callback_parser_decodes_once_and_rejects_duplicates() {
        let callback = parse_callback_target("/callback?code=a%2Bb&state=expected").unwrap();
        assert_eq!(callback.code.as_deref(), Some("a+b"));
        assert_eq!(callback.state.as_deref(), Some("expected"));
        assert!(parse_callback_target("/token").is_err());
        assert!(parse_callback_target("/callback?state=a&state=b").is_err());
    }

    #[test]
    fn state_check_fails_closed() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"diff"));
        assert!(!constant_time_eq(b"", b"state"));
    }

    #[test]
    fn cached_tokens_expire_with_a_small_clock_skew() {
        let token = TokenInfo {
            meta_access_token: "x".repeat(24),
            expires_in: Some(100),
            meta_user_id: None,
            created_at: 1_000,
        };
        assert!(token.is_valid_at(1_039));
        assert!(!token.is_valid_at(1_040));
    }

    #[test]
    fn oauth_defaults_are_least_privilege_and_output_is_escaped() {
        assert!(!OAUTH_SCOPE.contains("public_profile"));
        assert!(!OAUTH_SCOPE.contains("instagram_branded_content_ads_brand"));
        assert!(valid_app_id("123456"));
        assert!(!valid_app_id("123/456"));
        assert_eq!(html_escape("<script>&"), "&lt;script&gt;&amp;");
        assert_eq!(sanitize_message("\0denied\n"), "denied");
    }
}
