use std::{env, ffi::OsString, fmt, path::PathBuf, time::Duration};

use thiserror::Error;

use crate::auth::load_cached_access_token;

pub(crate) const DEFAULT_GRAPH_API_VERSION: &str = "v26.0";
pub(crate) const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_RETRIES: u8 = 3;
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const MEDIA_ROOT_ENV: &str = "META_MEDIA_ROOT";

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

#[derive(Clone, Debug)]
pub struct MetaConfig {
    pub(crate) api_base: String,
    pub(crate) access_token: Option<AccessToken>,
    pub(crate) token_origin: Option<TokenOrigin>,
    pub(crate) media_root: Option<PathBuf>,
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

        Ok(Self {
            api_base: format!("https://graph.facebook.com/{DEFAULT_GRAPH_API_VERSION}"),
            access_token,
            token_origin,
            media_root,
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(api_base: impl Into<String>, access_token: Option<&str>) -> Self {
        Self {
            api_base: api_base.into(),
            access_token: access_token.map(|value| AccessToken(value.to_owned())),
            token_origin: access_token.map(|_| TokenOrigin::Environment),
            media_root: None,
        }
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

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("META_ACCESS_TOKEN appears malformed")]
    AccessToken,
    #[error("META_MEDIA_ROOT must name an existing absolute directory")]
    MediaRoot,
}

#[cfg(test)]
mod tests {
    use super::{ConfigError, DEFAULT_GRAPH_API_VERSION, normalize_media_root};

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
