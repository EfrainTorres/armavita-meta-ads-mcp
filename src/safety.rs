use crate::error::PublicError;

pub(crate) const REMOVAL_ACKNOWLEDGEMENT: &str = "CONFIRM_META_ADS_REMOVALS";

pub(crate) fn validate_removal_acknowledgement(
    required: bool,
    acknowledgement: Option<&str>,
) -> Result<(), PublicError> {
    if acknowledgement.is_some_and(|value| value != REMOVAL_ACKNOWLEDGEMENT) {
        return Err(invalid_removal_acknowledgement());
    }
    if required && acknowledgement.is_none() {
        return Err(invalid_removal_acknowledgement());
    }
    Ok(())
}

fn invalid_removal_acknowledgement() -> PublicError {
    PublicError::invalid_input(
        "The Meta Ads removal acknowledgement is missing or invalid",
        "Obtain explicit operator approval, then pass CONFIRM_META_ADS_REMOVALS exactly as removal_acknowledgement",
    )
}

#[cfg(test)]
mod tests {
    use rmcp::schemars::{JsonSchema, schema_for};
    use serde_json::Value;

    use crate::{
        audience_mutations::{DeleteCustomAudienceInput, ManageCustomAudienceUsersInput},
        commerce_mutations::BatchProductsInput,
        custom_conversion_mutations::DeleteCustomConversionInput,
    };

    use super::{REMOVAL_ACKNOWLEDGEMENT, validate_removal_acknowledgement};

    #[test]
    fn requires_the_exact_phrase_only_for_removals() {
        assert!(validate_removal_acknowledgement(false, None).is_ok());
        assert!(validate_removal_acknowledgement(false, Some(REMOVAL_ACKNOWLEDGEMENT)).is_ok());
        assert!(validate_removal_acknowledgement(false, Some("yes")).is_err());
        assert!(validate_removal_acknowledgement(true, None).is_err());
        assert!(validate_removal_acknowledgement(true, Some("yes")).is_err());
        assert!(validate_removal_acknowledgement(true, Some(REMOVAL_ACKNOWLEDGEMENT)).is_ok());
    }

    #[test]
    fn destructive_input_schemas_publish_the_exact_bounded_phrase() {
        for schema in [
            schema::<DeleteCustomAudienceInput>(),
            schema::<ManageCustomAudienceUsersInput>(),
            schema::<BatchProductsInput>(),
            schema::<DeleteCustomConversionInput>(),
        ] {
            let acknowledgement = schema
                .pointer("/properties/removal_acknowledgement")
                .expect("removal acknowledgement schema");
            assert_eq!(acknowledgement["minLength"], 25);
            assert_eq!(acknowledgement["maxLength"], 25);
            assert_eq!(acknowledgement["pattern"], "^CONFIRM_META_ADS_REMOVALS$");
        }
    }

    fn schema<T: JsonSchema>() -> Value {
        serde_json::to_value(schema_for!(T)).expect("JSON schema")
    }
}
