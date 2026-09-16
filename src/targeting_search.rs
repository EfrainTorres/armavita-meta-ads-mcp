use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::numeric_value as meta_numeric_value,
};

const DEFAULT_PAGE_SIZE: u16 = 25;
const MAX_PAGE_SIZE: u16 = 100;
const MAX_CURSOR_CHARS: usize = 2_048;
const MAX_QUERY_CHARS: usize = 100;
const MAX_INTEREST_TERMS: usize = 25;
const MAX_TERM_CHARS: usize = 100;
const MAX_KEY_CHARS: usize = 128;
const MAX_NAME_CHARS: usize = 256;
const MAX_DESCRIPTION_CHARS: usize = 512;
const MAX_METADATA_CHARS: usize = 128;
const MAX_PATH_ITEMS: usize = 16;
const MAX_PATH_ITEM_CHARS: usize = 128;
const MAX_COUNTRY_CODES: usize = 250;

type SearchRequest = (Vec<(String, String)>, usize);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchInterestsInput {
    /// Case-sensitive interest text to autocomplete, from 1 through 100 characters.
    pub query: String,
    /// Number of interests to return, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuggestInterestsInput {
    /// Case-sensitive interest names used as suggestion seeds. Accepts 1 through 25 terms.
    #[schemars(length(min = 1, max = 25), inner(length(min = 1, max = 100)))]
    pub interest_list: Vec<String>,
    /// Number of suggestions to return, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchBehaviorsInput {
    /// Number of behavior categories to return, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchDemographicsInput {
    /// Defaults to `demographics`, which retrieves all documented demographic classes.
    pub demographic_class: Option<DemographicClass>,
    /// Number of demographic categories to return, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DemographicClass {
    Demographics,
    LifeEvents,
    Industries,
    Income,
    FamilyStatuses,
    UserDevice,
    UserOs,
}

impl DemographicClass {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Demographics => "demographics",
            Self::LifeEvents => "life_events",
            Self::Industries => "industries",
            Self::Income => "income",
            Self::FamilyStatuses => "family_statuses",
            Self::UserDevice => "user_device",
            Self::UserOs => "user_os",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchGeoLocationsInput {
    /// Location text to autocomplete, from 1 through 100 characters.
    pub query: String,
    /// Optional documented location categories. Omit to search all categories.
    pub location_types: Option<Vec<GeoLocationType>>,
    /// Number of locations to return, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GeoLocationType {
    Country,
    CountryGroup,
    Region,
    City,
    Zip,
    GeoMarket,
    ElectoralDistrict,
}

impl GeoLocationType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Country => "country",
            Self::CountryGroup => "country_group",
            Self::Region => "region",
            Self::City => "city",
            Self::Zip => "zip",
            Self::GeoMarket => "geo_market",
            Self::ElectoralDistrict => "electoral_district",
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct InterestPage {
    pub interests: Vec<Interest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// A targetable interest. Audience estimates are independent bounds and are never summed.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Interest {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience_size_lower_bound: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience_size_upper_bound: Option<u64>,
    #[serde(skip_serializing_if = "is_false")]
    pub content_truncated: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TargetingCategoryPage {
    pub categories: Vec<TargetingCategory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// A documented behavior or demographic targeting category.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TargetingCategory {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience_size_lower_bound: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience_size_upper_bound: Option<u64>,
    #[serde(skip_serializing_if = "is_false")]
    pub content_truncated: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GeoLocationPage {
    pub locations: Vec<GeoLocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Compact targeting identity and hierarchy metadata. `key`, not `name`, is the stable selector.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GeoLocation {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_codes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_city_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geo_hierarchy_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geo_hierarchy_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_worldwide: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_region: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_city: Option<bool>,
    #[serde(skip_serializing_if = "is_false")]
    pub content_truncated: bool,
}

#[derive(Deserialize)]
struct RawPage<T> {
    data: Option<Vec<T>>,
    paging: Option<RawPaging>,
}

#[derive(Deserialize)]
struct RawPaging {
    cursors: Option<RawCursors>,
}

#[derive(Deserialize)]
struct RawCursors {
    after: Option<String>,
}

#[derive(Deserialize)]
struct RawInterest {
    id: Option<Value>,
    name: Option<String>,
    locale: Option<String>,
    description: Option<String>,
    path: Option<Vec<String>>,
    audience_size_lower_bound: Option<Value>,
    audience_size_upper_bound: Option<Value>,
}

#[derive(Deserialize)]
struct RawTargetingCategory {
    id: Option<Value>,
    name: Option<String>,
    #[serde(rename = "type")]
    category_type: Option<String>,
    description: Option<String>,
    path: Option<Vec<String>>,
    audience_size_lower_bound: Option<Value>,
    audience_size_upper_bound: Option<Value>,
}

#[derive(Deserialize)]
struct RawGeoLocation {
    key: Option<Value>,
    name: Option<String>,
    #[serde(rename = "type")]
    location_type: Option<String>,
    country_code: Option<String>,
    country_codes: Option<Vec<String>>,
    region: Option<String>,
    region_id: Option<Value>,
    primary_city: Option<String>,
    primary_city_id: Option<Value>,
    geo_hierarchy_level: Option<String>,
    geo_hierarchy_name: Option<String>,
    is_worldwide: Option<bool>,
    supports_region: Option<bool>,
    supports_city: Option<bool>,
}

pub(crate) async fn search_interests(
    graph: &GraphClient,
    input: SearchInterestsInput,
) -> ToolResponse<InterestPage> {
    let (query, max_rows) = match build_search_interests_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    request_page(graph, query, max_rows, parse_interest_page).await
}

pub(crate) async fn suggest_interests(
    graph: &GraphClient,
    input: SuggestInterestsInput,
) -> ToolResponse<InterestPage> {
    let (query, max_rows) = match build_suggest_interests_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    request_page(graph, query, max_rows, parse_interest_page).await
}

pub(crate) async fn search_behaviors(
    graph: &GraphClient,
    input: SearchBehaviorsInput,
) -> ToolResponse<TargetingCategoryPage> {
    let (query, max_rows) = match build_behavior_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    request_page(graph, query, max_rows, parse_category_page).await
}

pub(crate) async fn search_demographics(
    graph: &GraphClient,
    input: SearchDemographicsInput,
) -> ToolResponse<TargetingCategoryPage> {
    let (query, max_rows) = match build_demographic_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    request_page(graph, query, max_rows, parse_category_page).await
}

pub(crate) async fn search_geo_locations(
    graph: &GraphClient,
    input: SearchGeoLocationsInput,
) -> ToolResponse<GeoLocationPage> {
    let (query, max_rows) = match build_geo_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    request_page(graph, query, max_rows, parse_geo_page).await
}

async fn request_page<T>(
    graph: &GraphClient,
    query: Vec<(String, String)>,
    max_rows: usize,
    parse: fn(Value, usize) -> Result<T, PublicError>,
) -> ToolResponse<T> {
    let payload = match graph.get_json("search", &query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    match parse(payload, max_rows) {
        Ok(page) => ToolResponse::success(page),
        Err(error) => ToolResponse::error(error),
    }
}

fn build_search_interests_request(
    input: &SearchInterestsInput,
) -> Result<SearchRequest, PublicError> {
    let query_text = clean_query(&input.query)?;
    let mut query = vec![
        ("type".to_owned(), "adinterest".to_owned()),
        ("q".to_owned(), query_text),
    ];
    let max_rows = append_paging(&mut query, input.page_size, input.page_cursor.as_deref())?;
    Ok((query, max_rows))
}

fn build_suggest_interests_request(
    input: &SuggestInterestsInput,
) -> Result<SearchRequest, PublicError> {
    let interests = clean_interest_list(&input.interest_list)?;
    let encoded = serde_json::to_string(&interests).map_err(|_| {
        PublicError::invalid_input(
            "interest_list could not be encoded",
            "Use plain case-sensitive interest names",
        )
    })?;
    let mut query = vec![
        ("type".to_owned(), "adinterestsuggestion".to_owned()),
        ("interest_list".to_owned(), encoded),
    ];
    let max_rows = append_paging(&mut query, input.page_size, input.page_cursor.as_deref())?;
    Ok((query, max_rows))
}

fn build_behavior_request(input: &SearchBehaviorsInput) -> Result<SearchRequest, PublicError> {
    let mut query = vec![
        ("type".to_owned(), "adTargetingCategory".to_owned()),
        ("class".to_owned(), "behaviors".to_owned()),
    ];
    let max_rows = append_paging(&mut query, input.page_size, input.page_cursor.as_deref())?;
    Ok((query, max_rows))
}

fn build_demographic_request(
    input: &SearchDemographicsInput,
) -> Result<SearchRequest, PublicError> {
    let demographic_class = input
        .demographic_class
        .unwrap_or(DemographicClass::Demographics);
    let mut query = vec![
        ("type".to_owned(), "adTargetingCategory".to_owned()),
        ("class".to_owned(), demographic_class.as_str().to_owned()),
    ];
    let max_rows = append_paging(&mut query, input.page_size, input.page_cursor.as_deref())?;
    Ok((query, max_rows))
}

fn build_geo_request(input: &SearchGeoLocationsInput) -> Result<SearchRequest, PublicError> {
    let query_text = clean_query(&input.query)?;
    let mut query = vec![
        ("type".to_owned(), "adgeolocation".to_owned()),
        ("q".to_owned(), query_text),
    ];
    if let Some(location_types) = input.location_types.as_deref() {
        if location_types.is_empty() || location_types.len() > 7 {
            return Err(PublicError::invalid_input(
                "location_types must contain between 1 and 7 values",
                "Use documented location categories or omit the filter",
            ));
        }
        let mut values = Vec::with_capacity(location_types.len());
        for location_type in location_types {
            let value = location_type.as_str();
            if !values.contains(&value) {
                values.push(value);
            }
        }
        let encoded = serde_json::to_string(&values).map_err(|_| {
            PublicError::invalid_input(
                "location_types could not be encoded",
                "Use only documented location categories",
            )
        })?;
        query.push(("location_types".to_owned(), encoded));
    }
    let max_rows = append_paging(&mut query, input.page_size, input.page_cursor.as_deref())?;
    Ok((query, max_rows))
}

fn append_paging(
    query: &mut Vec<(String, String)>,
    page_size: Option<u16>,
    page_cursor: Option<&str>,
) -> Result<usize, PublicError> {
    let page_size = page_size.unwrap_or(DEFAULT_PAGE_SIZE);
    if !(1..=MAX_PAGE_SIZE).contains(&page_size) {
        return Err(PublicError::invalid_input(
            "page_size must be between 1 and 100",
            "Choose a bounded page_size and paginate with next_cursor",
        ));
    }
    query.push(("limit".to_owned(), page_size.to_string()));

    if let Some(cursor) = page_cursor {
        let cursor = cursor.trim();
        if cursor.is_empty() {
            return Err(PublicError::invalid_input(
                "page_cursor must not be empty",
                "Omit page_cursor or use next_cursor exactly as returned",
            ));
        }
        if cursor.chars().count() > MAX_CURSOR_CHARS {
            return Err(PublicError::invalid_input(
                "page_cursor is too long",
                "Use next_cursor exactly as returned by the previous response",
            ));
        }
        query.push(("after".to_owned(), cursor.to_owned()));
    }

    Ok(usize::from(page_size))
}

fn clean_query(value: &str) -> Result<String, PublicError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > MAX_QUERY_CHARS {
        return Err(PublicError::invalid_input(
            "query must contain between 1 and 100 characters",
            "Use a short interest or location search phrase",
        ));
    }
    Ok(value.to_owned())
}

fn clean_interest_list(values: &[String]) -> Result<Vec<String>, PublicError> {
    if values.is_empty() || values.len() > MAX_INTEREST_TERMS {
        return Err(PublicError::invalid_input(
            "interest_list must contain between 1 and 25 terms",
            "Use a smaller list of case-sensitive interest names",
        ));
    }

    let mut interests = Vec::with_capacity(values.len());
    for value in values {
        let value = value.trim();
        if value.is_empty() || value.chars().count() > MAX_TERM_CHARS {
            return Err(PublicError::invalid_input(
                "each interest term must contain between 1 and 100 characters",
                "Remove empty values and shorten long interest names",
            ));
        }
        if !interests.iter().any(|interest| interest == value) {
            interests.push(value.to_owned());
        }
    }
    Ok(interests)
}

fn parse_interest_page(payload: Value, max_rows: usize) -> Result<InterestPage, PublicError> {
    let (rows, next_cursor) = decode_page::<RawInterest>(payload, max_rows, "interest")?;
    let interests = rows
        .into_iter()
        .map(normalize_interest)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| PublicError::invalid_upstream("Meta omitted a valid interest ID"))?;
    Ok(InterestPage {
        interests,
        next_cursor,
    })
}

fn parse_category_page(
    payload: Value,
    max_rows: usize,
) -> Result<TargetingCategoryPage, PublicError> {
    let (rows, next_cursor) =
        decode_page::<RawTargetingCategory>(payload, max_rows, "targeting-category")?;
    let categories = rows
        .into_iter()
        .map(normalize_category)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            PublicError::invalid_upstream("Meta omitted a valid targeting-category ID")
        })?;
    Ok(TargetingCategoryPage {
        categories,
        next_cursor,
    })
}

fn parse_geo_page(payload: Value, max_rows: usize) -> Result<GeoLocationPage, PublicError> {
    let (rows, next_cursor) = decode_page::<RawGeoLocation>(payload, max_rows, "geo-location")?;
    let locations = rows
        .into_iter()
        .map(normalize_geo_location)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| PublicError::invalid_upstream("Meta omitted a valid geo-location key"))?;
    Ok(GeoLocationPage {
        locations,
        next_cursor,
    })
}

fn decode_page<T: DeserializeOwned>(
    payload: Value,
    max_rows: usize,
    kind: &str,
) -> Result<(Vec<T>, Option<String>), PublicError> {
    let raw = serde_json::from_value::<RawPage<T>>(payload).map_err(|_| {
        PublicError::invalid_upstream(format!("Meta returned an unexpected {kind} search page"))
    })?;
    let rows = raw
        .data
        .ok_or_else(|| PublicError::invalid_upstream(format!("Meta omitted {kind} search data")))?;
    if rows.len() > max_rows {
        return Err(PublicError::invalid_upstream(format!(
            "Meta returned more {kind} results than requested"
        )));
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
        return Err(PublicError::invalid_upstream(format!(
            "Meta returned an oversized {kind} cursor"
        )));
    }

    Ok((rows, next_cursor))
}

fn normalize_interest(raw: RawInterest) -> Option<Interest> {
    let id = normalize_numeric_value(raw.id)?;
    let (name, name_truncated) = bounded_text(raw.name, MAX_NAME_CHARS);
    let (locale, locale_truncated) = bounded_text(raw.locale, 32);
    let (description, description_truncated) = bounded_text(raw.description, MAX_DESCRIPTION_CHARS);
    let (path, path_truncated) = bounded_text_list(raw.path, MAX_PATH_ITEMS, MAX_PATH_ITEM_CHARS);
    let (lower, upper) =
        normalize_audience_bounds(raw.audience_size_lower_bound, raw.audience_size_upper_bound);
    Some(Interest {
        id,
        name,
        locale,
        description,
        path,
        audience_size_lower_bound: lower,
        audience_size_upper_bound: upper,
        content_truncated: name_truncated
            || locale_truncated
            || description_truncated
            || path_truncated,
    })
}

fn normalize_category(raw: RawTargetingCategory) -> Option<TargetingCategory> {
    let id = normalize_numeric_value(raw.id)?;
    let (name, name_truncated) = bounded_text(raw.name, MAX_NAME_CHARS);
    let (category_type, type_truncated) = bounded_text(raw.category_type, MAX_METADATA_CHARS);
    let (description, description_truncated) = bounded_text(raw.description, MAX_DESCRIPTION_CHARS);
    let (path, path_truncated) = bounded_text_list(raw.path, MAX_PATH_ITEMS, MAX_PATH_ITEM_CHARS);
    let (lower, upper) =
        normalize_audience_bounds(raw.audience_size_lower_bound, raw.audience_size_upper_bound);
    Some(TargetingCategory {
        id,
        name,
        category_type,
        description,
        path,
        audience_size_lower_bound: lower,
        audience_size_upper_bound: upper,
        content_truncated: name_truncated
            || type_truncated
            || description_truncated
            || path_truncated,
    })
}

fn normalize_geo_location(raw: RawGeoLocation) -> Option<GeoLocation> {
    let key = normalize_geo_key(raw.key)?;
    let (name, name_truncated) = bounded_text(raw.name, MAX_NAME_CHARS);
    let (location_type, type_truncated) = bounded_text(raw.location_type, MAX_METADATA_CHARS);
    let (country_codes, codes_truncated) = clean_country_codes(raw.country_codes);
    let (region, region_truncated) = bounded_text(raw.region, MAX_NAME_CHARS);
    let (primary_city, city_truncated) = bounded_text(raw.primary_city, MAX_NAME_CHARS);
    let (geo_hierarchy_level, level_truncated) =
        bounded_text(raw.geo_hierarchy_level, MAX_METADATA_CHARS);
    let (geo_hierarchy_name, hierarchy_truncated) =
        bounded_text(raw.geo_hierarchy_name, MAX_METADATA_CHARS);
    Some(GeoLocation {
        key,
        name,
        location_type,
        country_code: raw.country_code.and_then(clean_country_code),
        country_codes,
        region,
        region_id: normalize_numeric_value(raw.region_id),
        primary_city,
        primary_city_id: normalize_numeric_value(raw.primary_city_id),
        geo_hierarchy_level,
        geo_hierarchy_name,
        is_worldwide: raw.is_worldwide,
        supports_region: raw.supports_region,
        supports_city: raw.supports_city,
        content_truncated: name_truncated
            || type_truncated
            || codes_truncated
            || region_truncated
            || city_truncated
            || level_truncated
            || hierarchy_truncated,
    })
}

fn normalize_audience_bounds(
    lower: Option<Value>,
    upper: Option<Value>,
) -> (Option<u64>, Option<u64>) {
    let lower = normalize_nonnegative_integer(lower);
    let upper = normalize_nonnegative_integer(upper);
    if matches!((lower, upper), (Some(lower), Some(upper)) if lower > upper) {
        return (None, None);
    }
    (lower, upper)
}

fn normalize_nonnegative_integer(value: Option<Value>) -> Option<u64> {
    match value? {
        Value::Number(value) => value.as_u64(),
        Value::String(value) => value.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn normalize_numeric_value(value: Option<Value>) -> Option<String> {
    meta_numeric_value(&value?)
}

fn normalize_geo_key(value: Option<Value>) -> Option<String> {
    let value = match value? {
        Value::Number(value) => value.as_u64()?.to_string(),
        Value::String(value) => value,
        _ => return None,
    };
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= MAX_KEY_CHARS
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-' | b'.')))
    .then(|| value.to_owned())
}

fn clean_country_code(value: String) -> Option<String> {
    let value = value.trim().to_ascii_uppercase();
    (value.len() == 2 && value.bytes().all(|byte| byte.is_ascii_alphabetic())).then_some(value)
}

fn clean_country_codes(values: Option<Vec<String>>) -> (Option<Vec<String>>, bool) {
    let Some(values) = values else {
        return (None, false);
    };
    let mut truncated = values.len() > MAX_COUNTRY_CODES;
    let mut codes = Vec::with_capacity(values.len().min(MAX_COUNTRY_CODES));
    for value in values.into_iter().take(MAX_COUNTRY_CODES) {
        if let Some(value) = clean_country_code(value) {
            if !codes.contains(&value) {
                codes.push(value);
            }
        } else {
            truncated = true;
        }
    }
    ((!codes.is_empty()).then_some(codes), truncated)
}

fn bounded_text(value: Option<String>, max_chars: usize) -> (Option<String>, bool) {
    let Some(value) = value else {
        return (None, false);
    };
    let value = value.trim();
    if value.is_empty() {
        return (None, false);
    }

    let mut chars = value.chars();
    let mut output = chars.by_ref().take(max_chars).collect::<String>();
    let truncated = chars.next().is_some();
    if truncated {
        output.pop();
        output.push('…');
    }
    (Some(output), truncated)
}

fn bounded_text_list(
    values: Option<Vec<String>>,
    max_items: usize,
    max_chars: usize,
) -> (Option<Vec<String>>, bool) {
    let Some(values) = values else {
        return (None, false);
    };
    let mut truncated = values.len() > max_items;
    let values = values
        .into_iter()
        .take(max_items)
        .filter_map(|value| {
            let (value, value_truncated) = bounded_text(Some(value), max_chars);
            truncated |= value_truncated;
            value
        })
        .collect::<Vec<_>>();
    ((!values.is_empty()).then_some(values), truncated)
}

const fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        DemographicClass, GeoLocationType, SearchBehaviorsInput, SearchDemographicsInput,
        SearchGeoLocationsInput, SearchInterestsInput, SuggestInterestsInput,
        build_behavior_request, build_demographic_request, build_geo_request,
        build_search_interests_request, build_suggest_interests_request, parse_category_page,
        parse_geo_page, parse_interest_page,
    };

    fn value<'a>(query: &'a [(String, String)], name: &str) -> Option<&'a str> {
        query
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn builds_only_documented_v26_global_search_parameters() {
        let (interest, _) = build_search_interests_request(&SearchInterestsInput {
            query: " repair ".to_owned(),
            page_size: Some(20),
            page_cursor: Some("cursor_5".to_owned()),
        })
        .unwrap();
        assert_eq!(value(&interest, "type"), Some("adinterest"));
        assert_eq!(value(&interest, "q"), Some("repair"));
        assert_eq!(value(&interest, "limit"), Some("20"));
        assert_eq!(value(&interest, "after"), Some("cursor_5"));

        let (suggestions, _) = build_suggest_interests_request(&SuggestInterestsInput {
            interest_list: vec!["Basketball".to_owned(), "Basketball".to_owned()],
            page_size: None,
            page_cursor: None,
        })
        .unwrap();
        assert_eq!(value(&suggestions, "type"), Some("adinterestsuggestion"));
        assert_eq!(
            value(&suggestions, "interest_list"),
            Some("[\"Basketball\"]")
        );

        let (behaviors, _) = build_behavior_request(&SearchBehaviorsInput {
            page_size: None,
            page_cursor: None,
        })
        .unwrap();
        assert_eq!(value(&behaviors, "type"), Some("adTargetingCategory"));
        assert_eq!(value(&behaviors, "class"), Some("behaviors"));

        let (demographics, _) = build_demographic_request(&SearchDemographicsInput {
            demographic_class: Some(DemographicClass::LifeEvents),
            page_size: None,
            page_cursor: None,
        })
        .unwrap();
        assert_eq!(value(&demographics, "class"), Some("life_events"));

        let (geo, _) = build_geo_request(&SearchGeoLocationsInput {
            query: "Austin".to_owned(),
            location_types: Some(vec![GeoLocationType::City, GeoLocationType::City]),
            page_size: None,
            page_cursor: None,
        })
        .unwrap();
        assert_eq!(value(&geo, "type"), Some("adgeolocation"));
        assert_eq!(value(&geo, "location_types"), Some("[\"city\"]"));

        for query in [&interest, &suggestions, &behaviors, &demographics, &geo] {
            assert!(!query.iter().any(|(key, _)| key == "access_token"));
            assert!(!query.iter().any(|(key, _)| key == "page_size"));
            assert!(!query.iter().any(|(key, _)| key == "page_cursor"));
        }
    }

    #[test]
    fn rejects_empty_or_unbounded_search_inputs() {
        assert!(
            build_search_interests_request(&SearchInterestsInput {
                query: " ".to_owned(),
                page_size: None,
                page_cursor: None,
            })
            .is_err()
        );
        assert!(
            build_search_interests_request(&SearchInterestsInput {
                query: "x".repeat(101),
                page_size: None,
                page_cursor: None,
            })
            .is_err()
        );
        assert!(
            build_suggest_interests_request(&SuggestInterestsInput {
                interest_list: Vec::new(),
                page_size: None,
                page_cursor: None,
            })
            .is_err()
        );
        assert!(
            build_behavior_request(&SearchBehaviorsInput {
                page_size: Some(101),
                page_cursor: None,
            })
            .is_err()
        );
        assert!(
            build_behavior_request(&SearchBehaviorsInput {
                page_size: None,
                page_cursor: Some("x".repeat(2_049)),
            })
            .is_err()
        );
        assert!(
            build_geo_request(&SearchGeoLocationsInput {
                query: "US".to_owned(),
                location_types: Some(Vec::new()),
                page_size: None,
                page_cursor: None,
            })
            .is_err()
        );
    }

    #[test]
    fn maps_compact_interest_and_category_pages_without_provider_blobs() {
        let interests = parse_interest_page(
            json!({
                "data": [{
                    "id": 6003598240487_u64,
                    "name": "Basketball",
                    "locale": "en_US",
                    "path": ["Sports"],
                    "audience_size_lower_bound": "1000",
                    "audience_size_upper_bound": 1200,
                    "audience_size": 999999,
                    "url": "https://provider.example/secret?access_token=token"
                }],
                "paging": {
                    "cursors": {"after": "opaque-next"},
                    "next": "https://graph.facebook.com/search?access_token=secret"
                }
            }),
            1,
        )
        .unwrap();
        assert_eq!(interests.interests[0].id, "6003598240487");
        assert_eq!(interests.interests[0].audience_size_lower_bound, Some(1000));
        assert_eq!(interests.next_cursor.as_deref(), Some("opaque-next"));

        let categories = parse_category_page(
            json!({
                "data": [{
                    "id": "42",
                    "name": "Frequent travelers",
                    "type": "behaviors",
                    "description": "Travel behavior",
                    "path": ["Behaviors", "Travel"],
                    "audience_size_lower_bound": 100,
                    "audience_size_upper_bound": 200,
                    "provider_blob": {"token": "secret"}
                }]
            }),
            1,
        )
        .unwrap();
        assert_eq!(
            categories.categories[0].category_type.as_deref(),
            Some("behaviors")
        );

        let output = format!(
            "{}{}",
            serde_json::to_string(&interests).unwrap(),
            serde_json::to_string(&categories).unwrap()
        );
        assert!(!output.contains("provider.example"));
        assert!(!output.contains("access_token"));
        assert!(!output.contains("provider_blob"));
        assert!(!output.contains("audience_size\""));
    }

    #[test]
    fn maps_documented_geo_identity_and_marks_bounded_content() {
        let page = parse_geo_page(
            json!({
                "data": [{
                    "key": "US:90028",
                    "name": "x".repeat(300),
                    "type": "zip",
                    "country_code": "us",
                    "region": "California",
                    "region_id": 3847,
                    "primary_city": "Los Angeles",
                    "primary_city_id": "2420379",
                    "supports_region": true,
                    "supports_city": true,
                    "raw_url": "https://provider.example/raw"
                }]
            }),
            1,
        )
        .unwrap();
        let location = &page.locations[0];
        assert_eq!(location.key, "US:90028");
        assert_eq!(location.country_code.as_deref(), Some("US"));
        assert_eq!(location.region_id.as_deref(), Some("3847"));
        assert!(location.content_truncated);
        assert!(!serde_json::to_string(&page).unwrap().contains("raw_url"));
    }

    #[test]
    fn rejects_oversized_upstream_pages_and_cursors() {
        assert!(parse_interest_page(json!({"data": [{"id": "1"}, {"id": "2"}]}), 1,).is_err());
        assert!(
            parse_geo_page(
                json!({
                    "data": [],
                    "paging": {"cursors": {"after": "x".repeat(2_049)}}
                }),
                1,
            )
            .is_err()
        );
        assert!(parse_category_page(json!({"data": [{"name": "missing id"}]}), 1).is_err());
    }
}
