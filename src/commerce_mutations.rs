// Copyright (C) 2025 ArmaVita LLC
// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::HashSet;

use reqwest::Url;
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    bounded_json::{credential_value, encode_nonempty_object, request_control_key},
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{numeric, numeric_value as normalize_numeric_value},
    mutation_result::{ambiguous_mutation_result, mutation_error_without_blind_retry},
    safety::validate_removal_acknowledgement,
};

const MAX_RETAILER_ID_CHARS: usize = 100;
const MAX_NAME_CHARS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 5_000;
const MAX_BRAND_CHARS: usize = 100;
const MAX_CATEGORY_CHARS: usize = 250;
const MAX_GTIN_CHARS: usize = 50;
const MAX_MPN_CHARS: usize = 70;
const MAX_URL_CHARS: usize = 2_048;
const MAX_ADDITIONAL_IMAGES: usize = 10;
const MAX_BATCH_ITEMS: usize = 100;
// GraphClient caps encoded mutation bodies at 128 KiB. Form encoding can
// expand every JSON byte to three bytes, so 40 KiB remains safely below it.
const MAX_BATCH_JSON_BYTES: usize = 40 * 1_024;
// Fifteen typed values plus retailer_id and allow_upsert leave 47 of
// GraphClient's 64 form pairs for provider fields.
const MAX_SINGLE_ADDITIONAL_FIELDS: usize = 47;
const MAX_BATCH_ADDITIONAL_FIELDS: usize = 128;
const MAX_HANDLES: usize = 16;
const MAX_HANDLE_CHARS: usize = 2_048;
const MAX_PRICE_MINOR_UNITS: u64 = 1_000_000_000_000_000;

const CONTROL_FIELDS: &[&str] = &[
    "retailer_id",
    "allow_upsert",
    "product_catalog_id",
    "additional_fields",
    "item_type",
    "requests",
    "method",
    "data",
    "operation",
    "removal_acknowledgement",
];

const SINGLE_TYPED_FIELDS: &[&str] = &[
    "name",
    "description",
    "price",
    "price_minor_units",
    "sale_price",
    "sale_price_minor_units",
    "currency",
    "availability",
    "condition",
    "url",
    "image_url",
    "additional_image_urls",
    "brand",
    "category",
    "gtin",
    "manufacturer_part_number",
    "retailer_product_group_id",
];

const BATCH_TYPED_FIELDS: &[&str] = &[
    "title",
    "description",
    "price",
    "price_amount",
    "sale_price",
    "sale_price_amount",
    "currency",
    "availability",
    "condition",
    "url",
    "link",
    "image_url",
    "image_link",
    "additional_image_urls",
    "additional_image_link",
    "brand",
    "gtin",
    "manufacturer_part_number",
    "mpn",
    "retailer_product_group_id",
    "item_group_id",
];

/// Create or update one product by stable retailer ID through Meta v26's
/// `/products` edge. Repeating the same input has the same catalog effect.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpsertProductInput {
    /// Numeric Meta product-catalog ID.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub product_catalog_id: String,
    /// Stable retailer product ID plus the fields to set.
    pub product: ProductUpsert,
    /// Create the retailer ID when absent. Defaults to true.
    #[serde(default = "default_true")]
    pub allow_upsert: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProductUpsert {
    /// Stable retailer/SKU ID, from 1 through 100 printable characters.
    #[schemars(length(min = 1, max = 100))]
    pub retailer_id: String,
    /// Product title, from 1 through 200 characters.
    #[schemars(length(min = 1, max = 200))]
    pub name: Option<String>,
    /// Product description, from 1 through 5,000 characters.
    #[schemars(length(min = 1, max = 5000))]
    pub description: Option<String>,
    /// Regular price in the currency's minor units, such as 1299 for USD 12.99.
    #[schemars(range(max = 1_000_000_000_000_000_u64))]
    pub price_minor_units: Option<u64>,
    /// Optional sale price in the same minor units; requires `currency`.
    #[schemars(range(max = 1_000_000_000_000_000_u64))]
    pub sale_price_minor_units: Option<u64>,
    /// Three-letter ISO 4217 currency code. Required with either price.
    #[schemars(length(min = 3, max = 3), regex(pattern = "^[A-Za-z]{3}$"))]
    pub currency: Option<String>,
    pub availability: Option<ProductAvailability>,
    pub condition: Option<ProductCondition>,
    /// Public HTTPS landing-page URL.
    #[schemars(length(min = 1, max = 2048), url)]
    pub url: Option<String>,
    /// Public HTTPS primary-image URL.
    #[schemars(length(min = 1, max = 2048), url)]
    pub image_url: Option<String>,
    /// Up to ten public HTTPS secondary-image URLs.
    #[schemars(default, length(max = 10), inner(length(min = 1, max = 2048), url))]
    pub additional_image_urls: Option<Vec<String>>,
    #[schemars(length(min = 1, max = 100))]
    pub brand: Option<String>,
    #[schemars(length(min = 1, max = 250))]
    pub category: Option<String>,
    #[schemars(length(min = 1, max = 50))]
    pub gtin: Option<String>,
    #[schemars(length(min = 1, max = 70))]
    pub manufacturer_part_number: Option<String>,
    /// Stable retailer group ID for product variants.
    #[schemars(length(min = 1, max = 100))]
    pub retailer_product_group_id: Option<String>,
    /// Current Meta product fields not modeled above, such as vertical-specific
    /// attributes. Keys must not duplicate typed/control fields or contain secrets.
    #[serde(default)]
    #[schemars(default, schema_with = "single_additional_fields_schema")]
    pub additional_fields: Option<Map<String, Value>>,
}

/// Submit a bounded set of stable-ID upserts and deletions to Meta's
/// asynchronous `/items_batch` edge. No append/create operation is exposed.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchProductsInput {
    /// Numeric Meta product-catalog ID.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub product_catalog_id: String,
    /// One through 100 explicit upsert/delete operations with unique retailer IDs.
    #[schemars(length(min = 1, max = 100))]
    pub operations: Vec<ProductBatchOperation>,
    /// Required when any operation deletes a product, after operator approval; omit for upserts.
    #[schemars(
        length(min = 25, max = 25),
        regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
    )]
    pub removal_acknowledgement: Option<String>,
    /// Let upserts create retailer IDs that do not exist. Defaults to true.
    #[serde(default = "default_true")]
    pub allow_upsert: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProductBatchOperation {
    /// Idempotently set fields on one retailer ID, creating it when allowed.
    Upsert {
        #[schemars(length(min = 1, max = 100))]
        retailer_id: String,
        product: Box<BatchProductUpsert>,
    },
    /// Permanently delete one retailer ID from the catalog.
    Delete {
        #[schemars(length(min = 1, max = 100))]
        retailer_id: String,
    },
}

impl ProductBatchOperation {
    fn retailer_id(&self) -> &str {
        match self {
            Self::Upsert { retailer_id, .. } | Self::Delete { retailer_id } => retailer_id,
        }
    }
}

/// Product fields for Meta's batch/feed contract. Prices use decimal major
/// units because that contract differs from the single-product edge.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchProductUpsert {
    #[schemars(length(min = 1, max = 200))]
    pub title: Option<String>,
    #[schemars(length(min = 1, max = 5000))]
    pub description: Option<String>,
    /// Decimal major-unit amount without a currency suffix, such as `12.99`.
    #[schemars(
        length(min = 1, max = 17),
        regex(pattern = "^[0-9]{1,12}(\\.[0-9]{1,4})?$")
    )]
    pub price_amount: Option<String>,
    /// Optional decimal sale amount; requires `currency`.
    #[schemars(
        length(min = 1, max = 17),
        regex(pattern = "^[0-9]{1,12}(\\.[0-9]{1,4})?$")
    )]
    pub sale_price_amount: Option<String>,
    #[schemars(length(min = 3, max = 3), regex(pattern = "^[A-Za-z]{3}$"))]
    pub currency: Option<String>,
    pub availability: Option<ProductAvailability>,
    pub condition: Option<ProductCondition>,
    #[schemars(length(min = 1, max = 2048), url)]
    pub url: Option<String>,
    #[schemars(length(min = 1, max = 2048), url)]
    pub image_url: Option<String>,
    #[schemars(default, length(max = 10), inner(length(min = 1, max = 2048), url))]
    pub additional_image_urls: Option<Vec<String>>,
    #[schemars(length(min = 1, max = 100))]
    pub brand: Option<String>,
    #[schemars(length(min = 1, max = 50))]
    pub gtin: Option<String>,
    #[schemars(length(min = 1, max = 70))]
    pub manufacturer_part_number: Option<String>,
    #[schemars(length(min = 1, max = 100))]
    pub retailer_product_group_id: Option<String>,
    /// Current Meta batch product fields not modeled above. These are flattened
    /// into `data`; keys must not duplicate typed/control fields or contain secrets.
    #[serde(default)]
    #[schemars(default, schema_with = "batch_additional_fields_schema")]
    pub additional_fields: Option<Map<String, Value>>,
}

fn single_additional_fields_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    additional_fields_schema(generator, MAX_SINGLE_ADDITIONAL_FIELDS)
}

fn batch_additional_fields_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    additional_fields_schema(generator, MAX_BATCH_ADDITIONAL_FIELDS)
}

fn additional_fields_schema(
    generator: &mut schemars::SchemaGenerator,
    maximum: usize,
) -> schemars::Schema {
    let mut schema = Map::<String, Value>::json_schema(generator);
    schema.insert("minProperties".into(), 1_u64.into());
    schema.insert("maxProperties".into(), (maximum as u64).into());
    schema
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductAvailability {
    AvailableForOrder,
    Discontinued,
    InStock,
    MarkAsExpired,
    MarkAsSold,
    OutOfStock,
    Pending,
    Preorder,
}

impl ProductAvailability {
    const fn as_str(self) -> &'static str {
        match self {
            Self::AvailableForOrder => "available for order",
            Self::Discontinued => "discontinued",
            Self::InStock => "in stock",
            Self::MarkAsExpired => "mark_as_expired",
            Self::MarkAsSold => "mark_as_sold",
            Self::OutOfStock => "out of stock",
            Self::Pending => "pending",
            Self::Preorder => "preorder",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductCondition {
    Cpo,
    New,
    OpenBoxNew,
    Refurbished,
    Used,
    UsedFair,
    UsedGood,
    UsedLikeNew,
}

impl ProductCondition {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Cpo => "cpo",
            Self::New => "new",
            Self::OpenBoxNew => "open_box_new",
            Self::Refurbished => "refurbished",
            Self::Used => "used",
            Self::UsedFair => "used_fair",
            Self::UsedGood => "used_good",
            Self::UsedLikeNew => "used_like_new",
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UpsertedProduct {
    pub product_id: String,
    pub retailer_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AcceptedProductBatch {
    pub handles: Vec<String>,
    pub submitted: usize,
    pub deletions: usize,
}

#[derive(Debug, PartialEq, Eq)]
struct MutationRequest {
    endpoint: String,
    form: Vec<(String, String)>,
}

struct BuiltUpsert {
    request: MutationRequest,
    retailer_id: String,
}

struct BuiltBatch {
    request: MutationRequest,
    submitted: usize,
    deletions: usize,
}

pub(crate) async fn upsert_product(
    graph: &GraphClient,
    input: UpsertProductInput,
) -> ToolResponse<UpsertedProduct> {
    let built = match build_upsert_request(input) {
        Ok(built) => built,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph
        .post_form_json(&built.request.endpoint, &built.request.form)
        .await
    {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    match product_id(&payload) {
        Some(product_id) => ToolResponse::success(UpsertedProduct {
            product_id,
            retailer_id: built.retailer_id,
        }),
        None => ToolResponse::error(retryable_idempotent_result(
            "Meta did not return the upserted product ID",
            "Retry the same upsert; the retailer ID makes its catalog effect idempotent",
        )),
    }
}

pub(crate) async fn batch_products(
    graph: &GraphClient,
    input: BatchProductsInput,
) -> ToolResponse<AcceptedProductBatch> {
    let built = match build_batch_request(input) {
        Ok(built) => built,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph
        .post_form_json(&built.request.endpoint, &built.request.form)
        .await
    {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(mutation_error_without_blind_retry(
                error,
                "Check Commerce Manager for the batch before submitting it again",
            ));
        }
    };
    match batch_handles(&payload) {
        Some(handles) => ToolResponse::success(AcceptedProductBatch {
            handles,
            submitted: built.submitted,
            deletions: built.deletions,
        }),
        None => ToolResponse::error(ambiguous_mutation_result(
            "Meta did not return a catalog batch handle",
            "Check Commerce Manager for the batch before submitting it again",
        )),
    }
}

fn build_upsert_request(input: UpsertProductInput) -> Result<BuiltUpsert, PublicError> {
    let catalog_id = numeric_id(input.product_catalog_id, "product_catalog_id")?;
    let ProductUpsert {
        retailer_id,
        name,
        description,
        price_minor_units,
        sale_price_minor_units,
        currency,
        availability,
        condition,
        url,
        image_url,
        additional_image_urls,
        brand,
        category,
        gtin,
        manufacturer_part_number,
        retailer_product_group_id,
        additional_fields,
    } = input.product;
    let retailer_id = required_text(retailer_id, MAX_RETAILER_ID_CHARS, "retailer_id")?;
    validate_minor_prices(
        price_minor_units,
        sale_price_minor_units,
        currency.as_deref(),
    )?;

    let mut form = Vec::with_capacity(17);
    form.push(("retailer_id".to_owned(), retailer_id.clone()));
    push_text(&mut form, "name", name, MAX_NAME_CHARS)?;
    push_text(&mut form, "description", description, MAX_DESCRIPTION_CHARS)?;
    push_number(&mut form, "price", price_minor_units);
    push_number(&mut form, "sale_price", sale_price_minor_units);
    if let Some(currency) = currency {
        form.push(("currency".to_owned(), normalize_currency(currency)?));
    }
    if let Some(availability) = availability {
        form.push(("availability".to_owned(), availability.as_str().to_owned()));
    }
    if let Some(condition) = condition {
        form.push(("condition".to_owned(), condition.as_str().to_owned()));
    }
    push_url(&mut form, "url", url)?;
    push_url(&mut form, "image_url", image_url)?;
    if let Some(urls) = additional_image_urls {
        let urls = normalized_urls(urls, "additional_image_urls")?;
        let encoded = serde_json::to_string(&urls).map_err(|_| {
            PublicError::invalid_input(
                "additional_image_urls could not be encoded",
                "Use a list of HTTPS image URLs",
            )
        })?;
        form.push(("additional_image_urls".to_owned(), encoded));
    }
    push_text(&mut form, "brand", brand, MAX_BRAND_CHARS)?;
    push_text(&mut form, "category", category, MAX_CATEGORY_CHARS)?;
    push_text(&mut form, "gtin", gtin, MAX_GTIN_CHARS)?;
    push_text(
        &mut form,
        "manufacturer_part_number",
        manufacturer_part_number,
        MAX_MPN_CHARS,
    )?;
    push_text(
        &mut form,
        "retailer_product_group_id",
        retailer_product_group_id,
        MAX_RETAILER_ID_CHARS,
    )?;
    for (key, value) in validated_additional_fields(
        additional_fields,
        MAX_SINGLE_ADDITIONAL_FIELDS,
        SINGLE_TYPED_FIELDS,
    )? {
        let value = provider_form_value(value, &key)?;
        form.push((key, value));
    }
    if form.len() == 1 {
        return Err(PublicError::invalid_input(
            "at least one writable product field is required",
            "Provide a product field in addition to retailer_id",
        ));
    }
    form.push(("allow_upsert".to_owned(), input.allow_upsert.to_string()));
    Ok(BuiltUpsert {
        request: MutationRequest {
            endpoint: format!("{catalog_id}/products"),
            form,
        },
        retailer_id,
    })
}

fn build_batch_request(input: BatchProductsInput) -> Result<BuiltBatch, PublicError> {
    let catalog_id = numeric_id(input.product_catalog_id, "product_catalog_id")?;
    if input.operations.is_empty() || input.operations.len() > MAX_BATCH_ITEMS {
        return Err(PublicError::invalid_input(
            "operations must contain 1 through 100 items",
            "Split larger catalogs into batches of at most 100 operations",
        ));
    }
    validate_removal_acknowledgement(
        input
            .operations
            .iter()
            .any(|operation| matches!(operation, ProductBatchOperation::Delete { .. })),
        input.removal_acknowledgement.as_deref(),
    )?;
    let mut retailer_ids = HashSet::with_capacity(input.operations.len());
    for operation in &input.operations {
        let retailer_id = validated_text(
            operation.retailer_id(),
            MAX_RETAILER_ID_CHARS,
            "retailer_id",
        )?;
        if !retailer_ids.insert(retailer_id) {
            return Err(PublicError::invalid_input(
                "retailer IDs must be unique within one batch",
                "Combine fields into one operation per retailer ID",
            ));
        }
    }

    let submitted = input.operations.len();
    let mut deletions = 0;
    let mut requests = Vec::with_capacity(submitted);
    for operation in input.operations {
        match operation {
            ProductBatchOperation::Upsert {
                retailer_id,
                product,
            } => requests.push(ProviderBatchRequest::Update {
                retailer_id: required_text(retailer_id, MAX_RETAILER_ID_CHARS, "retailer_id")?,
                data: Box::new(provider_batch_product(*product)?),
            }),
            ProductBatchOperation::Delete { retailer_id } => {
                deletions += 1;
                requests.push(ProviderBatchRequest::Delete {
                    retailer_id: required_text(retailer_id, MAX_RETAILER_ID_CHARS, "retailer_id")?,
                });
            }
        }
    }
    let encoded = serde_json::to_string(&requests).map_err(|_| {
        PublicError::invalid_input(
            "operations could not be encoded",
            "Use the typed product batch fields",
        )
    })?;
    if encoded.len() > MAX_BATCH_JSON_BYTES {
        return Err(PublicError::invalid_input(
            "catalog batch exceeds the 40 KiB server safety limit",
            "Use fewer operations or shorter product text and URLs",
        ));
    }
    Ok(BuiltBatch {
        request: MutationRequest {
            endpoint: format!("{catalog_id}/items_batch"),
            form: vec![
                ("allow_upsert".to_owned(), input.allow_upsert.to_string()),
                ("item_type".to_owned(), "PRODUCT_ITEM".to_owned()),
                ("requests".to_owned(), encoded),
            ],
        },
        submitted,
        deletions,
    })
}

#[derive(Serialize)]
#[serde(tag = "method", rename_all = "UPPERCASE")]
enum ProviderBatchRequest {
    Update {
        retailer_id: String,
        data: Box<ProviderBatchProduct>,
    },
    Delete {
        retailer_id: String,
    },
}

#[derive(Serialize)]
struct ProviderBatchProduct {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    price: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sale_price: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    availability: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    condition: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    additional_image_link: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    brand: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gtin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mpn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    item_group_id: Option<String>,
    #[serde(flatten)]
    additional_fields: Map<String, Value>,
}

fn provider_batch_product(input: BatchProductUpsert) -> Result<ProviderBatchProduct, PublicError> {
    let BatchProductUpsert {
        title,
        description,
        price_amount,
        sale_price_amount,
        currency,
        availability,
        condition,
        url,
        image_url,
        additional_image_urls,
        brand,
        gtin,
        manufacturer_part_number,
        retailer_product_group_id,
        additional_fields,
    } = input;
    let price_amount = optional_decimal(price_amount, "price_amount")?;
    let sale_price_amount = optional_decimal(sale_price_amount, "sale_price_amount")?;
    validate_decimal_prices(
        price_amount.as_deref(),
        sale_price_amount.as_deref(),
        currency.as_deref(),
    )?;
    let currency = currency.map(normalize_currency).transpose()?;
    let price = price_amount.map(|amount| {
        format!(
            "{amount} {}",
            currency.as_deref().expect("currency validated")
        )
    });
    let sale_price = sale_price_amount.map(|amount| {
        format!(
            "{amount} {}",
            currency.as_deref().expect("currency validated")
        )
    });
    let product = ProviderBatchProduct {
        title: optional_text(title, MAX_NAME_CHARS, "title")?,
        description: optional_text(description, MAX_DESCRIPTION_CHARS, "description")?,
        price,
        sale_price,
        availability: availability.map(ProductAvailability::as_str),
        condition: condition.map(ProductCondition::as_str),
        link: optional_url(url, "url")?,
        image_link: optional_url(image_url, "image_url")?,
        additional_image_link: additional_image_urls
            .map(|urls| normalized_urls(urls, "additional_image_urls"))
            .transpose()?,
        brand: optional_text(brand, MAX_BRAND_CHARS, "brand")?,
        gtin: optional_text(gtin, MAX_GTIN_CHARS, "gtin")?,
        mpn: optional_text(
            manufacturer_part_number,
            MAX_MPN_CHARS,
            "manufacturer_part_number",
        )?,
        item_group_id: optional_text(
            retailer_product_group_id,
            MAX_RETAILER_ID_CHARS,
            "retailer_product_group_id",
        )?,
        additional_fields: validated_additional_fields(
            additional_fields,
            MAX_BATCH_ADDITIONAL_FIELDS,
            BATCH_TYPED_FIELDS,
        )?,
    };
    if product.is_empty() {
        return Err(PublicError::invalid_input(
            "batch upsert product cannot be empty",
            "Provide at least one product field",
        ));
    }
    Ok(product)
}

impl ProviderBatchProduct {
    fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.description.is_none()
            && self.price.is_none()
            && self.sale_price.is_none()
            && self.availability.is_none()
            && self.condition.is_none()
            && self.link.is_none()
            && self.image_link.is_none()
            && self.additional_image_link.is_none()
            && self.brand.is_none()
            && self.gtin.is_none()
            && self.mpn.is_none()
            && self.item_group_id.is_none()
            && self.additional_fields.is_empty()
    }
}

fn validated_additional_fields(
    fields: Option<Map<String, Value>>,
    maximum_fields: usize,
    typed_fields: &[&str],
) -> Result<Map<String, Value>, PublicError> {
    let Some(fields) = fields else {
        return Ok(Map::new());
    };
    if fields.len() > maximum_fields {
        return Err(PublicError::invalid_input(
            format!("additional_fields supports at most {maximum_fields} fields here"),
            "Remove unused fields or split catalog changes into smaller requests",
        ));
    }
    encode_nonempty_object(&fields, "additional_fields")?;
    for (key, value) in &fields {
        if request_control_key(key)
            || CONTROL_FIELDS
                .iter()
                .chain(typed_fields)
                .any(|reserved| key.eq_ignore_ascii_case(reserved))
        {
            return Err(PublicError::invalid_input(
                format!("additional_fields cannot override `{key}`"),
                "Use the typed field or omit this reserved control field",
            ));
        }
        if whatsapp_key(key) || contains_whatsapp_value(value) {
            return Err(PublicError::invalid_input(
                "additional_fields cannot contain WhatsApp-specific fields or values",
                "Use non-WhatsApp Meta catalog attributes",
            ));
        }
        if value.is_null() {
            return Err(PublicError::invalid_input(
                format!("additional_fields.{key} cannot be null"),
                "Omit the field or provide its intended value",
            ));
        }
    }
    Ok(fields)
}

fn provider_form_value(value: Value, field: &str) -> Result<String, PublicError> {
    match value {
        Value::Null => Err(PublicError::invalid_input(
            format!("additional_fields.{field} cannot be null"),
            "Omit the field or provide its intended value",
        )),
        Value::String(value) => Ok(value),
        value => serde_json::to_string(&value).map_err(|_| {
            PublicError::invalid_input(
                format!("additional_fields.{field} could not be encoded"),
                "Use a JSON-compatible provider value",
            )
        }),
    }
}

fn whatsapp_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("whatsapp")
        || key.contains("whats_app")
        || key == "ctwa"
        || key.starts_with("ctwa_")
        || key.starts_with("wa_")
}

fn contains_whatsapp_value(root: &Value) -> bool {
    let mut stack = vec![root];
    while let Some(value) = stack.pop() {
        match value {
            Value::Object(object) => {
                if object.keys().any(|key| whatsapp_key(key)) {
                    return true;
                }
                stack.extend(object.values());
            }
            Value::Array(items) => stack.extend(items),
            Value::String(text) => {
                let text = text.to_ascii_lowercase();
                if text.contains("whatsapp")
                    || text.contains("whats_app")
                    || text.contains("wa.me")
                    || text == "ctwa"
                    || text.starts_with("ctwa_")
                {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn validate_minor_prices(
    price: Option<u64>,
    sale_price: Option<u64>,
    currency: Option<&str>,
) -> Result<(), PublicError> {
    if price.is_some_and(|value| value > MAX_PRICE_MINOR_UNITS)
        || sale_price.is_some_and(|value| value > MAX_PRICE_MINOR_UNITS)
    {
        return Err(PublicError::invalid_input(
            "product price exceeds the server safety limit",
            "Use at most 1,000,000,000,000,000 minor units",
        ));
    }
    validate_price_pair(price, sale_price, currency)
}

fn validate_price_pair<T: Copy + PartialOrd>(
    price: Option<T>,
    sale_price: Option<T>,
    currency: Option<&str>,
) -> Result<(), PublicError> {
    if (price.is_some() || sale_price.is_some()) != currency.is_some() {
        return Err(PublicError::invalid_input(
            "currency and price fields must be provided together",
            "Provide currency with price/sale price, or omit all price fields",
        ));
    }
    if matches!((price, sale_price), (Some(price), Some(sale)) if sale > price) {
        return Err(PublicError::invalid_input(
            "sale price cannot exceed regular price",
            "Lower the sale price or raise the regular price",
        ));
    }
    Ok(())
}

fn validate_decimal_prices(
    price: Option<&str>,
    sale_price: Option<&str>,
    currency: Option<&str>,
) -> Result<(), PublicError> {
    validate_price_pair(
        price.map(decimal_units).transpose()?,
        sale_price.map(decimal_units).transpose()?,
        currency,
    )
}

fn decimal_units(value: &str) -> Result<u64, PublicError> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || whole.len() > 12
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 4
        || (!fraction.is_empty() && !fraction.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(invalid_decimal());
    }
    let whole = whole.parse::<u64>().map_err(|_| invalid_decimal())?;
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u64>().map_err(|_| invalid_decimal())?
            * 10_u64.pow(4 - fraction.len() as u32)
    };
    whole
        .checked_mul(10_000)
        .and_then(|value| value.checked_add(fraction))
        .ok_or_else(invalid_decimal)
}

fn optional_decimal(raw: Option<String>, field: &str) -> Result<Option<String>, PublicError> {
    raw.map(|value| {
        let value = required_text(value, 17, field)?;
        decimal_units(&value)?;
        Ok(value)
    })
    .transpose()
}

fn invalid_decimal() -> PublicError {
    PublicError::invalid_input(
        "price amount must be a nonnegative decimal with at most four fraction digits",
        "Use a value such as `12.99`; provide currency separately",
    )
}

fn push_text(
    form: &mut Vec<(String, String)>,
    key: &str,
    value: Option<String>,
    maximum: usize,
) -> Result<(), PublicError> {
    if let Some(value) = optional_text(value, maximum, key)? {
        form.push((key.to_owned(), value));
    }
    Ok(())
}

fn push_number(form: &mut Vec<(String, String)>, key: &str, value: Option<u64>) {
    if let Some(value) = value {
        form.push((key.to_owned(), value.to_string()));
    }
}

fn push_url(
    form: &mut Vec<(String, String)>,
    key: &str,
    value: Option<String>,
) -> Result<(), PublicError> {
    if let Some(value) = optional_url(value, key)? {
        form.push((key.to_owned(), value));
    }
    Ok(())
}

fn numeric_id(raw: String, field: &str) -> Result<String, PublicError> {
    let Some(value) = numeric(&raw) else {
        return Err(PublicError::invalid_input(
            format!("{field} must be a numeric Meta ID"),
            "Use the numeric ID returned by Meta",
        ));
    };
    Ok(if value.len() == raw.len() {
        raw
    } else {
        value.to_owned()
    })
}

fn required_text(mut raw: String, maximum: usize, field: &str) -> Result<String, PublicError> {
    let value = validated_text(&raw, maximum, field)?;
    if value.len() != raw.len() {
        raw = value.to_owned();
    }
    Ok(raw)
}

fn optional_text(
    raw: Option<String>,
    maximum: usize,
    field: &str,
) -> Result<Option<String>, PublicError> {
    raw.map(|value| required_text(value, maximum, field))
        .transpose()
}

fn validated_text<'a>(raw: &'a str, maximum: usize, field: &str) -> Result<&'a str, PublicError> {
    let value = raw.trim();
    if value.is_empty() || value.chars().count() > maximum || value.chars().any(char::is_control) {
        return Err(PublicError::invalid_input(
            format!("{field} is empty, invalid, or too long"),
            format!("Use 1 through {maximum} printable characters"),
        ));
    }
    Ok(value)
}

fn normalize_currency(mut raw: String) -> Result<String, PublicError> {
    let value = raw.trim();
    if value.len() != 3 || !value.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return Err(PublicError::invalid_input(
            "currency must be a three-letter ISO 4217 code",
            "Use a code such as USD, EUR, or JPY",
        ));
    }
    if value.len() != raw.len() {
        raw = value.to_owned();
    }
    raw.make_ascii_uppercase();
    Ok(raw)
}

fn optional_url(raw: Option<String>, field: &str) -> Result<Option<String>, PublicError> {
    raw.map(|value| https_url(value, field)).transpose()
}

fn https_url(raw: String, field: &str) -> Result<String, PublicError> {
    let value = required_text(raw, MAX_URL_CHARS, field)?;
    let parsed = Url::parse(&value).map_err(|_| invalid_url(field))?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || credential_value(&value)
    {
        return Err(invalid_url(field));
    }
    Ok(value)
}

fn invalid_url(field: &str) -> PublicError {
    PublicError::invalid_input(
        format!("{field} must be a public HTTPS URL without user credentials"),
        "Use an absolute https:// URL",
    )
}

fn normalized_urls(values: Vec<String>, field: &str) -> Result<Vec<String>, PublicError> {
    if values.len() > MAX_ADDITIONAL_IMAGES {
        return Err(PublicError::invalid_input(
            format!("{field} supports at most 10 URLs"),
            "Keep the primary image separate and provide at most 10 secondary images",
        ));
    }
    values
        .into_iter()
        .map(|value| https_url(value, field))
        .collect()
}

fn product_id(payload: &Value) -> Option<String> {
    normalize_numeric_value(payload.get("id")?)
}

fn batch_handles(payload: &Value) -> Option<Vec<String>> {
    let raw = payload.get("handles")?.as_array()?;
    if raw.is_empty() || raw.len() > MAX_HANDLES {
        return None;
    }
    let mut handles = Vec::with_capacity(raw.len());
    for value in raw {
        let handle = value.as_str()?.trim();
        if handle.is_empty()
            || handle.chars().count() > MAX_HANDLE_CHARS
            || handle.chars().any(char::is_control)
        {
            return None;
        }
        handles.push(handle.to_owned());
    }
    Some(handles)
}

fn retryable_idempotent_result(message: &str, action: &str) -> PublicError {
    PublicError {
        code: "AMBIGUOUS_MUTATION_RESULT".to_owned(),
        message: message.to_owned(),
        retryable: true,
        action: Some(action.to_owned()),
    }
}

const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use rmcp::schemars::schema_for;
    use serde_json::{Value, json};

    use crate::safety::REMOVAL_ACKNOWLEDGEMENT;

    use super::{
        BatchProductUpsert, BatchProductsInput, MAX_SINGLE_ADDITIONAL_FIELDS, ProductAvailability,
        ProductBatchOperation, ProductCondition, ProductUpsert, UpsertProductInput, batch_handles,
        build_batch_request, build_upsert_request, product_id, provider_batch_product,
    };

    fn empty_product(retailer_id: &str) -> ProductUpsert {
        ProductUpsert {
            retailer_id: retailer_id.to_owned(),
            name: None,
            description: None,
            price_minor_units: None,
            sale_price_minor_units: None,
            currency: None,
            availability: None,
            condition: None,
            url: None,
            image_url: None,
            additional_image_urls: None,
            brand: None,
            category: None,
            gtin: None,
            manufacturer_part_number: None,
            retailer_product_group_id: None,
            additional_fields: None,
        }
    }

    fn empty_batch_product() -> BatchProductUpsert {
        BatchProductUpsert {
            title: None,
            description: None,
            price_amount: None,
            sale_price_amount: None,
            currency: None,
            availability: None,
            condition: None,
            url: None,
            image_url: None,
            additional_image_urls: None,
            brand: None,
            gtin: None,
            manufacturer_part_number: None,
            retailer_product_group_id: None,
            additional_fields: None,
        }
    }

    #[test]
    fn additional_field_schemas_advertise_runtime_bounds() {
        let single = serde_json::to_value(schema_for!(ProductUpsert)).unwrap();
        let batch = serde_json::to_value(schema_for!(BatchProductUpsert)).unwrap();
        assert_eq!(
            single["properties"]["additional_fields"]["maxProperties"],
            MAX_SINGLE_ADDITIONAL_FIELDS
        );
        assert_eq!(
            batch["properties"]["additional_fields"]["maxProperties"],
            super::MAX_BATCH_ADDITIONAL_FIELDS
        );
        assert_eq!(
            single["properties"]["additional_fields"]["minProperties"],
            1
        );
        for schema in [&single, &batch] {
            assert!(
                !schema["required"]
                    .as_array()
                    .is_some_and(|required| required
                        .iter()
                        .any(|field| field == "additional_fields"))
            );
        }
    }

    #[test]
    fn builds_exact_single_product_form() {
        let mut product = empty_product(" SKU-123 ");
        product.name = Some("Coffee beans".to_owned());
        product.price_minor_units = Some(1299);
        product.sale_price_minor_units = Some(999);
        product.currency = Some("usd".to_owned());
        product.availability = Some(ProductAvailability::InStock);
        product.condition = Some(ProductCondition::New);
        product.url = Some("https://shop.example/products/sku-123".to_owned());
        product.image_url = Some("https://cdn.example/sku-123.jpg".to_owned());
        let built = build_upsert_request(UpsertProductInput {
            product_catalog_id: "42".to_owned(),
            product,
            allow_upsert: true,
        })
        .unwrap();
        assert_eq!(built.retailer_id, "SKU-123");
        assert_eq!(built.request.endpoint, "42/products");
        assert_eq!(
            built.request.form,
            vec![
                ("retailer_id".to_owned(), "SKU-123".to_owned()),
                ("name".to_owned(), "Coffee beans".to_owned()),
                ("price".to_owned(), "1299".to_owned()),
                ("sale_price".to_owned(), "999".to_owned()),
                ("currency".to_owned(), "USD".to_owned()),
                ("availability".to_owned(), "in stock".to_owned()),
                ("condition".to_owned(), "new".to_owned()),
                (
                    "url".to_owned(),
                    "https://shop.example/products/sku-123".to_owned(),
                ),
                (
                    "image_url".to_owned(),
                    "https://cdn.example/sku-123.jpg".to_owned(),
                ),
                ("allow_upsert".to_owned(), "true".to_owned()),
            ]
        );
    }

    #[test]
    fn rejects_ambiguous_or_unsafe_single_product_input() {
        let product = empty_product("SKU-1");
        assert!(
            build_upsert_request(UpsertProductInput {
                product_catalog_id: "42".to_owned(),
                product,
                allow_upsert: true,
            })
            .is_err()
        );

        let mut product = empty_product("SKU-1");
        product.price_minor_units = Some(100);
        assert!(
            build_upsert_request(UpsertProductInput {
                product_catalog_id: "42".to_owned(),
                product,
                allow_upsert: true,
            })
            .is_err()
        );

        let mut product = empty_product("SKU-1");
        product.image_url = Some("http://cdn.example/image.jpg".to_owned());
        assert!(
            build_upsert_request(UpsertProductInput {
                product_catalog_id: "../42".to_owned(),
                product,
                allow_upsert: true,
            })
            .is_err()
        );
    }

    #[test]
    fn flattens_bounded_additional_fields_into_single_product_form() {
        let mut product = empty_product("SKU-1");
        product.additional_fields = Some(
            json!({
                "color": "navy",
                "custom_label_0": ["launch", 2],
                "inventory": true
            })
            .as_object()
            .unwrap()
            .clone(),
        );
        let built = build_upsert_request(UpsertProductInput {
            product_catalog_id: "42".to_owned(),
            product,
            allow_upsert: true,
        })
        .unwrap();
        assert_eq!(
            built.request.form,
            vec![
                ("retailer_id".to_owned(), "SKU-1".to_owned()),
                ("color".to_owned(), "navy".to_owned()),
                ("custom_label_0".to_owned(), "[\"launch\",2]".to_owned(),),
                ("inventory".to_owned(), "true".to_owned()),
                ("allow_upsert".to_owned(), "true".to_owned()),
            ]
        );
    }

    #[test]
    fn rejects_unsafe_or_oversized_single_additional_fields() {
        for fields in [
            json!({}).as_object().unwrap().clone(),
            json!({"batch": []}).as_object().unwrap().clone(),
            json!({"execution_options": ["validate_only"]})
                .as_object()
                .unwrap()
                .clone(),
            json!({"suppress_http_code": true})
                .as_object()
                .unwrap()
                .clone(),
            json!({"relative_url": "me"}).as_object().unwrap().clone(),
            json!({"name": "typed collision"})
                .as_object()
                .unwrap()
                .clone(),
            json!({"metadata": {"catalog_access_token": "secret"}})
                .as_object()
                .unwrap()
                .clone(),
            json!({"wa_compliance_category": "commerce"})
                .as_object()
                .unwrap()
                .clone(),
            json!({"destination": "https://wa.me/15551234567"})
                .as_object()
                .unwrap()
                .clone(),
            json!({"color": null}).as_object().unwrap().clone(),
            json!({"vertical_attribute": "x".repeat(2_049)})
                .as_object()
                .unwrap()
                .clone(),
        ] {
            let mut product = empty_product("SKU-1");
            product.additional_fields = Some(fields);
            assert!(
                build_upsert_request(UpsertProductInput {
                    product_catalog_id: "42".to_owned(),
                    product,
                    allow_upsert: true,
                })
                .is_err()
            );
        }

        let fields = (0..=MAX_SINGLE_ADDITIONAL_FIELDS)
            .map(|index| (format!("custom_field_{index}"), json!(index)))
            .collect();
        let mut product = empty_product("SKU-1");
        product.additional_fields = Some(fields);
        assert!(
            build_upsert_request(UpsertProductInput {
                product_catalog_id: "42".to_owned(),
                product,
                allow_upsert: true,
            })
            .is_err()
        );
    }

    #[test]
    fn builds_exact_typed_mixed_batch() {
        assert!(
            build_batch_request(BatchProductsInput {
                product_catalog_id: "42".to_owned(),
                operations: vec![ProductBatchOperation::Delete {
                    retailer_id: "SKU-2".to_owned(),
                }],
                removal_acknowledgement: None,
                allow_upsert: false,
            })
            .is_err()
        );

        let mut upsert_only_product = empty_batch_product();
        upsert_only_product.title = Some("Coffee beans".to_owned());
        assert!(
            build_batch_request(BatchProductsInput {
                product_catalog_id: "42".to_owned(),
                operations: vec![ProductBatchOperation::Upsert {
                    retailer_id: "SKU-1".to_owned(),
                    product: Box::new(upsert_only_product),
                }],
                removal_acknowledgement: None,
                allow_upsert: true,
            })
            .is_ok()
        );

        let mut product = empty_batch_product();
        product.title = Some("Coffee beans".to_owned());
        product.price_amount = Some("12.99".to_owned());
        product.currency = Some("usd".to_owned());
        product.availability = Some(ProductAvailability::InStock);
        product.additional_fields = Some(
            json!({
                "color": "navy",
                "custom_label_0": ["launch", 2]
            })
            .as_object()
            .unwrap()
            .clone(),
        );
        let built = build_batch_request(BatchProductsInput {
            product_catalog_id: "42".to_owned(),
            operations: vec![
                ProductBatchOperation::Upsert {
                    retailer_id: "SKU-1".to_owned(),
                    product: Box::new(product),
                },
                ProductBatchOperation::Delete {
                    retailer_id: "SKU-2".to_owned(),
                },
            ],
            removal_acknowledgement: Some(REMOVAL_ACKNOWLEDGEMENT.to_owned()),
            allow_upsert: false,
        })
        .unwrap();
        assert_eq!(built.request.endpoint, "42/items_batch");
        assert_eq!(built.submitted, 2);
        assert_eq!(built.deletions, 1);
        assert_eq!(built.request.form[0].1, "false");
        assert_eq!(built.request.form[1].1, "PRODUCT_ITEM");
        assert_eq!(
            serde_json::from_str::<Value>(&built.request.form[2].1).unwrap(),
            json!([
                {
                    "method": "UPDATE",
                    "retailer_id": "SKU-1",
                    "data": {
                        "title": "Coffee beans",
                        "price": "12.99 USD",
                        "availability": "in stock",
                        "color": "navy",
                        "custom_label_0": ["launch", 2]
                    }
                },
                {"method": "DELETE", "retailer_id": "SKU-2"}
            ])
        );
    }

    #[test]
    fn rejects_unsafe_batch_additional_fields() {
        for fields in [
            json!({"title": "typed collision"})
                .as_object()
                .unwrap()
                .clone(),
            json!({"metadata": {"merchant_client_secret": "secret"}})
                .as_object()
                .unwrap()
                .clone(),
            json!({"messaging_channel": "whatsapp"})
                .as_object()
                .unwrap()
                .clone(),
        ] {
            let mut product = empty_batch_product();
            product.additional_fields = Some(fields);
            assert!(provider_batch_product(product).is_err());
        }
    }

    #[test]
    fn enforces_batch_uniqueness_and_encoded_body_cap() {
        let duplicate = BatchProductsInput {
            product_catalog_id: "42".to_owned(),
            operations: vec![
                ProductBatchOperation::Delete {
                    retailer_id: "SKU-1".to_owned(),
                },
                ProductBatchOperation::Delete {
                    retailer_id: " SKU-1 ".to_owned(),
                },
            ],
            removal_acknowledgement: Some(REMOVAL_ACKNOWLEDGEMENT.to_owned()),
            allow_upsert: true,
        };
        assert!(build_batch_request(duplicate).is_err());

        let operations = (0..9)
            .map(|index| {
                let mut product = empty_batch_product();
                product.description = Some("x".repeat(5_000));
                ProductBatchOperation::Upsert {
                    retailer_id: format!("SKU-{index}"),
                    product: Box::new(product),
                }
            })
            .collect();
        assert!(
            build_batch_request(BatchProductsInput {
                product_catalog_id: "42".to_owned(),
                operations,
                removal_acknowledgement: None,
                allow_upsert: true,
            })
            .is_err()
        );

        let operations = (0..21)
            .map(|index| {
                let mut product = empty_batch_product();
                product.additional_fields = Some(
                    json!({"vertical_attribute": "x".repeat(2_000)})
                        .as_object()
                        .unwrap()
                        .clone(),
                );
                ProductBatchOperation::Upsert {
                    retailer_id: format!("EXTRA-{index}"),
                    product: Box::new(product),
                }
            })
            .collect();
        assert!(
            build_batch_request(BatchProductsInput {
                product_catalog_id: "42".to_owned(),
                operations,
                removal_acknowledgement: None,
                allow_upsert: true,
            })
            .is_err()
        );
    }

    #[test]
    fn accepts_only_compact_provider_confirmations() {
        assert_eq!(product_id(&json!({"id": 123})).as_deref(), Some("123"));
        assert_eq!(
            batch_handles(&json!({"handles": ["handle_1", "handle-2"]})).unwrap(),
            vec!["handle_1", "handle-2"]
        );
        assert!(batch_handles(&json!({"handles": []})).is_none());
        assert!(batch_handles(&json!({"handles": ["bad\nhandle"]})).is_none());
    }
}
