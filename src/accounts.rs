use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{ad_account as normalize_account_id, numeric_owned as normalize_numeric_id},
};

const ACCOUNT_FIELDS: &str = "id,name,account_id,account_status,amount_spent,balance,currency,age,business_city,business_country_code";
const ACCOUNT_DETAIL_FIELDS: &str = "id,name,account_id,account_status,amount_spent,balance,currency,age,business_city,business_country_code,timezone_name";
const DEFAULT_PAGE_SIZE: u16 = 25;
const MAX_PAGE_SIZE: u16 = 100;
const MAX_CURSOR_CHARS: usize = 2_048;
const MAX_NAME_CHARS: usize = 256;
const MAX_METADATA_CHARS: usize = 128;

const ZERO_DECIMAL_CURRENCIES: &[&str] = &[
    "BIF", "CLP", "DJF", "GNF", "JPY", "KMF", "KRW", "MGA", "PYG", "RWF", "UGX", "VND", "VUV",
    "XAF", "XOF", "XPF",
];

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListAdAccountsInput {
    /// Meta user context. Defaults to `me`.
    pub meta_user_id: Option<String>,
    /// Number of accounts to return, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadAdAccountInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    pub ad_account_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AdAccountList {
    pub accounts: Vec<AdAccount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AdAccount {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_status: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_spent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub business_city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub business_country_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawAccountList {
    #[serde(default)]
    data: Vec<RawAdAccount>,
    paging: Option<RawPaging>,
}

#[derive(Debug, Deserialize)]
struct RawPaging {
    cursors: Option<RawCursors>,
}

#[derive(Debug, Deserialize)]
struct RawCursors {
    after: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawAdAccount {
    id: Option<String>,
    account_id: Option<String>,
    name: Option<String>,
    account_status: Option<i64>,
    amount_spent: Option<Value>,
    balance: Option<Value>,
    currency: Option<String>,
    age: Option<f64>,
    business_city: Option<String>,
    business_country_code: Option<String>,
    timezone_name: Option<String>,
}

pub(crate) async fn list_ad_accounts(
    graph: &GraphClient,
    input: ListAdAccountsInput,
) -> ToolResponse<AdAccountList> {
    let user_id = input.meta_user_id.as_deref().unwrap_or("me").trim();
    if !valid_user_id(user_id) {
        return ToolResponse::error(PublicError::invalid_input(
            "meta_user_id must be `me` or a Meta object ID",
            "Use `me` or a numeric Meta user ID",
        ));
    }

    let page_size = input.page_size.unwrap_or(DEFAULT_PAGE_SIZE);
    if !(1..=MAX_PAGE_SIZE).contains(&page_size) {
        return ToolResponse::error(PublicError::invalid_input(
            "page_size must be between 1 and 100",
            "Choose a bounded page_size and paginate with next_cursor",
        ));
    }
    let cursor = input
        .page_cursor
        .as_deref()
        .filter(|cursor| !cursor.is_empty());
    if cursor.is_some_and(|cursor| cursor.chars().count() > MAX_CURSOR_CHARS) {
        return ToolResponse::error(PublicError::invalid_input(
            "page_cursor is too long",
            "Use next_cursor exactly as returned by the previous response",
        ));
    }

    let mut query = vec![
        ("fields".to_owned(), ACCOUNT_FIELDS.to_owned()),
        ("limit".to_owned(), page_size.to_string()),
    ];
    if let Some(cursor) = cursor {
        query.push(("after".to_owned(), cursor.to_owned()));
    }

    let endpoint = format!("{user_id}/adaccounts");
    let payload = match graph.get_json(&endpoint, &query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    let raw = match serde_json::from_value::<RawAccountList>(payload) {
        Ok(raw) => raw,
        Err(_) => {
            return ToolResponse::error(PublicError::invalid_upstream(
                "Meta returned an unexpected ad-account list",
            ));
        }
    };
    if raw.data.len() > usize::from(page_size) {
        return ToolResponse::error(PublicError::invalid_upstream(
            "Meta returned more ad accounts than requested",
        ));
    }

    let next_cursor = raw
        .paging
        .and_then(|paging| paging.cursors)
        .and_then(|cursors| cursors.after)
        .filter(|cursor| !cursor.is_empty());
    if next_cursor
        .as_ref()
        .is_some_and(|cursor| cursor.chars().count() > MAX_CURSOR_CHARS)
    {
        return ToolResponse::error(PublicError::invalid_upstream(
            "Meta returned an oversized ad-account cursor",
        ));
    }
    let accounts = raw.data.into_iter().filter_map(normalize_account).collect();
    ToolResponse::success(AdAccountList {
        accounts,
        next_cursor,
    })
}

pub(crate) async fn read_ad_account(
    graph: &GraphClient,
    input: ReadAdAccountInput,
) -> ToolResponse<AdAccount> {
    let Some(account_id) = normalize_account_id(&input.ad_account_id) else {
        return ToolResponse::error(PublicError::invalid_input(
            "ad_account_id must be a numeric Meta account ID",
            "Use an ID such as `act_123456789` or `123456789`",
        ));
    };
    let query = vec![("fields".to_owned(), ACCOUNT_DETAIL_FIELDS.to_owned())];
    let payload = match graph.get_json(&account_id, &query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    let raw = match serde_json::from_value::<RawAdAccount>(payload) {
        Ok(raw) => raw,
        Err(_) => {
            return ToolResponse::error(PublicError::invalid_upstream(
                "Meta returned unexpected ad-account metadata",
            ));
        }
    };
    match normalize_account(raw) {
        Some(account) => ToolResponse::success(account),
        None => ToolResponse::error(PublicError::invalid_upstream(
            "Meta omitted the ad-account ID",
        )),
    }
}

fn normalize_account(raw: RawAdAccount) -> Option<AdAccount> {
    let id = normalize_account_id(raw.id.as_deref()?)?;
    let currency = normalize_currency(raw.currency);
    let amount_spent = normalize_minor_units(raw.amount_spent, currency.as_deref());
    let balance = normalize_minor_units(raw.balance, currency.as_deref());

    Some(AdAccount {
        id,
        account_id: raw.account_id.as_deref().and_then(normalize_numeric_id),
        name: bounded_text(raw.name, MAX_NAME_CHARS),
        account_status: raw.account_status,
        amount_spent,
        balance,
        currency,
        age: raw.age,
        business_city: bounded_text(raw.business_city, MAX_METADATA_CHARS),
        business_country_code: bounded_text(raw.business_country_code, 2)
            .map(|value| value.to_ascii_uppercase()),
        timezone_name: bounded_text(raw.timezone_name, MAX_METADATA_CHARS),
    })
}

fn normalize_minor_units(value: Option<Value>, currency: Option<&str>) -> Option<String> {
    let value = value?;
    let raw = value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_i64().map(|number| number.to_string()))?;
    let number = raw.parse::<i128>().ok()?;
    if currency.is_some_and(|code| ZERO_DECIMAL_CURRENCIES.contains(&code)) {
        Some(number.to_string())
    } else {
        let magnitude = number.unsigned_abs();
        let sign = if number.is_negative() { "-" } else { "" };
        Some(format!("{sign}{}.{:02}", magnitude / 100, magnitude % 100))
    }
}

fn valid_user_id(value: &str) -> bool {
    value == "me" || normalize_numeric_id(value).is_some()
}

fn normalize_currency(value: Option<String>) -> Option<String> {
    let value = value?.trim().to_ascii_uppercase();
    (value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_alphabetic())).then_some(value)
}

fn bounded_text(value: Option<String>, max_chars: usize) -> Option<String> {
    let value = value?;
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let mut chars = value.chars();
    let mut output = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        output.pop();
        output.push('…');
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{normalize_account_id, normalize_minor_units, valid_user_id};

    #[test]
    fn normalizes_account_ids_without_accepting_paths() {
        assert_eq!(normalize_account_id("123"), Some("act_123".to_owned()));
        assert_eq!(normalize_account_id("act_123"), Some("act_123".to_owned()));
        assert_eq!(normalize_account_id("../123"), None);
        assert_eq!(normalize_account_id(&"1".repeat(65)), None);
        assert!(valid_user_id("me"));
        assert!(valid_user_id("123"));
        assert!(!valid_user_id("me/adaccounts"));
    }

    #[test]
    fn normalizes_currency_minor_units() {
        assert_eq!(
            normalize_minor_units(Some(json!(12345)), Some("USD")),
            Some("123.45".to_owned())
        );
        assert_eq!(
            normalize_minor_units(Some(json!(12345)), Some("JPY")),
            Some("12345".to_owned())
        );
        assert_eq!(
            normalize_minor_units(Some(json!(-5)), Some("USD")),
            Some("-0.05".to_owned())
        );
    }
}
