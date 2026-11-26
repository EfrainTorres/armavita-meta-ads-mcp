use crate::error::{GraphError, PublicError};

/// Convert a Graph write failure without inviting a blind retry when Meta may
/// have committed the mutation before the response failed.
pub(crate) fn mutation_error_without_blind_retry(
    error: GraphError,
    verify_action: &str,
) -> PublicError {
    let outcome_is_ambiguous = matches!(
        &error,
        GraphError::Transport { .. }
            | GraphError::InvalidJson
            | GraphError::ResponseTooLarge { .. }
            | GraphError::Api {
                retryable: true,
                ..
            }
    );
    // Meta details and any `retry_after_seconds` survive: they tell the caller
    // how long to wait before verifying, even though the write is not retried.
    let mut error = PublicError::from(error);
    if outcome_is_ambiguous {
        error.retryable = false;
        error.action = Some(verify_action.to_owned());
    }
    error
}

pub(crate) fn ambiguous_mutation_result(
    message: impl Into<String>,
    verify_action: &str,
) -> PublicError {
    PublicError {
        code: "AMBIGUOUS_MUTATION_RESULT".to_owned(),
        message: message.into(),
        retryable: false,
        action: Some(verify_action.to_owned()),
        details: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::{ambiguous_mutation_result, mutation_error_without_blind_retry};
    use crate::error::{ApiErrorDetails, GraphError, MetaErrorDetails, RateLimitDetails};

    #[test]
    fn only_uncertain_write_outcomes_replace_the_retry_action() {
        let verify_action = "Verify provider state before retrying";
        let cases = [
            (GraphError::NotAuthenticated, false),
            (GraphError::InvalidEndpoint, false),
            (GraphError::InvalidQuery, false),
            (GraphError::ResponseTooLarge { limit: 1 }, true),
            (GraphError::InvalidJson, true),
            (
                GraphError::Transport {
                    message: "request failed".to_owned(),
                },
                true,
            ),
            (
                GraphError::Api {
                    status: 503,
                    code: Some(2),
                    message: "temporarily unavailable".to_owned(),
                    retryable: true,
                    details: None,
                },
                true,
            ),
            (
                GraphError::Api {
                    status: 400,
                    code: Some(100),
                    message: "invalid parameter".to_owned(),
                    retryable: false,
                    details: None,
                },
                false,
            ),
        ];

        for (graph_error, outcome_is_ambiguous) in cases {
            let error = mutation_error_without_blind_retry(graph_error, verify_action);
            assert!(!error.retryable);
            assert_eq!(
                error.action.as_deref() == Some(verify_action),
                outcome_is_ambiguous
            );
        }
    }

    #[test]
    fn throttled_writes_keep_the_wait_and_meta_details_but_never_retry() {
        let error = mutation_error_without_blind_retry(
            GraphError::Api {
                status: 400,
                code: Some(80004),
                message: "request rejected".to_owned(),
                retryable: true,
                details: Some(Box::new(ApiErrorDetails {
                    meta: Some(MetaErrorDetails {
                        code: Some(80004),
                        error_subcode: Some(2_446_079),
                        fbtrace_id: Some("AbC123".to_owned()),
                        rate_limit: Some(RateLimitDetails {
                            scope: "business_use_case".to_owned(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    retry_after_seconds: Some(600),
                })),
            },
            "Verify provider state before retrying",
        );
        assert_eq!(error.code, "RATE_LIMITED");
        assert!(!error.retryable);
        assert_eq!(
            error.action.as_deref(),
            Some("Verify provider state before retrying")
        );
        assert_eq!(error.details.retry_after_seconds, Some(600));
        let meta = error.details.meta.as_ref().unwrap();
        assert_eq!(meta.error_subcode, Some(2_446_079));
        assert_eq!(meta.fbtrace_id.as_deref(), Some("AbC123"));
        let encoded = serde_json::to_value(&error).unwrap();
        assert_eq!(encoded["retry_after_seconds"], 600);
        assert_eq!(encoded["meta"]["rate_limit"]["scope"], "business_use_case");
    }

    #[test]
    fn ambiguous_results_are_explicit_and_never_retryable() {
        let error = ambiguous_mutation_result("Meta did not confirm the ID", "Inspect the list");
        assert_eq!(error.code, "AMBIGUOUS_MUTATION_RESULT");
        assert_eq!(error.message, "Meta did not confirm the ID");
        assert!(!error.retryable);
        assert_eq!(error.action.as_deref(), Some("Inspect the list"));
    }
}
