use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{numeric_owned as normalize_numeric_id, numeric_value as meta_numeric_value},
};

const PAGE_FIELDS: &str =
    "id,name,username,category,fan_count,link,verification_status,is_published";
const DEFAULT_PAGE_SIZE: u16 = 25;
const MAX_PAGE_SIZE: u16 = 100;
const MAX_CURSOR_CHARS: usize = 2_048;
const MAX_QUERY_CHARS: usize = 100;
const MAX_NAME_CHARS: usize = 256;
const MAX_METADATA_CHARS: usize = 128;
const MAX_URL_CHARS: usize = 2_048;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListPagesInput {
    /// Page relationship to list. Defaults to `user_promotable`.
    pub source: Option<PageSource>,
    /// Required for business-owned or business-client pages; otherwise omit.
    pub business_id: Option<String>,
    /// Optional case-insensitive name or username filter applied to this page of results.
    pub query: Option<String>,
    /// Number of Pages to request, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PageSource {
    UserPromotable,
    BusinessOwned,
    BusinessClient,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PageList {
    pub source: PageSource,
    pub pages: Vec<FacebookPage>,
    pub matched_on_page: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FacebookPage {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fan_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_published: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

#[derive(Debug)]
struct PageRequest {
    endpoint: String,
    query: Vec<(String, String)>,
    page_size: u16,
    source: PageSource,
    name_query: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawPageList {
    #[serde(default)]
    data: Vec<RawPage>,
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
struct RawPage {
    id: Option<Value>,
    name: Option<String>,
    username: Option<String>,
    category: Option<String>,
    fan_count: Option<Value>,
    link: Option<String>,
    verification_status: Option<String>,
    is_published: Option<bool>,
}

pub(crate) async fn list_pages(
    graph: &GraphClient,
    input: ListPagesInput,
) -> ToolResponse<PageList> {
    let request = match build_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.get_json(&request.endpoint, &request.query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    match normalize_page(payload, &request) {
        Ok(page) => ToolResponse::success(page),
        Err(error) => ToolResponse::error(error),
    }
}

fn build_request(input: &ListPagesInput) -> Result<PageRequest, PublicError> {
    let source = input.source.unwrap_or(PageSource::UserPromotable);
    let endpoint = match source {
        PageSource::UserPromotable => {
            if input.business_id.is_some() {
                return Err(PublicError::invalid_input(
                    "business_id cannot be used with user_promotable pages",
                    "Omit business_id or choose a business page source",
                ));
            }
            "me/accounts".to_owned()
        }
        PageSource::BusinessOwned | PageSource::BusinessClient => {
            let business_id = input
                .business_id
                .as_deref()
                .and_then(normalize_numeric_id)
                .ok_or_else(|| {
                    PublicError::invalid_input(
                        "business_id is required and must be numeric",
                        "Provide a Meta business ID for the selected source",
                    )
                })?;
            let edge = match source {
                PageSource::BusinessOwned => "owned_pages",
                PageSource::BusinessClient => "client_pages",
                PageSource::UserPromotable => unreachable!(),
            };
            format!("{business_id}/{edge}")
        }
    };

    let page_size = input.page_size.unwrap_or(DEFAULT_PAGE_SIZE);
    if !(1..=MAX_PAGE_SIZE).contains(&page_size) {
        return Err(PublicError::invalid_input(
            "page_size must be between 1 and 100",
            "Choose a bounded page_size and paginate with next_cursor",
        ));
    }
    let cursor = input
        .page_cursor
        .as_deref()
        .filter(|cursor| !cursor.is_empty());
    if cursor.is_some_and(|cursor| {
        cursor.chars().count() > MAX_CURSOR_CHARS || cursor.chars().any(char::is_control)
    }) {
        return Err(PublicError::invalid_input(
            "page_cursor is invalid or too long",
            "Use next_cursor exactly as returned by the previous response",
        ));
    }
    let name_query = input
        .query
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .map(|query| {
            if query.chars().count() > MAX_QUERY_CHARS || query.chars().any(char::is_control) {
                Err(PublicError::invalid_input(
                    "query must be at most 100 characters without control characters",
                    "Use a shorter Page name or username fragment",
                ))
            } else {
                Ok(query.to_lowercase())
            }
        })
        .transpose()?;

    let mut query = vec![
        ("fields".to_owned(), PAGE_FIELDS.to_owned()),
        ("limit".to_owned(), page_size.to_string()),
    ];
    if matches!(source, PageSource::UserPromotable) {
        query.push(("is_promotable".to_owned(), "true".to_owned()));
    }
    if let Some(cursor) = cursor {
        query.push(("after".to_owned(), cursor.to_owned()));
    }
    Ok(PageRequest {
        endpoint,
        query,
        page_size,
        source,
        name_query,
    })
}

fn normalize_page(payload: Value, request: &PageRequest) -> Result<PageList, PublicError> {
    let raw = serde_json::from_value::<RawPageList>(payload)
        .map_err(|_| PublicError::invalid_upstream("Meta returned an unexpected Page list"))?;
    if raw.data.len() > usize::from(request.page_size) {
        return Err(PublicError::invalid_upstream(
            "Meta returned more Pages than requested",
        ));
    }
    let next_cursor = raw
        .paging
        .and_then(|paging| paging.cursors)
        .and_then(|cursors| cursors.after)
        .filter(|cursor| !cursor.is_empty());
    if next_cursor.as_ref().is_some_and(|cursor| {
        cursor.chars().count() > MAX_CURSOR_CHARS || cursor.chars().any(char::is_control)
    }) {
        return Err(PublicError::invalid_upstream(
            "Meta returned an invalid Page cursor",
        ));
    }

    let mut pages = Vec::with_capacity(raw.data.len());
    for raw in raw.data {
        let page = normalize_result(raw).ok_or_else(|| {
            PublicError::invalid_upstream("Meta omitted a valid Facebook Page ID")
        })?;
        if request.name_query.as_ref().is_none_or(|query| {
            page.name
                .as_ref()
                .is_some_and(|name| name.to_lowercase().contains(query))
                || page
                    .username
                    .as_ref()
                    .is_some_and(|username| username.to_lowercase().contains(query))
        }) {
            pages.push(page);
        }
    }
    let matched_on_page = u16::try_from(pages.len()).unwrap_or(u16::MAX);
    Ok(PageList {
        source: request.source,
        pages,
        matched_on_page,
        next_cursor,
    })
}

fn normalize_result(raw: RawPage) -> Option<FacebookPage> {
    Some(FacebookPage {
        id: normalize_numeric_value(raw.id)?,
        name: bounded_text(raw.name, MAX_NAME_CHARS),
        username: bounded_text(raw.username, MAX_NAME_CHARS),
        category: bounded_text(raw.category, MAX_METADATA_CHARS),
        fan_count: nonnegative_integer(raw.fan_count),
        verification_status: bounded_text(raw.verification_status, MAX_METADATA_CHARS),
        is_published: raw.is_published,
        link: bounded_url(raw.link),
    })
}

fn normalize_numeric_value(raw: Option<Value>) -> Option<String> {
    meta_numeric_value(&raw?)
}

fn nonnegative_integer(raw: Option<Value>) -> Option<u64> {
    match raw? {
        Value::Number(value) => value.as_u64(),
        Value::String(value) => value.trim().parse().ok(),
        _ => None,
    }
}

fn bounded_text(raw: Option<String>, max_chars: usize) -> Option<String> {
    let value = raw?.trim().to_owned();
    if value.is_empty() {
        return None;
    }
    if value.chars().count() <= max_chars {
        return Some(value);
    }
    let mut bounded = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    bounded.push('…');
    Some(bounded)
}

fn bounded_url(raw: Option<String>) -> Option<String> {
    let value = raw?.trim().to_owned();
    if value.is_empty() || value.chars().count() > MAX_URL_CHARS {
        return None;
    }
    let mut url = reqwest::Url::parse(&value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    url.set_fragment(None);
    Some(url.into())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ListPagesInput, PageSource, build_request, normalize_page};

    fn input() -> ListPagesInput {
        ListPagesInput {
            source: None,
            business_id: None,
            query: None,
            page_size: None,
            page_cursor: None,
        }
    }

    #[test]
    fn builds_only_current_documented_page_edges() {
        let user = build_request(&input()).unwrap();
        assert_eq!(user.endpoint, "me/accounts");
        assert!(
            user.query
                .contains(&("is_promotable".to_owned(), "true".to_owned()))
        );

        let business = build_request(&ListPagesInput {
            source: Some(PageSource::BusinessClient),
            business_id: Some("123".to_owned()),
            query: None,
            page_size: Some(10),
            page_cursor: Some("opaque+/=cursor".to_owned()),
        })
        .unwrap();
        assert_eq!(business.endpoint, "123/client_pages");
        assert!(
            business
                .query
                .contains(&("after".to_owned(), "opaque+/=cursor".to_owned()))
        );
        assert!(!business.query.iter().any(|(key, _)| key == "access_token"));
    }

    #[test]
    fn rejects_ambiguous_or_unbounded_page_requests() {
        let mut invalid = input();
        invalid.business_id = Some("123".to_owned());
        assert!(build_request(&invalid).is_err());

        let mut invalid = input();
        invalid.source = Some(PageSource::BusinessOwned);
        assert!(build_request(&invalid).is_err());

        let mut invalid = input();
        invalid.page_size = Some(101);
        assert!(build_request(&invalid).is_err());
    }

    #[test]
    fn filters_one_page_without_exposing_raw_provider_data() {
        let request = build_request(&ListPagesInput {
            query: Some("ARMAVITA".to_owned()),
            ..input()
        })
        .unwrap();
        let page = normalize_page(
            json!({
                "data": [
                    {"id": "1", "name": "ArmaVita Health", "fan_count": "42"},
                    {"id": "2", "name": "Different Page", "access_token": "secret"}
                ],
                "paging": {"cursors": {"after": "next"}}
            }),
            &request,
        )
        .unwrap();
        assert_eq!(page.matched_on_page, 1);
        assert_eq!(page.pages[0].id, "1");
        assert_eq!(page.next_cursor.as_deref(), Some("next"));
        assert!(!serde_json::to_string(&page).unwrap().contains("secret"));
    }
}
