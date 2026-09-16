use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{
        ad_account as normalize_ad_account_id, numeric_owned as normalize_numeric_id,
        numeric_value as meta_numeric_value,
    },
};

// Fields verified against Meta's generated Business SDK v26.0. Large catalog settings,
// product descriptions/media arrays, and conversion data-source objects stay out of the
// model-facing default contract.
const CATALOG_FIELDS: &str =
    "id,name,vertical,product_count,feed_count,is_catalog_segment,is_local_catalog";
const PRODUCT_FIELDS: &str = "id,retailer_id,name,price,currency,availability,condition,url,image_url,brand,category,live_special_price,review_status";
const PRODUCT_SET_FIELDS: &str = "id,name,retailer_id,product_count,filter,auto_creation_url";
const CUSTOM_CONVERSION_LIST_FIELDS: &str = "id,name,custom_event_type,event_source_id,event_source_type,action_source_type,is_archived,is_unavailable,creation_time,last_fired_time,default_conversion_value,retention_days";
const CUSTOM_CONVERSION_DETAIL_FIELDS: &str = "id,name,description,custom_event_type,event_source_id,event_source_type,action_source_type,is_archived,is_unavailable,creation_time,first_fired_time,last_fired_time,default_conversion_value,retention_days,rule,advanced_rule";
const DATASET_FIELDS: &str = "id,name,creation_time,last_fired_time,is_unavailable";

const DEFAULT_PAGE_SIZE: u16 = 25;
const MAX_PAGE_SIZE: u16 = 100;
const MAX_CURSOR_CHARS: usize = 2_048;
const MAX_NAME_CHARS: usize = 256;
const MAX_IDENTIFIER_CHARS: usize = 256;
const MAX_METADATA_CHARS: usize = 128;
const MAX_URL_CHARS: usize = 2_048;
const MAX_RULE_CHARS: usize = 2_048;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListProductCatalogsInput {
    /// Numeric Meta business ID.
    pub business_id: String,
    /// Number of catalogs to return, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListProductsInput {
    /// Numeric Meta product-catalog ID.
    pub product_catalog_id: String,
    /// Number of products to return, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
    /// When true, ask Meta to return only approved products.
    pub return_only_approved_products: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListProductSetsInput {
    /// Numeric Meta product-catalog ID.
    pub product_catalog_id: String,
    /// Number of product sets to return, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListCustomConversionsInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    pub ad_account_id: String,
    /// Number of custom conversions to return, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadCustomConversionInput {
    /// Numeric Meta custom-conversion ID.
    pub custom_conversion_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListBusinessDatasetsInput {
    /// Numeric Meta business ID.
    pub business_id: String,
    /// Number of datasets to return, from 1 through 100. Defaults to 25.
    pub page_size: Option<u16>,
    /// Opaque `next_cursor` from the previous response.
    pub page_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ProductCatalogList {
    pub catalogs: Vec<ProductCatalog>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ProductCatalog {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feed_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_catalog_segment: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_local_catalog: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ProductList {
    pub products: Vec<Product>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Compact catalog item. Descriptions, secondary images, and videos are deliberately absent.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Product {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retailer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub availability: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_special_price: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ProductSetList {
    pub product_sets: Vec<ProductSet>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ProductSet {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retailer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_creation_url: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CustomConversionList {
    pub custom_conversions: Vec<CustomConversion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CustomConversion {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_event_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_source_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_source_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_source_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_archived: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_unavailable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_fired_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fired_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_conversion_value: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advanced_rule: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advanced_rule_truncated: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BusinessDatasetList {
    pub datasets: Vec<BusinessDataset>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Business Ads Dataset metadata. Event payloads and quality details are absent.
#[derive(Debug, Serialize, JsonSchema)]
pub struct BusinessDataset {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fired_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_unavailable: Option<bool>,
}

#[derive(Debug)]
struct Page {
    size: u16,
    cursor: Option<String>,
}

#[derive(Debug)]
struct ListRequest {
    endpoint: String,
    query: Vec<(String, String)>,
    page_size: u16,
}

#[derive(Debug, Deserialize)]
struct RawPage {
    #[serde(default)]
    data: Vec<Value>,
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
struct RawProductCatalog {
    id: Option<Value>,
    name: Option<String>,
    vertical: Option<String>,
    product_count: Option<Value>,
    feed_count: Option<Value>,
    is_catalog_segment: Option<bool>,
    is_local_catalog: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct RawProduct {
    id: Option<Value>,
    retailer_id: Option<String>,
    name: Option<String>,
    price: Option<String>,
    currency: Option<String>,
    availability: Option<String>,
    condition: Option<String>,
    brand: Option<String>,
    category: Option<String>,
    live_special_price: Option<String>,
    review_status: Option<String>,
    url: Option<String>,
    image_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawProductSet {
    id: Option<Value>,
    name: Option<String>,
    retailer_id: Option<String>,
    product_count: Option<Value>,
    filter: Option<String>,
    auto_creation_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawCustomConversion {
    id: Option<Value>,
    name: Option<String>,
    description: Option<String>,
    custom_event_type: Option<String>,
    event_source_id: Option<String>,
    event_source_type: Option<String>,
    action_source_type: Option<String>,
    is_archived: Option<bool>,
    is_unavailable: Option<bool>,
    creation_time: Option<String>,
    first_fired_time: Option<String>,
    last_fired_time: Option<String>,
    default_conversion_value: Option<i64>,
    retention_days: Option<u32>,
    rule: Option<String>,
    advanced_rule: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawBusinessDataset {
    id: Option<Value>,
    name: Option<String>,
    creation_time: Option<String>,
    last_fired_time: Option<String>,
    is_unavailable: Option<bool>,
}

pub(crate) async fn list_product_catalogs(
    graph: &GraphClient,
    input: ListProductCatalogsInput,
) -> ToolResponse<ProductCatalogList> {
    let request = match list_request(
        &input.business_id,
        "business_id",
        "owned_product_catalogs",
        CATALOG_FIELDS,
        input.page_size,
        input.page_cursor.as_deref(),
        false,
    ) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };

    match fetch_page(graph, request, "product catalog", normalize_catalog).await {
        Ok((catalogs, next_cursor)) => ToolResponse::success(ProductCatalogList {
            catalogs,
            next_cursor,
        }),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn list_products(
    graph: &GraphClient,
    input: ListProductsInput,
) -> ToolResponse<ProductList> {
    let mut request = match list_request(
        &input.product_catalog_id,
        "product_catalog_id",
        "products",
        PRODUCT_FIELDS,
        input.page_size,
        input.page_cursor.as_deref(),
        false,
    ) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    if let Some(approved_only) = input.return_only_approved_products {
        request.query.push((
            "return_only_approved_products".to_owned(),
            approved_only.to_string(),
        ));
    }

    match fetch_page(graph, request, "product", normalize_product).await {
        Ok((products, next_cursor)) => ToolResponse::success(ProductList {
            products,
            next_cursor,
        }),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn list_product_sets(
    graph: &GraphClient,
    input: ListProductSetsInput,
) -> ToolResponse<ProductSetList> {
    let request = match list_request(
        &input.product_catalog_id,
        "product_catalog_id",
        "product_sets",
        PRODUCT_SET_FIELDS,
        input.page_size,
        input.page_cursor.as_deref(),
        false,
    ) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };

    match fetch_page(graph, request, "product set", normalize_product_set).await {
        Ok((product_sets, next_cursor)) => ToolResponse::success(ProductSetList {
            product_sets,
            next_cursor,
        }),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn list_custom_conversions(
    graph: &GraphClient,
    input: ListCustomConversionsInput,
) -> ToolResponse<CustomConversionList> {
    let request = match list_request(
        &input.ad_account_id,
        "ad_account_id",
        "customconversions",
        CUSTOM_CONVERSION_LIST_FIELDS,
        input.page_size,
        input.page_cursor.as_deref(),
        true,
    ) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };

    match fetch_page(
        graph,
        request,
        "custom conversion",
        normalize_custom_conversion,
    )
    .await
    {
        Ok((custom_conversions, next_cursor)) => ToolResponse::success(CustomConversionList {
            custom_conversions,
            next_cursor,
        }),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn read_custom_conversion(
    graph: &GraphClient,
    input: ReadCustomConversionInput,
) -> ToolResponse<CustomConversion> {
    let Some(conversion_id) = normalize_numeric_id(&input.custom_conversion_id) else {
        return ToolResponse::error(PublicError::invalid_input(
            "custom_conversion_id must be a numeric Meta conversion ID",
            "Use an ID returned by list_custom_conversions",
        ));
    };
    let query = vec![(
        "fields".to_owned(),
        CUSTOM_CONVERSION_DETAIL_FIELDS.to_owned(),
    )];
    let payload = match graph.get_json(&conversion_id, &query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    let raw = match serde_json::from_value::<RawCustomConversion>(payload) {
        Ok(raw) => raw,
        Err(_) => {
            return ToolResponse::error(PublicError::invalid_upstream(
                "Meta returned unexpected custom-conversion metadata",
            ));
        }
    };
    match normalize_custom_conversion(raw) {
        Some(conversion) => ToolResponse::success(conversion),
        None => ToolResponse::error(PublicError::invalid_upstream(
            "Meta omitted a valid custom-conversion ID",
        )),
    }
}

pub(crate) async fn list_business_datasets(
    graph: &GraphClient,
    input: ListBusinessDatasetsInput,
) -> ToolResponse<BusinessDatasetList> {
    let request = match list_request(
        &input.business_id,
        "business_id",
        "ads_dataset",
        DATASET_FIELDS,
        input.page_size,
        input.page_cursor.as_deref(),
        false,
    ) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };

    match fetch_page(graph, request, "business dataset", normalize_dataset).await {
        Ok((datasets, next_cursor)) => ToolResponse::success(BusinessDatasetList {
            datasets,
            next_cursor,
        }),
        Err(error) => ToolResponse::error(error),
    }
}

fn list_request(
    parent_id: &str,
    field_name: &str,
    edge: &str,
    fields: &str,
    page_size: Option<u16>,
    page_cursor: Option<&str>,
    ad_account: bool,
) -> Result<ListRequest, PublicError> {
    let parent_id = if ad_account {
        normalize_ad_account_id(parent_id)
    } else {
        normalize_numeric_id(parent_id)
    }
    .ok_or_else(|| {
        PublicError::invalid_input(
            format!("{field_name} must be a numeric Meta ID"),
            format!("Use a numeric {field_name} without URL or path characters"),
        )
    })?;
    let page = validate_page(page_size, page_cursor)?;
    let mut query = vec![
        ("fields".to_owned(), fields.to_owned()),
        ("limit".to_owned(), page.size.to_string()),
    ];
    if let Some(cursor) = page.cursor {
        query.push(("after".to_owned(), cursor));
    }
    Ok(ListRequest {
        endpoint: format!("{parent_id}/{edge}"),
        query,
        page_size: page.size,
    })
}

fn validate_page(page_size: Option<u16>, page_cursor: Option<&str>) -> Result<Page, PublicError> {
    let size = page_size.unwrap_or(DEFAULT_PAGE_SIZE);
    if !(1..=MAX_PAGE_SIZE).contains(&size) {
        return Err(PublicError::invalid_input(
            "page_size must be between 1 and 100",
            "Choose a bounded page_size and paginate with next_cursor",
        ));
    }
    let cursor = page_cursor
        .filter(|cursor| !cursor.is_empty())
        .map(str::to_owned);
    if cursor.as_ref().is_some_and(|cursor| {
        cursor.chars().count() > MAX_CURSOR_CHARS || cursor.chars().any(char::is_control)
    }) {
        return Err(PublicError::invalid_input(
            "page_cursor is invalid or too long",
            "Use next_cursor exactly as returned by the previous response",
        ));
    }
    Ok(Page { size, cursor })
}

async fn fetch_page<T: DeserializeOwned, O>(
    graph: &GraphClient,
    request: ListRequest,
    resource: &str,
    normalize: fn(T) -> Option<O>,
) -> Result<(Vec<O>, Option<String>), PublicError> {
    let payload = graph
        .get_json(&request.endpoint, &request.query)
        .await
        .map_err(PublicError::from)?;
    let raw = serde_json::from_value::<RawPage>(payload).map_err(|_| {
        PublicError::invalid_upstream(format!("Meta returned an unexpected {resource} list"))
    })?;
    if raw.data.len() > usize::from(request.page_size) {
        return Err(PublicError::invalid_upstream(format!(
            "Meta returned more {resource} items than requested"
        )));
    }
    let next_cursor = extract_next_cursor(raw.paging, resource)?;
    let mut items = Vec::with_capacity(raw.data.len());
    for value in raw.data {
        let raw_item = serde_json::from_value::<T>(value).map_err(|_| {
            PublicError::invalid_upstream(format!("Meta returned malformed {resource} metadata"))
        })?;
        let item = normalize(raw_item).ok_or_else(|| {
            PublicError::invalid_upstream(format!("Meta omitted a valid {resource} ID"))
        })?;
        items.push(item);
    }
    Ok((items, next_cursor))
}

fn extract_next_cursor(
    paging: Option<RawPaging>,
    resource: &str,
) -> Result<Option<String>, PublicError> {
    let cursor = paging
        .and_then(|paging| paging.cursors)
        .and_then(|cursors| cursors.after)
        .filter(|cursor| !cursor.is_empty());
    if cursor.as_ref().is_some_and(|cursor| {
        cursor.chars().count() > MAX_CURSOR_CHARS || cursor.chars().any(char::is_control)
    }) {
        return Err(PublicError::invalid_upstream(format!(
            "Meta returned an invalid {resource} cursor"
        )));
    }
    Ok(cursor)
}

fn normalize_catalog(raw: RawProductCatalog) -> Option<ProductCatalog> {
    Some(ProductCatalog {
        id: normalize_numeric_value(raw.id)?,
        name: bounded_text(raw.name, MAX_NAME_CHARS),
        vertical: bounded_text(raw.vertical, MAX_METADATA_CHARS),
        product_count: nonnegative_integer(raw.product_count),
        feed_count: nonnegative_integer(raw.feed_count),
        is_catalog_segment: raw.is_catalog_segment,
        is_local_catalog: raw.is_local_catalog,
    })
}

fn normalize_product(raw: RawProduct) -> Option<Product> {
    Some(Product {
        id: normalize_numeric_value(raw.id)?,
        retailer_id: bounded_text(raw.retailer_id, MAX_IDENTIFIER_CHARS),
        name: bounded_text(raw.name, MAX_NAME_CHARS),
        price: bounded_text(raw.price, 64),
        currency: bounded_text(raw.currency, 16).map(|currency| currency.to_ascii_uppercase()),
        availability: bounded_text(raw.availability, MAX_METADATA_CHARS),
        condition: bounded_text(raw.condition, MAX_METADATA_CHARS),
        brand: bounded_text(raw.brand, MAX_NAME_CHARS),
        category: bounded_text(raw.category, MAX_NAME_CHARS),
        live_special_price: bounded_text(raw.live_special_price, 64),
        review_status: bounded_text(raw.review_status, MAX_METADATA_CHARS),
        url: bounded_url(raw.url),
        image_url: bounded_url(raw.image_url),
    })
}

fn normalize_product_set(raw: RawProductSet) -> Option<ProductSet> {
    let (filter, filter_truncated) = bounded_text_with_flag(raw.filter, MAX_RULE_CHARS);
    Some(ProductSet {
        id: normalize_numeric_value(raw.id)?,
        name: bounded_text(raw.name, MAX_NAME_CHARS),
        retailer_id: bounded_text(raw.retailer_id, MAX_IDENTIFIER_CHARS),
        product_count: nonnegative_integer(raw.product_count),
        filter,
        filter_truncated: filter_truncated.then_some(true),
        auto_creation_url: bounded_url(raw.auto_creation_url),
    })
}

fn normalize_custom_conversion(raw: RawCustomConversion) -> Option<CustomConversion> {
    let (rule, rule_truncated) = bounded_text_with_flag(raw.rule, MAX_RULE_CHARS);
    let (advanced_rule, advanced_rule_truncated) =
        bounded_text_with_flag(raw.advanced_rule, MAX_RULE_CHARS);
    Some(CustomConversion {
        id: normalize_numeric_value(raw.id)?,
        name: bounded_text(raw.name, MAX_NAME_CHARS),
        description: bounded_text(raw.description, 512),
        custom_event_type: bounded_text(raw.custom_event_type, MAX_METADATA_CHARS),
        event_source_id: raw.event_source_id.and_then(|id| normalize_numeric_id(&id)),
        event_source_type: bounded_text(raw.event_source_type, MAX_METADATA_CHARS),
        action_source_type: bounded_text(raw.action_source_type, MAX_METADATA_CHARS),
        is_archived: raw.is_archived,
        is_unavailable: raw.is_unavailable,
        creation_time: bounded_text(raw.creation_time, 64),
        first_fired_time: bounded_text(raw.first_fired_time, 64),
        last_fired_time: bounded_text(raw.last_fired_time, 64),
        default_conversion_value: raw.default_conversion_value,
        retention_days: raw.retention_days,
        rule,
        rule_truncated: rule_truncated.then_some(true),
        advanced_rule,
        advanced_rule_truncated: advanced_rule_truncated.then_some(true),
    })
}

fn normalize_dataset(raw: RawBusinessDataset) -> Option<BusinessDataset> {
    Some(BusinessDataset {
        id: normalize_numeric_value(raw.id)?,
        name: bounded_text(raw.name, MAX_NAME_CHARS),
        creation_time: bounded_text(raw.creation_time, 64),
        last_fired_time: bounded_text(raw.last_fired_time, 64),
        is_unavailable: raw.is_unavailable,
    })
}

fn normalize_numeric_value(raw: Option<Value>) -> Option<String> {
    meta_numeric_value(&raw?)
}

fn nonnegative_integer(raw: Option<Value>) -> Option<u64> {
    match raw? {
        Value::Number(value) => value.as_u64(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

fn bounded_text(raw: Option<String>, max_chars: usize) -> Option<String> {
    bounded_text_with_flag(raw, max_chars).0
}

fn bounded_text_with_flag(raw: Option<String>, max_chars: usize) -> (Option<String>, bool) {
    let Some(value) = raw else {
        return (None, false);
    };
    let value = value.trim();
    if value.is_empty() {
        return (None, false);
    }
    if value.chars().count() <= max_chars {
        return (Some(value.to_owned()), false);
    }
    let mut bounded = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    bounded.push('…');
    (Some(bounded), true)
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

    use super::{
        CATALOG_FIELDS, CUSTOM_CONVERSION_DETAIL_FIELDS, CUSTOM_CONVERSION_LIST_FIELDS,
        ListProductsInput, MAX_CURSOR_CHARS, MAX_RULE_CHARS, PRODUCT_FIELDS, PRODUCT_SET_FIELDS,
        RawCustomConversion, RawProduct, RawProductSet, bounded_url, list_request,
        normalize_custom_conversion, normalize_product, normalize_product_set,
    };

    #[test]
    fn builds_current_bounded_edge_requests() {
        let catalogs = list_request(
            "123",
            "business_id",
            "owned_product_catalogs",
            CATALOG_FIELDS,
            Some(10),
            Some("opaque+/=cursor"),
            false,
        )
        .unwrap();
        assert_eq!(catalogs.endpoint, "123/owned_product_catalogs");
        assert!(
            catalogs
                .query
                .contains(&("limit".to_owned(), "10".to_owned()))
        );
        assert!(
            catalogs
                .query
                .contains(&("after".to_owned(), "opaque+/=cursor".to_owned()))
        );

        let conversions = list_request(
            "456",
            "ad_account_id",
            "customconversions",
            CUSTOM_CONVERSION_LIST_FIELDS,
            None,
            None,
            true,
        )
        .unwrap();
        assert_eq!(conversions.endpoint, "act_456/customconversions");

        for fields in [PRODUCT_FIELDS, PRODUCT_SET_FIELDS] {
            assert!(!fields.contains("description"));
            assert!(!fields.contains("additional_image_urls"));
            assert!(!fields.contains("videos"));
        }
        assert!(!CUSTOM_CONVERSION_LIST_FIELDS.contains("rule"));
        assert!(CUSTOM_CONVERSION_DETAIL_FIELDS.contains("rule"));
        assert!(!CATALOG_FIELDS.contains("da_display_settings"));
    }

    #[test]
    fn validates_numeric_ids_pages_and_cursors() {
        assert!(
            list_request(
                "../123",
                "business_id",
                "ads_dataset",
                "id",
                None,
                None,
                false,
            )
            .is_err()
        );
        assert!(
            list_request(
                "123",
                "business_id",
                "ads_dataset",
                "id",
                Some(101),
                None,
                false,
            )
            .is_err()
        );
        assert!(
            list_request(
                "123",
                "business_id",
                "ads_dataset",
                "id",
                None,
                Some(&"x".repeat(MAX_CURSOR_CHARS + 1)),
                false,
            )
            .is_err()
        );
        assert!(
            list_request(
                "123",
                "business_id",
                "ads_dataset",
                "id",
                None,
                Some("cursor\nvalue"),
                false,
            )
            .is_err()
        );
    }

    #[test]
    fn normalizes_compact_products_and_safe_urls() {
        let raw: RawProduct = serde_json::from_value(json!({
            "id": "101",
            "retailer_id": "  SKU-1  ",
            "name": "  Sample  ",
            "currency": "usd",
            "url": "https://shop.example/item#tracking",
            "image_url": "data:image/png;base64,secret",
            "description": "must not escape",
            "videos": [{"url": "https://cdn.example/video.mp4"}]
        }))
        .unwrap();
        let product = normalize_product(raw).unwrap();
        assert_eq!(product.id, "101");
        assert_eq!(product.retailer_id.as_deref(), Some("SKU-1"));
        assert_eq!(product.currency.as_deref(), Some("USD"));
        assert_eq!(product.url.as_deref(), Some("https://shop.example/item"));
        assert!(product.image_url.is_none());
        let encoded = serde_json::to_string(&product).unwrap();
        assert!(!encoded.contains("must not escape"));
        assert!(!encoded.contains("video.mp4"));
        assert!(bounded_url(Some("https://user:pass@example.test".to_owned())).is_none());

        let input = ListProductsInput {
            product_catalog_id: "101".to_owned(),
            page_size: None,
            page_cursor: None,
            return_only_approved_products: Some(true),
        };
        assert!(input.return_only_approved_products.unwrap());
    }

    #[test]
    fn marks_truncated_filters_and_conversion_rules() {
        let raw_set: RawProductSet = serde_json::from_value(json!({
            "id": "201",
            "filter": "x".repeat(MAX_RULE_CHARS + 20),
            "auto_creation_url": "javascript:alert(1)"
        }))
        .unwrap();
        let product_set = normalize_product_set(raw_set).unwrap();
        assert_eq!(product_set.filter_truncated, Some(true));
        assert_eq!(
            product_set.filter.as_ref().unwrap().chars().count(),
            MAX_RULE_CHARS
        );
        assert!(product_set.auto_creation_url.is_none());

        let raw_conversion: RawCustomConversion = serde_json::from_value(json!({
            "id": 301,
            "event_source_id": "302",
            "default_conversion_value": 42,
            "retention_days": 30,
            "rule": "r".repeat(MAX_RULE_CHARS + 1),
            "advanced_rule": "{}"
        }))
        .unwrap();
        let conversion = normalize_custom_conversion(raw_conversion).unwrap();
        assert_eq!(conversion.id, "301");
        assert_eq!(conversion.event_source_id.as_deref(), Some("302"));
        assert_eq!(conversion.rule_truncated, Some(true));
        assert!(conversion.advanced_rule_truncated.is_none());
    }
}
