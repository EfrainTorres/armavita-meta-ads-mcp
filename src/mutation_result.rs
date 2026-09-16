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
    }
}

#[cfg(test)]
mod tests {
    use super::{ambiguous_mutation_result, mutation_error_without_blind_retry};
    use crate::error::GraphError;

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
                },
                true,
            ),
            (
                GraphError::Api {
                    status: 400,
                    code: Some(100),
                    message: "invalid parameter".to_owned(),
                    retryable: false,
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
    fn ambiguous_results_are_explicit_and_never_retryable() {
        let error = ambiguous_mutation_result("Meta did not confirm the ID", "Inspect the list");
        assert_eq!(error.code, "AMBIGUOUS_MUTATION_RESULT");
        assert_eq!(error.message, "Meta did not confirm the ID");
        assert!(!error.retryable);
        assert_eq!(error.action.as_deref(), Some("Inspect the list"));
    }
}
