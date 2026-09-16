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
    },
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PublicError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

impl PublicError {
    pub(crate) fn invalid_input(message: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            code: "INVALID_INPUT".to_owned(),
            message: message.into(),
            retryable: false,
            action: Some(action.into()),
        }
    }

    pub(crate) fn invalid_upstream(message: impl Into<String>) -> Self {
        Self {
            code: "INVALID_UPSTREAM_RESPONSE".to_owned(),
            message: message.into(),
            retryable: true,
            action: Some("Retry once; inspect Meta service status if it persists".to_owned()),
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
            },
            GraphError::InvalidEndpoint => Self {
                code: "INVALID_INPUT".to_owned(),
                message: "The requested Meta object identifier is invalid".to_owned(),
                retryable: false,
                action: Some("Use a Meta numeric ID without URL or path characters".to_owned()),
            },
            GraphError::InvalidQuery => Self {
                code: "INVALID_INPUT".to_owned(),
                message: "The Meta request parameters exceed the server safety limits".to_owned(),
                retryable: false,
                action: Some("Request fewer fields, filters, or identifiers".to_owned()),
            },
            GraphError::ResponseTooLarge { limit } => Self {
                code: "RESPONSE_TOO_LARGE".to_owned(),
                message: format!("Meta response exceeded the {limit} byte safety limit"),
                retryable: false,
                action: Some("Request fewer fields or a smaller page".to_owned()),
            },
            GraphError::InvalidJson => Self {
                code: "INVALID_UPSTREAM_RESPONSE".to_owned(),
                message: "Meta returned an invalid JSON response".to_owned(),
                retryable: true,
                action: Some("Retry once; inspect Meta service status if it persists".to_owned()),
            },
            GraphError::Transport { message } => Self {
                code: "META_UNAVAILABLE".to_owned(),
                message,
                retryable: true,
                action: Some("Retry after checking network connectivity".to_owned()),
            },
            GraphError::Api {
                status,
                code,
                message: _,
                retryable,
            } => {
                let auth_error = matches!(code, Some(102 | 190));
                Self {
                    code: if auth_error {
                        "AUTH_EXPIRED".to_owned()
                    } else {
                        "META_API_ERROR".to_owned()
                    },
                    message: format_meta_error(status, code),
                    retryable,
                    action: auth_error.then(|| {
                        "Refresh META_ACCESS_TOKEN or complete OAuth login again".to_owned()
                    }),
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
