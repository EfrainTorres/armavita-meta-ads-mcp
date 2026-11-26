use rmcp::{
    Json,
    schemars::{self, JsonSchema},
};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StartupError {
    #[error("invalid Meta access token header")]
    InvalidAccessToken,
    #[error("failed to construct HTTP client: {0}")]
    HttpClient(#[from] reqwest::Error),
}

#[derive(Debug, Error)]
pub(crate) enum GraphError {
    #[error("Meta authentication is not configured")]
    NotAuthenticated,
    #[error("Meta rejected the configured access token")]
    AuthExpired,
    #[error("Meta did not provide Page authorization")]
    PageAccessRequired,
    #[error("invalid Graph API endpoint")]
    InvalidEndpoint,
    #[error("invalid Graph API query")]
    InvalidQuery,
    #[error("Meta response exceeded the {limit} byte safety limit")]
    ResponseTooLarge { limit: usize },
    #[error("Meta returned invalid JSON")]
    InvalidJson,
    #[error("Meta request failed: {message}")]
    Transport { message: String },
    #[error("Meta API error {status}: {message}")]
    Api {
        status: u16,
        code: Option<i64>,
        message: String,
        retryable: bool,
        /// Non-prose provider diagnostics and throttling hints.
        details: Option<Box<ApiErrorDetails>>,
    },
}

/// Structured diagnostics Meta attaches to a rejected Graph request.
///
/// Meta's prose (`message`, `error_user_title`, `error_user_msg`) is never
/// copied: it can echo private request input in arbitrary formats. Only codes,
/// short machine identifiers validated against a fixed alphabet, and a static
/// server-side `hint` keyed by code and subcode cross this boundary.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct MetaErrorDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_subcode: Option<i64>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_transient: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fbtrace_id: Option<String>,
    /// Server-authored guidance for well-known Marketing API errors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<RateLimitDetails>,
}

/// Throttling state reported by Meta's rate-limit codes and usage headers.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct RateLimitDetails {
    /// Which limit Meta enforced: `business_use_case`, `ad_account`,
    /// `application`, `user`, `page`, or `ads_insights`.
    pub scope: String,
    /// Highest utilisation percentage reported by Meta's usage headers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_pct: Option<u32>,
    /// Business use case named by `X-Business-Use-Case-Usage`, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub business_use_case: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_time_to_regain_access_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ads_api_access_tier: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ApiErrorDetails {
    pub(crate) meta: Option<MetaErrorDetails>,
    pub(crate) retry_after_seconds: Option<u64>,
}

/// Optional structured fields shared by every public error.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct PublicErrorDetails {
    /// Seconds to wait before Meta is expected to accept the request again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<u64>,
    // Boxed so `Result<_, PublicError>` stays small on every tool path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Box<MetaErrorDetails>>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PublicError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(flatten)]
    pub details: PublicErrorDetails,
}

impl PublicError {
    pub(crate) fn invalid_input(message: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            code: "INVALID_INPUT".to_owned(),
            message: message.into(),
            retryable: false,
            action: Some(action.into()),
            details: PublicErrorDetails::default(),
        }
    }

    pub(crate) fn invalid_upstream(message: impl Into<String>) -> Self {
        Self {
            code: "INVALID_UPSTREAM_RESPONSE".to_owned(),
            message: message.into(),
            retryable: true,
            action: Some("Retry once; inspect Meta service status if it persists".to_owned()),
            details: PublicErrorDetails::default(),
        }
    }
}

impl From<GraphError> for PublicError {
    fn from(error: GraphError) -> Self {
        match error {
            GraphError::NotAuthenticated => Self {
                code: "AUTH_REQUIRED".to_owned(),
                message: "Meta authentication is not configured".to_owned(),
                retryable: false,
                action: Some("Set META_ACCESS_TOKEN or complete the OAuth login flow".to_owned()),
                details: PublicErrorDetails::default(),
            },
            GraphError::AuthExpired => Self {
                code: "AUTH_EXPIRED".to_owned(),
                message: "Meta rejected the configured access token".to_owned(),
                retryable: false,
                action: Some(
                    "Complete OAuth login again or restart with a new META_ACCESS_TOKEN; the next request re-reads the token"
                        .to_owned(),
                ),
                details: PublicErrorDetails::default(),
            },
            GraphError::PageAccessRequired => Self {
                code: "PAGE_ACCESS_REQUIRED".to_owned(),
                message: "Meta did not provide a usable Page access token".to_owned(),
                retryable: false,
                action: Some("Grant the app the required Page permissions and asset tasks, then refresh the server login".to_owned()),
                details: PublicErrorDetails::default(),
            },
            GraphError::InvalidEndpoint => Self {
                code: "INVALID_INPUT".to_owned(),
                message: "The requested Meta object identifier is invalid".to_owned(),
                retryable: false,
                action: Some("Use a Meta numeric ID without URL or path characters".to_owned()),
                details: PublicErrorDetails::default(),
            },
            GraphError::InvalidQuery => Self {
                code: "INVALID_INPUT".to_owned(),
                message: "The Meta request parameters exceed the server safety limits".to_owned(),
                retryable: false,
                action: Some("Request fewer fields, filters, or identifiers".to_owned()),
                details: PublicErrorDetails::default(),
            },
            GraphError::ResponseTooLarge { limit } => Self {
                code: "RESPONSE_TOO_LARGE".to_owned(),
                message: format!("Meta response exceeded the {limit} byte safety limit"),
                retryable: false,
                action: Some("Request fewer fields or a smaller page".to_owned()),
                details: PublicErrorDetails::default(),
            },
            GraphError::InvalidJson => Self {
                code: "INVALID_UPSTREAM_RESPONSE".to_owned(),
                message: "Meta returned an invalid JSON response".to_owned(),
                retryable: true,
                action: Some("Retry once; inspect Meta service status if it persists".to_owned()),
                details: PublicErrorDetails::default(),
            },
            GraphError::Transport { message } => Self {
                code: "META_UNAVAILABLE".to_owned(),
                message,
                retryable: true,
                action: Some("Retry after checking network connectivity".to_owned()),
                details: PublicErrorDetails::default(),
            },
            GraphError::Api {
                status,
                code,
                message: _,
                retryable,
                details,
            } => {
                let auth_error = matches!(code, Some(102 | 190));
                let details = details.map(|details| *details).unwrap_or_default();
                let rate_limited = details
                    .meta
                    .as_ref()
                    .is_some_and(|meta| meta.rate_limit.is_some());
                Self {
                    code: if auth_error {
                        "AUTH_EXPIRED".to_owned()
                    } else if rate_limited {
                        "RATE_LIMITED".to_owned()
                    } else {
                        "META_API_ERROR".to_owned()
                    },
                    message: format_meta_error(status, code),
                    retryable,
                    action: if auth_error {
                        Some(
                            "Refresh META_ACCESS_TOKEN or complete OAuth login again; the next request re-reads the token"
                                .to_owned(),
                        )
                    } else if rate_limited {
                        Some(match details.retry_after_seconds {
                            Some(seconds) => format!(
                                "Wait at least {seconds} seconds before calling Meta again for this account"
                            ),
                            None => {
                                "Wait before calling Meta again and reduce request volume".to_owned()
                            }
                        })
                    } else {
                        None
                    },
                    details: PublicErrorDetails {
                        retry_after_seconds: details.retry_after_seconds,
                        meta: details.meta.map(Box::new),
                    },
                }
            }
        }
    }
}

fn format_meta_error(status: u16, code: Option<i64>) -> String {
    match code {
        Some(code) => format!("Meta API error {code} (HTTP {status}): request rejected"),
        None => format!("Meta API HTTP {status}: request rejected"),
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ToolResponse<T> {
    Success { data: T },
    Error { error: PublicError },
}

impl<T> ToolResponse<T> {
    pub(crate) fn success(data: T) -> Self {
        Self::Success { data }
    }

    pub(crate) fn error(error: impl Into<PublicError>) -> Self {
        Self::Error {
            error: error.into(),
        }
    }

    pub(crate) fn into_mcp_result(self) -> Result<Json<Self>, Json<Self>> {
        if matches!(self, Self::Error { .. }) {
            Err(Json(self))
        } else {
            Ok(Json(self))
        }
    }
}
