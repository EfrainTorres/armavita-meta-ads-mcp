use serde_json::Value;

pub(crate) const MAX_DIGITS: usize = 64;

pub(crate) fn numeric(raw: &str) -> Option<&str> {
    let value = raw.trim();
    valid_digits(value).then_some(value)
}

pub(crate) fn numeric_owned(raw: &str) -> Option<String> {
    numeric(raw).map(str::to_owned)
}

pub(crate) fn ad_account(raw: &str) -> Option<String> {
    ad_account_digits(raw).map(|digits| format!("act_{digits}"))
}

pub(crate) fn ad_account_digits(raw: &str) -> Option<&str> {
    let value = raw.trim();
    let digits = value.strip_prefix("act_").unwrap_or(value);
    valid_digits(digits).then_some(digits)
}

fn valid_digits(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_DIGITS
        && value.bytes().all(|byte| byte.is_ascii_digit())
}

pub(crate) fn numeric_value(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => numeric_owned(value),
        Value::Number(value) => value.as_u64().map(|value| value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ad_account, numeric, numeric_value};

    #[test]
    fn accepts_only_bounded_meta_ids() {
        let sixty_four = "1".repeat(64);
        let sixty_five = "1".repeat(65);
        for (raw, expected) in [
            ("1", Some("1")),
            (" 123 ", Some("123")),
            (sixty_four.as_str(), Some(sixty_four.as_str())),
            ("", None),
            ("   ", None),
            (sixty_five.as_str(), None),
            ("+1", None),
            ("-1", None),
            ("1.0", None),
            ("123/path", None),
            ("https://example.test/123", None),
            ("١٢٣", None),
        ] {
            assert_eq!(numeric(raw), expected, "raw={raw:?}");
        }

        for (raw, expected) in [
            ("123", Some("act_123")),
            (" act_123 ", Some("act_123")),
            ("act_ 123", None),
            ("ACT_123", None),
            ("act_act_123", None),
            ("act_", None),
            ("act_123/path", None),
        ] {
            assert_eq!(ad_account(raw).as_deref(), expected, "raw={raw:?}");
        }

        for (value, expected) in [
            (json!(" 123 "), Some("123")),
            (json!(123_u64), Some("123")),
            (json!(-1), None),
            (json!(1.5), None),
            (json!(true), None),
            (json!(null), None),
        ] {
            assert_eq!(numeric_value(&value).as_deref(), expected, "value={value}");
        }
    }
}
