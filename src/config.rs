use std::{env, ffi::OsString, fmt, path::PathBuf, time::Duration};

use thiserror::Error;

use crate::auth::load_cached_access_token;

pub(crate) const DEFAULT_GRAPH_API_VERSION: &str = "v26.0";
pub(crate) const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_RETRIES: u8 = 3;
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const MEDIA_ROOT_ENV: &str = "META_MEDIA_ROOT";
pub(crate) const APPSECRET_PROOF_ENV: &str = "META_APPSECRET_PROOF";
pub(crate) const APP_SECRET_ENV: &str = "META_APP_SECRET";
const MIN_APP_SECRET_CHARS: usize = 16;

#[derive(Clone)]
pub(crate) struct AccessToken(String);

impl AccessToken {
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AccessToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AccessToken([REDACTED])")
    }
}

/// App secret used only to sign requests with `appsecret_proof`.
#[derive(Clone)]
pub(crate) struct AppSecret(String);

impl AppSecret {
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AppSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AppSecret([REDACTED])")
    }
}

#[derive(Clone, Debug)]
pub struct MetaConfig {
    pub(crate) api_base: String,
    pub(crate) access_token: Option<AccessToken>,
    pub(crate) token_origin: Option<TokenOrigin>,
    pub(crate) media_root: Option<PathBuf>,
    /// When set, every Graph request carries `appsecret_proof` (opt-in).
    pub(crate) appsecret_proof: Option<AppSecret>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TokenOrigin {
    Environment,
    Cache,
}

impl MetaConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let environment_token = env::var("META_ACCESS_TOKEN")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        if environment_token
            .as_ref()
            .is_some_and(|token| token.chars().count() < 20)
        {
            return Err(ConfigError::AccessToken);
        }
        let (access_token, token_origin) = match environment_token {
            Some(token) => (Some(AccessToken(token)), Some(TokenOrigin::Environment)),
            None => match load_cached_access_token() {
                Some(token) => (Some(AccessToken(token)), Some(TokenOrigin::Cache)),
                None => (None, None),
            },
        };
        let media_root = normalize_media_root(env::var_os(MEDIA_ROOT_ENV))?;
        let appsecret_proof = appsecret_proof_secret(
            env::var(APPSECRET_PROOF_ENV).ok(),
            env::var(APP_SECRET_ENV).ok(),
        )?;

        Ok(Self {
            api_base: format!("https://graph.facebook.com/{DEFAULT_GRAPH_API_VERSION}"),
            access_token,
            token_origin,
            media_root,
            appsecret_proof,
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(api_base: impl Into<String>, access_token: Option<&str>) -> Self {
        Self {
            api_base: api_base.into(),
            access_token: access_token.map(|value| AccessToken(value.to_owned())),
            token_origin: access_token.map(|_| TokenOrigin::Environment),
            media_root: None,
            appsecret_proof: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_appsecret_proof(mut self, secret: &str) -> Self {
        self.appsecret_proof = Some(AppSecret(secret.to_owned()));
        self
    }
}

fn normalize_media_root(raw: Option<OsString>) -> Result<Option<PathBuf>, ConfigError> {
    let Some(raw) = raw.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        return Err(ConfigError::MediaRoot);
    }
    let canonical = std::fs::canonicalize(path).map_err(|_| ConfigError::MediaRoot)?;
    if !canonical.is_dir() {
        return Err(ConfigError::MediaRoot);
    }
    Ok(Some(canonical))
}

/// Read the opt-in `appsecret_proof` secret. Returns `None` unless
/// `META_APPSECRET_PROOF` is truthy; then `META_APP_SECRET` is required.
fn appsecret_proof_secret(
    flag: Option<String>,
    secret: Option<String>,
) -> Result<Option<AppSecret>, ConfigError> {
    let enabled = flag.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    });
    if !enabled {
        return Ok(None);
    }
    let secret = secret
        .map(|value| value.trim().to_owned())
        .filter(|value| value.len() >= MIN_APP_SECRET_CHARS && value.is_ascii())
        .ok_or(ConfigError::AppSecretProof)?;
    Ok(Some(AppSecret(secret)))
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("META_ACCESS_TOKEN appears malformed")]
    AccessToken,
    #[error("META_MEDIA_ROOT must name an existing absolute directory")]
    MediaRoot,
    #[error("META_APPSECRET_PROOF is on but META_APP_SECRET is missing or malformed")]
    AppSecretProof,
}

#[cfg(test)]
mod tests {
    use super::{
        ConfigError, DEFAULT_GRAPH_API_VERSION, appsecret_proof_secret, normalize_media_root,
    };

    #[test]
    fn appsecret_proof_is_opt_in_and_needs_a_secret() {
        let secret = Some("0123456789abcdef0123456789abcdef".to_owned());
        assert!(
            appsecret_proof_secret(None, secret.clone())
                .unwrap()
                .is_none()
        );
        assert!(
            appsecret_proof_secret(Some("0".into()), secret.clone())
                .unwrap()
                .is_none()
        );
        let enabled = appsecret_proof_secret(Some(" TRUE ".into()), secret).unwrap();
        assert_eq!(
            enabled.as_ref().map(super::AppSecret::expose),
            Some("0123456789abcdef0123456789abcdef")
        );
        assert!(!format!("{enabled:?}").contains("0123"));
        assert!(matches!(
            appsecret_proof_secret(Some("1".into()), None),
            Err(ConfigError::AppSecretProof)
        ));
        assert!(matches!(
            appsecret_proof_secret(Some("1".into()), Some("short".into())),
            Err(ConfigError::AppSecretProof)
        ));
    }

    #[test]
    fn graph_version_is_pinned_to_the_audited_contract() {
        assert_eq!(DEFAULT_GRAPH_API_VERSION, "v26.0");
    }

    #[test]
    fn canonicalizes_only_absolute_media_directories() {
        let expected = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        assert_eq!(
            normalize_media_root(Some(std::env::temp_dir().into_os_string())).unwrap(),
            Some(expected)
        );
        assert!(matches!(
            normalize_media_root(Some("relative/media".into())),
            Err(ConfigError::MediaRoot)
        ));
        assert_eq!(
            normalize_media_root(Some(std::ffi::OsString::new())).unwrap(),
            None
        );
    }
}
