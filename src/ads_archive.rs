use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
};

const ARCHIVE_FIELDS: &str = "id,ad_creation_time,ad_creative_bodies,ad_creative_link_captions,ad_creative_link_descriptions,ad_creative_link_titles,ad_delivery_start_time,ad_delivery_stop_time,ad_snapshot_url,page_id,page_name,publisher_platforms,languages";
const DEFAULT_PAGE_SIZE: u16 = 25;
const MAX_PAGE_SIZE: u16 = 100;
const MAX_CURSOR_CHARS: usize = 2_048;
const MAX_COUNTRIES: usize = 25;
const MAX_PAGE_IDS: usize = 10;
const MAX_LANGUAGES: usize = 20;
const MAX_TEXT_ITEMS: usize = 8;
const MAX_CREATIVE_TEXT_CHARS: usize = 1_000;
const MAX_METADATA_TEXT_CHARS: usize = 2_048;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchAdsArchiveInput {
    /// Reached-country filters as two-letter codes, or `ALL` by itself.
    #[schemars(length(min = 1, max = 25))]
    pub ad_reached_countries: Vec<String>,
    /// Search text, at most 100 characters. Required unless page IDs are supplied.
    pub search_terms: Option<String>,
    /// Facebook Page IDs. At most 10. Required unless search text is supplied.
    pub search_page_ids: Option<Vec<String>>,
    /// Defaults to `all`.
    pub ad_type: Option<ArchiveAdType>,
    /// Defaults to Meta's active-only behavior.
    pub ad_active_status: Option<ArchiveActiveStatus>,
    pub search_type: Option<ArchiveSearchType>,
    pub media_type: Option<ArchiveMediaType>,
    pub publisher_platforms: Option<Vec<ArchivePublisherPlatform>>,
    /// ISO language codes, at most 20.
    pub languages: Option<Vec<String>>,
    /// Inclusive minimum delivery date in `YYYY-MM-DD` format.
    pub ad_delivery_date_min: Option<String>,
    /// Inclusive maximum delivery date in `YYYY-MM-DD` format.
    pub ad_delivery_date_max: Option<String>,
    /// Number of ads to return, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveAdType {
    All,
    EmploymentAds,
    FinancialProductsAndServicesAds,
    HousingAds,
    PoliticalAndIssueAds,
}

impl ArchiveAdType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::EmploymentAds => "EMPLOYMENT_ADS",
            Self::FinancialProductsAndServicesAds => "FINANCIAL_PRODUCTS_AND_SERVICES_ADS",
            Self::HousingAds => "HOUSING_ADS",
            Self::PoliticalAndIssueAds => "POLITICAL_AND_ISSUE_ADS",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveActiveStatus {
    Active,
    All,
    Inactive,
}

impl ArchiveActiveStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::All => "ALL",
            Self::Inactive => "INACTIVE",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveSearchType {
    KeywordUnordered,
    KeywordExactPhrase,
}

impl ArchiveSearchType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::KeywordUnordered => "KEYWORD_UNORDERED",
            Self::KeywordExactPhrase => "KEYWORD_EXACT_PHRASE",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveMediaType {
    All,
    Image,
    Meme,
    Video,
    None,
}

impl ArchiveMediaType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::Image => "IMAGE",
            Self::Meme => "MEME",
            Self::Video => "VIDEO",
            Self::None => "NONE",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArchivePublisherPlatform {
    Facebook,
    Instagram,
    AudienceNetwork,
    Messenger,
    Oculus,
    Threads,
}

impl ArchivePublisherPlatform {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Facebook => "FACEBOOK",
            Self::Instagram => "INSTAGRAM",
            Self::AudienceNetwork => "AUDIENCE_NETWORK",
            Self::Messenger => "MESSENGER",
            Self::Oculus => "OCULUS",
            Self::Threads => "THREADS",
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ArchivedAdList {
    pub ads: Vec<ArchivedAd>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ArchivedAd {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ad_creation_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ad_delivery_start_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ad_delivery_stop_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ad_snapshot_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher_platforms: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub languages: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bodies: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_titles: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_descriptions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_captions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "is_false")]
    pub content_truncated: bool,
}

#[derive(Debug, Deserialize)]
struct RawArchivedAdList {
    #[serde(default)]
    data: Vec<RawArchivedAd>,
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
struct RawArchivedAd {
    id: Option<String>,
    page_id: Option<String>,
    page_name: Option<String>,
    ad_creation_time: Option<String>,
    ad_delivery_start_time: Option<String>,
    ad_delivery_stop_time: Option<String>,
    ad_snapshot_url: Option<String>,
    publisher_platforms: Option<Vec<String>>,
    languages: Option<Vec<String>>,
    ad_creative_bodies: Option<Vec<String>>,
    ad_creative_link_titles: Option<Vec<String>>,
    ad_creative_link_descriptions: Option<Vec<String>>,
    ad_creative_link_captions: Option<Vec<String>>,
}

pub(crate) async fn search_ads_archive(
    graph: &GraphClient,
    input: SearchAdsArchiveInput,
) -> ToolResponse<ArchivedAdList> {
    let query = match build_query(&input) {
        Ok(query) => query,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.get_json("ads_archive", &query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    let max_ads = usize::from(input.page_size.unwrap_or(DEFAULT_PAGE_SIZE));
    match parse_archive_page(payload, max_ads) {
        Ok(page) => ToolResponse::success(page),
        Err(error) => ToolResponse::error(error),
    }
}

fn parse_archive_page(payload: Value, max_ads: usize) -> Result<ArchivedAdList, PublicError> {
    let raw = serde_json::from_value::<RawArchivedAdList>(payload).map_err(|_| {
        PublicError::invalid_upstream("Meta returned an unexpected Ads Library result")
    })?;
    if raw.data.len() > max_ads {
        return Err(PublicError::invalid_upstream(
            "Meta returned more Ads Library results than requested",
        ));
    }
    let next_cursor = raw
        .paging
        .and_then(|paging| paging.cursors)
        .and_then(|cursors| cursors.after);
    if next_cursor
        .as_ref()
        .is_some_and(|cursor| cursor.chars().count() > MAX_CURSOR_CHARS)
    {
        return Err(PublicError::invalid_upstream(
            "Meta returned an oversized Ads Library cursor",
        ));
    }
    let ads = raw.data.into_iter().filter_map(normalize_ad).collect();
    Ok(ArchivedAdList { ads, next_cursor })
}

fn build_query(input: &SearchAdsArchiveInput) -> Result<Vec<(String, String)>, PublicError> {
    let countries = clean_countries(&input.ad_reached_countries)?;
    let terms = input
        .search_terms
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if terms.is_some_and(|value| value.chars().count() > 100) {
        return Err(PublicError::invalid_input(
            "search_terms must be at most 100 characters",
            "Use a shorter keyword or exact phrase",
        ));
    }
    let page_ids = clean_page_ids(input.search_page_ids.as_deref())?;
    if terms.is_none() && page_ids.is_empty() {
        return Err(PublicError::invalid_input(
            "search_terms or search_page_ids is required",
            "Provide search text, up to 10 Page IDs, or both",
        ));
    }
    let languages = clean_languages(input.languages.as_deref())?;
    validate_dates(
        input.ad_delivery_date_min.as_deref(),
        input.ad_delivery_date_max.as_deref(),
    )?;
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
        .filter(|value| !value.is_empty());
    if cursor.is_some_and(|value| value.chars().count() > MAX_CURSOR_CHARS) {
        return Err(PublicError::invalid_input(
            "page_cursor is too long",
            "Use next_cursor exactly as returned by the previous response",
        ));
    }

    let mut query = vec![
        ("fields".to_owned(), ARCHIVE_FIELDS.to_owned()),
        ("limit".to_owned(), page_size.to_string()),
        ("ad_reached_countries".to_owned(), encode_array(&countries)?),
        (
            "ad_type".to_owned(),
            input
                .ad_type
                .unwrap_or(ArchiveAdType::All)
                .as_str()
                .to_owned(),
        ),
    ];
    if let Some(terms) = terms {
        query.push(("search_terms".to_owned(), terms.to_owned()));
    }
    if !page_ids.is_empty() {
        query.push(("search_page_ids".to_owned(), encode_array(&page_ids)?));
    }
    if !languages.is_empty() {
        query.push(("languages".to_owned(), encode_array(&languages)?));
    }
    if let Some(status) = input.ad_active_status {
        query.push(("ad_active_status".to_owned(), status.as_str().to_owned()));
    }
    if let Some(search_type) = input.search_type {
        query.push(("search_type".to_owned(), search_type.as_str().to_owned()));
    }
    if let Some(media_type) = input.media_type {
        query.push(("media_type".to_owned(), media_type.as_str().to_owned()));
    }
    if let Some(platforms) = &input.publisher_platforms {
        if platforms.is_empty() || platforms.len() > 7 {
            return Err(PublicError::invalid_input(
                "publisher_platforms must contain between 1 and 7 values",
                "Remove duplicates or omit the filter",
            ));
        }
        let platforms = platforms
            .iter()
            .map(|platform| platform.as_str())
            .collect::<Vec<_>>();
        query.push(("publisher_platforms".to_owned(), encode_array(&platforms)?));
    }
    if let Some(date) = &input.ad_delivery_date_min {
        query.push(("ad_delivery_date_min".to_owned(), date.clone()));
    }
    if let Some(date) = &input.ad_delivery_date_max {
        query.push(("ad_delivery_date_max".to_owned(), date.clone()));
    }
    if let Some(cursor) = cursor {
        query.push(("after".to_owned(), cursor.to_owned()));
    }
    Ok(query)
}

fn clean_countries(values: &[String]) -> Result<Vec<String>, PublicError> {
    if values.is_empty() || values.len() > MAX_COUNTRIES {
        return Err(PublicError::invalid_input(
            "ad_reached_countries must contain between 1 and 25 values",
            "Use two-letter country codes, or `ALL` by itself",
        ));
    }
    let countries = values
        .iter()
        .map(|value| value.trim().to_ascii_uppercase())
        .collect::<Vec<_>>();
    if countries.iter().any(|value| {
        value != "ALL" && (value.len() != 2 || !value.bytes().all(|b| b.is_ascii_alphabetic()))
    }) || (countries.iter().any(|value| value == "ALL") && countries.len() != 1)
    {
        return Err(PublicError::invalid_input(
            "ad_reached_countries contains an invalid value",
            "Use two-letter country codes, or `ALL` by itself",
        ));
    }
    Ok(countries)
}

fn clean_page_ids(values: Option<&[String]>) -> Result<Vec<String>, PublicError> {
    let Some(values) = values else {
        return Ok(Vec::new());
    };
    if values.len() > MAX_PAGE_IDS {
        return Err(PublicError::invalid_input(
            "search_page_ids accepts at most 10 Page IDs",
            "Split the search into multiple bounded requests",
        ));
    }
    values
        .iter()
        .map(|value| {
            let value = value.trim();
            (!value.is_empty()
                && value.len() <= 64
                && value.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| value.to_owned())
            .ok_or_else(|| {
                PublicError::invalid_input(
                    "search_page_ids contains an invalid Page ID",
                    "Use numeric Page IDs without URL or path characters",
                )
            })
        })
        .collect()
}

fn clean_languages(values: Option<&[String]>) -> Result<Vec<String>, PublicError> {
    let Some(values) = values else {
        return Ok(Vec::new());
    };
    if values.len() > MAX_LANGUAGES {
        return Err(PublicError::invalid_input(
            "languages accepts at most 20 values",
            "Use a smaller ISO language-code filter",
        ));
    }
    values
        .iter()
        .map(|value| {
            let value = value.trim();
            (!value.is_empty()
                && value.len() <= 16
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
            .then(|| value.to_owned())
            .ok_or_else(|| {
                PublicError::invalid_input(
                    "languages contains an invalid code",
                    "Use ISO language codes without spaces or path characters",
                )
            })
        })
        .collect()
}

fn validate_dates(minimum: Option<&str>, maximum: Option<&str>) -> Result<(), PublicError> {
    if minimum.is_some_and(|date| !valid_date(date))
        || maximum.is_some_and(|date| !valid_date(date))
        || matches!((minimum, maximum), (Some(minimum), Some(maximum)) if minimum > maximum)
    {
        return Err(PublicError::invalid_input(
            "delivery dates must form a valid inclusive YYYY-MM-DD range",
            "Use real calendar dates with the minimum no later than the maximum",
        ));
    }
    Ok(())
}

fn valid_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| !matches!(index, 4 | 7) && !byte.is_ascii_digit())
    {
        return false;
    }
    let year = u16::from(bytes[0] - b'0') * 1_000
        + u16::from(bytes[1] - b'0') * 100
        + u16::from(bytes[2] - b'0') * 10
        + u16::from(bytes[3] - b'0');
    let month = (bytes[5] - b'0') * 10 + (bytes[6] - b'0');
    let day = (bytes[8] - b'0') * 10 + (bytes[9] - b'0');
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=max_day).contains(&day)
}

fn encode_array<T: Serialize>(values: &[T]) -> Result<String, PublicError> {
    serde_json::to_string(values).map_err(|_| {
        PublicError::invalid_input(
            "a search filter could not be encoded",
            "Use plain IDs, codes, and enum values",
        )
    })
}

fn normalize_ad(raw: RawArchivedAd) -> Option<ArchivedAd> {
    let id = clean_id(raw.id?)?;
    let (bodies, bodies_truncated) = bounded_texts(raw.ad_creative_bodies);
    let (link_titles, titles_truncated) = bounded_texts(raw.ad_creative_link_titles);
    let (link_descriptions, descriptions_truncated) =
        bounded_texts(raw.ad_creative_link_descriptions);
    let (link_captions, captions_truncated) = bounded_texts(raw.ad_creative_link_captions);
    Some(ArchivedAd {
        id,
        page_id: raw.page_id.and_then(clean_id),
        page_name: bounded_text(raw.page_name, MAX_METADATA_TEXT_CHARS).0,
        ad_creation_time: bounded_text(raw.ad_creation_time, 64).0,
        ad_delivery_start_time: bounded_text(raw.ad_delivery_start_time, 64).0,
        ad_delivery_stop_time: bounded_text(raw.ad_delivery_stop_time, 64).0,
        ad_snapshot_url: bounded_text(raw.ad_snapshot_url, MAX_METADATA_TEXT_CHARS).0,
        publisher_platforms: bounded_short_texts(raw.publisher_platforms),
        languages: bounded_short_texts(raw.languages),
        bodies,
        link_titles,
        link_descriptions,
        link_captions,
        content_truncated: bodies_truncated
            || titles_truncated
            || descriptions_truncated
            || captions_truncated,
    })
}

fn clean_id(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty() && value.len() <= 64 && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.to_owned())
}

fn bounded_text(value: Option<String>, limit: usize) -> (Option<String>, bool) {
    let Some(value) = value else {
        return (None, false);
    };
    let value = value.trim();
    if value.is_empty() {
        return (None, false);
    }
    let mut bounded = value.chars().take(limit).collect::<String>();
    let truncated = value.chars().count() > limit;
    if truncated {
        bounded.push('…');
    }
    (Some(bounded), truncated)
}

fn bounded_texts(values: Option<Vec<String>>) -> (Option<Vec<String>>, bool) {
    let Some(values) = values else {
        return (None, false);
    };
    let mut truncated = values.len() > MAX_TEXT_ITEMS;
    let values = values
        .into_iter()
        .take(MAX_TEXT_ITEMS)
        .filter_map(|value| {
            let (value, was_truncated) = bounded_text(Some(value), MAX_CREATIVE_TEXT_CHARS);
            truncated |= was_truncated;
            value
        })
        .collect::<Vec<_>>();
    ((!values.is_empty()).then_some(values), truncated)
}

fn bounded_short_texts(values: Option<Vec<String>>) -> Option<Vec<String>> {
    let values = values?
        .into_iter()
        .take(20)
        .filter_map(|value| bounded_text(Some(value), 64).0)
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

const fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        ArchiveAdType, SearchAdsArchiveInput, build_query, clean_countries, normalize_ad,
        parse_archive_page,
    };

    fn input() -> SearchAdsArchiveInput {
        SearchAdsArchiveInput {
            ad_reached_countries: vec!["US".to_owned()],
            search_terms: Some("efficient creative".to_owned()),
            search_page_ids: None,
            ad_type: Some(ArchiveAdType::All),
            ad_active_status: None,
            search_type: None,
            media_type: None,
            publisher_platforms: None,
            languages: None,
            ad_delivery_date_min: None,
            ad_delivery_date_max: None,
            page_size: None,
            page_cursor: None,
        }
    }

    #[test]
    fn builds_a_bounded_current_archive_query() {
        let query = build_query(&input()).unwrap();
        assert!(query.contains(&("limit".to_owned(), "25".to_owned())));
        assert!(query.contains(&("ad_reached_countries".to_owned(), "[\"US\"]".to_owned())));
        assert!(query.contains(&("ad_type".to_owned(), "ALL".to_owned())));
        assert!(!query.iter().any(|(key, _)| key == "access_token"));
    }

    #[test]
    fn rejects_ambiguous_or_unbounded_searches() {
        let mut missing_search = input();
        missing_search.search_terms = None;
        assert!(build_query(&missing_search).is_err());

        let mut long_terms = input();
        long_terms.search_terms = Some("x".repeat(101));
        assert!(build_query(&long_terms).is_err());

        assert!(clean_countries(&["ALL".to_owned(), "US".to_owned()]).is_err());
        assert!(clean_countries(&["u/s".to_owned()]).is_err());
    }

    #[test]
    fn normalizes_and_marks_truncated_creative_text() {
        let ad = normalize_ad(
            serde_json::from_value(json!({
                "id": "123",
                "page_id": "456",
                "page_name": " Example ",
                "ad_creative_bodies": ["x".repeat(1001)],
                "publisher_platforms": ["FACEBOOK"]
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(ad.page_name.as_deref(), Some("Example"));
        assert!(ad.content_truncated);
        assert!(ad.bodies.unwrap()[0].ends_with('…'));
    }

    #[test]
    fn rejects_an_upstream_page_larger_than_requested() {
        let payload = json!({"data": [{"id": "1"}, {"id": "2"}]});
        assert!(parse_archive_page(payload, 1).is_err());
    }
}
