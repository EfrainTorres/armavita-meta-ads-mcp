// Copyright (C) 2025 ArmaVita LLC
// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    fmt::Write as _,
    net::IpAddr,
    time::{SystemTime, UNIX_EPOCH},
};

use reqwest::Url;
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::{
    bounded_json::{credential_value, encode_nonempty_object},
    error::{GraphError, PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::numeric_owned as numeric_id,
    mutation_result::{ambiguous_mutation_result, mutation_error_without_blind_retry},
};

const MAX_EVENTS: usize = 50;
const MAX_EVENT_NAME_CHARS: usize = 128;
const MAX_EVENT_ID_CHARS: usize = 256;
const MAX_URL_CHARS: usize = 4_096;
const MAX_VALUES_PER_FIELD: usize = 5;
const MAX_IDENTIFIERS_PER_EVENT: usize = 20;
const MAX_IDENTIFIER_CHARS: usize = 512;
const MAX_TECHNICAL_ID_CHARS: usize = 512;
const MAX_USER_AGENT_CHARS: usize = 1_024;
const MAX_CUSTOM_TEXT_CHARS: usize = 512;
const MAX_CONTENTS: usize = 20;
const MAX_CONTENT_ID_CHARS: usize = 256;
const MAX_NUM_ITEMS: u32 = 1_000_000;
const MAX_MONETARY_VALUE: f64 = 1_000_000_000_000.0;
const MAX_TEST_CODE_CHARS: usize = 128;
const MAX_PARTNER_AGENT_CHARS: usize = 256;
const MAX_RAW_INPUT_BYTES: usize = 128 * 1024;
// Form encoding can expand each byte threefold. Keep the JSON field comfortably
// inside GraphClient's 128 KiB mutation-body ceiling.
const MAX_DATA_JSON_BYTES: usize = 40 * 1024;
const MAX_PROVIDER_MESSAGES: usize = 100;
const MAX_EVENT_AGE_SECONDS: u64 = 7 * 24 * 60 * 60;
const MAX_CLOCK_SKEW_SECONDS: u64 = 5 * 60;
const MAX_UNIX_TIME: u64 = 4_102_444_800; // 2100-01-01T00:00:00Z

/// Submit one bounded batch of first-party server events. Raw matching data is
/// always normalized and SHA-256 hashed locally; pre-hashed SHA-256 values are
/// lowercased and passed through. This input intentionally does not implement
/// `Debug` because it may contain customer information.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SendCapiEventsInput {
    /// Numeric Meta Dataset or Pixel ID.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub dataset_id: String,
    /// One through 50 events. Use `event_id` for browser/server deduplication.
    #[schemars(length(min = 1, max = 50))]
    pub events: Vec<CapiEventInput>,
    /// Optional code from Events Manager Test Events. Omit for production events.
    #[schemars(length(min = 1, max = 128), regex(pattern = "^[A-Za-z0-9_-]{1,128}$"))]
    pub test_event_code: Option<String>,
    /// Optional platform identifier for a technology-provider integration.
    #[schemars(length(min = 1, max = 256))]
    pub partner_agent: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapiEventInput {
    /// Standard or custom event name.
    #[schemars(length(min = 1, max = 128))]
    pub event_name: String,
    /// Unix timestamp in seconds. Events older than seven days are rejected.
    #[schemars(range(min = 1, max = 4_102_444_800_u64))]
    pub event_time: u64,
    pub action_source: CapiActionSource,
    /// Required for `website`; must be credential-free HTTPS.
    #[schemars(length(min = 1, max = 4_096), url)]
    pub event_source_url: Option<String>,
    /// Optional credential-free HTTPS referrer URL.
    #[schemars(length(min = 1, max = 4_096), url)]
    pub referrer_url: Option<String>,
    /// Advertiser-generated deduplication ID shared with the matching browser event.
    #[schemars(length(min = 1, max = 256))]
    pub event_id: Option<String>,
    pub user_data: CapiUserDataInput,
    pub custom_data: Option<CapiCustomDataInput>,
    /// Provider-version-dependent Meta App Data object. Valid only for `app`
    /// events and bounded to 16 KiB, eight levels, and credential-free values.
    /// Keep customer matching data in typed `user_data` so it is hashed locally.
    pub app_data: Option<Map<String, Value>>,
    /// True limits use to attribution rather than ads-delivery optimization.
    pub opt_out: Option<bool>,
    /// Optional Limited Data Use setting. `automatic` asks Meta to determine
    /// location; named states emit Meta's documented US country/state codes.
    pub data_processing: Option<CapiDataProcessing>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapiActionSource {
    Website,
    App,
    Chat,
    BusinessMessaging,
    Email,
    PhoneCall,
    PhysicalStore,
    SystemGenerated,
    Other,
}

/// Meta currently documents Limited Data Use as the supported event-level
/// data-processing option. The adjacent tag keeps the option closed while the
/// location enum prevents incoherent country/state combinations.
#[derive(Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(
    tag = "option",
    content = "location",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum CapiDataProcessing {
    Ldu(CapiLduLocation),
}

#[derive(Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapiLduLocation {
    Automatic,
    California,
    Colorado,
    Connecticut,
    Florida,
    Oregon,
    Texas,
    Montana,
    Delaware,
    Nebraska,
    NewHampshire,
    NewJersey,
    Minnesota,
    Maryland,
    RhodeIsland,
}

impl CapiLduLocation {
    const fn wire_codes(self) -> (u16, u16) {
        match self {
            Self::Automatic => (0, 0),
            Self::California => (1, 1000),
            Self::Colorado => (1, 1001),
            Self::Connecticut => (1, 1002),
            Self::Florida => (1, 1003),
            Self::Oregon => (1, 1004),
            Self::Texas => (1, 1005),
            Self::Montana => (1, 1006),
            Self::Delaware => (1, 1007),
            Self::Nebraska => (1, 1008),
            Self::NewHampshire => (1, 1009),
            Self::NewJersey => (1, 1010),
            Self::Minnesota => (1, 1011),
            Self::Maryland => (1, 1012),
            Self::RhodeIsland => (1, 1013),
        }
    }
}

/// Matching identifiers. Hashable fields accept raw values or 64-character
/// SHA-256 hex digests and produce arrays under Meta's compact wire keys.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapiUserDataInput {
    #[schemars(length(min = 1, max = 5), inner(length(min = 1, max = 512)))]
    pub emails: Option<Vec<String>>,
    #[schemars(length(min = 1, max = 5), inner(length(min = 1, max = 512)))]
    pub phones: Option<Vec<String>>,
    #[schemars(length(min = 1, max = 5), inner(length(min = 1, max = 512)))]
    pub first_names: Option<Vec<String>>,
    #[schemars(length(min = 1, max = 5), inner(length(min = 1, max = 512)))]
    pub last_names: Option<Vec<String>>,
    #[schemars(length(min = 1, max = 5))]
    pub genders: Option<Vec<CapiGender>>,
    /// Raw dates use `YYYYMMDD`; pre-hashed SHA-256 values are also accepted.
    #[schemars(length(min = 1, max = 5), inner(length(min = 8, max = 64)))]
    pub dates_of_birth: Option<Vec<String>>,
    #[schemars(length(min = 1, max = 5), inner(length(min = 1, max = 512)))]
    pub cities: Option<Vec<String>>,
    #[schemars(length(min = 1, max = 5), inner(length(min = 1, max = 512)))]
    pub states: Option<Vec<String>>,
    #[schemars(length(min = 1, max = 5), inner(length(min = 1, max = 512)))]
    pub zip_codes: Option<Vec<String>>,
    #[schemars(length(min = 1, max = 5), inner(length(min = 2, max = 64)))]
    pub country_codes: Option<Vec<String>>,
    #[schemars(length(min = 1, max = 5), inner(length(min = 1, max = 512)))]
    pub external_ids: Option<Vec<String>>,
    #[schemars(length(min = 1, max = 64))]
    pub client_ip_address: Option<String>,
    #[schemars(length(min = 1, max = 1_024))]
    pub client_user_agent: Option<String>,
    #[schemars(length(min = 1, max = 512))]
    pub fbc: Option<String>,
    #[schemars(length(min = 1, max = 512))]
    pub fbp: Option<String>,
    #[schemars(length(min = 1, max = 512))]
    pub subscription_id: Option<String>,
    #[schemars(length(min = 1, max = 512))]
    pub fb_login_id: Option<String>,
    #[schemars(length(min = 1, max = 512))]
    pub lead_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapiGender {
    Male,
    Female,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapiCustomDataInput {
    #[schemars(range(min = 0.0, max = 1_000_000_000_000.0))]
    pub value: Option<f64>,
    /// ISO 4217 currency code. Required when `value` is present.
    #[schemars(length(min = 3, max = 3), regex(pattern = "^[A-Za-z]{3}$"))]
    pub currency: Option<String>,
    #[schemars(length(min = 1, max = 512))]
    pub content_name: Option<String>,
    #[schemars(length(min = 1, max = 512))]
    pub content_category: Option<String>,
    #[schemars(length(min = 1, max = 20), inner(length(min = 1, max = 256)))]
    pub content_ids: Option<Vec<String>>,
    #[schemars(length(min = 1, max = 20))]
    pub contents: Option<Vec<CapiContentInput>>,
    pub content_type: Option<CapiContentType>,
    #[schemars(length(min = 1, max = 512))]
    pub order_id: Option<String>,
    #[schemars(range(min = 1, max = 1_000_000_u32))]
    pub num_items: Option<u32>,
    #[schemars(length(min = 1, max = 512))]
    pub search_string: Option<String>,
    #[schemars(length(min = 1, max = 512))]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapiContentType {
    Product,
    ProductGroup,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapiContentInput {
    #[schemars(length(min = 1, max = 256))]
    pub product_id: String,
    #[schemars(range(min = 1, max = 1_000_000_u32))]
    pub quantity: Option<u32>,
    #[schemars(range(min = 0.0, max = 1_000_000_000_000.0))]
    pub item_price: Option<f64>,
    pub delivery_category: Option<CapiDeliveryCategory>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapiDeliveryCategory {
    InStore,
    Curbside,
    HomeDelivery,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SentCapiEvents {
    pub dataset_id: String,
    pub submitted_events: u16,
    pub events_received: u16,
    pub test_mode: bool,
    pub accepted: bool,
    pub provider_message_count: u16,
}

struct CapiRequest {
    endpoint: String,
    form: Vec<(String, String)>,
    dataset_id: String,
    submitted_events: u16,
    test_mode: bool,
}

#[derive(Serialize)]
struct NormalizedEvent {
    event_name: String,
    event_time: u64,
    action_source: CapiActionSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_source_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    referrer_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_id: Option<String>,
    user_data: NormalizedUserData,
    #[serde(skip_serializing_if = "Option::is_none")]
    custom_data: Option<NormalizedCustomData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    app_data: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    opt_out: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_processing_options: Option<[&'static str; 1]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_processing_options_country: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_processing_options_state: Option<u16>,
}

#[derive(Serialize)]
struct NormalizedUserData {
    #[serde(rename = "em", skip_serializing_if = "Option::is_none")]
    emails: Option<Vec<String>>,
    #[serde(rename = "ph", skip_serializing_if = "Option::is_none")]
    phones: Option<Vec<String>>,
    #[serde(rename = "fn", skip_serializing_if = "Option::is_none")]
    first_names: Option<Vec<String>>,
    #[serde(rename = "ln", skip_serializing_if = "Option::is_none")]
    last_names: Option<Vec<String>>,
    #[serde(rename = "ge", skip_serializing_if = "Option::is_none")]
    genders: Option<Vec<String>>,
    #[serde(rename = "db", skip_serializing_if = "Option::is_none")]
    dates_of_birth: Option<Vec<String>>,
    #[serde(rename = "ct", skip_serializing_if = "Option::is_none")]
    cities: Option<Vec<String>>,
    #[serde(rename = "st", skip_serializing_if = "Option::is_none")]
    states: Option<Vec<String>>,
    #[serde(rename = "zp", skip_serializing_if = "Option::is_none")]
    zip_codes: Option<Vec<String>>,
    #[serde(rename = "country", skip_serializing_if = "Option::is_none")]
    country_codes: Option<Vec<String>>,
    #[serde(rename = "external_id", skip_serializing_if = "Option::is_none")]
    external_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_ip_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_user_agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fbc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fbp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subscription_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fb_login_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lead_id: Option<String>,
}

#[derive(Serialize)]
struct NormalizedCustomData {
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    contents: Option<Vec<NormalizedContent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_type: Option<CapiContentType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    order_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_items: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    search_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
}

#[derive(Serialize)]
struct NormalizedContent {
    #[serde(rename = "id")]
    product_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    quantity: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    item_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delivery_category: Option<CapiDeliveryCategory>,
}

pub(crate) async fn send_capi_events(
    graph: &GraphClient,
    input: SendCapiEventsInput,
) -> ToolResponse<SentCapiEvents> {
    let request = match build_request(input, unix_now()) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(private_capi_graph_error(error));
        }
    };
    match parse_ack(
        &payload,
        request.dataset_id,
        request.submitted_events,
        request.test_mode,
    ) {
        Ok(ack) => ToolResponse::success(ack),
        Err(error) => ToolResponse::error(error),
    }
}

fn build_request(input: SendCapiEventsInput, now: u64) -> Result<CapiRequest, PublicError> {
    let dataset_id = numeric_id(&input.dataset_id).ok_or_else(|| {
        PublicError::invalid_input(
            "dataset_id must be a numeric Meta Dataset or Pixel ID",
            "Use an ID returned by list_business_datasets or Events Manager",
        )
    })?;
    if input.events.is_empty() || input.events.len() > MAX_EVENTS {
        return Err(PublicError::invalid_input(
            "events must contain 1 through 50 items",
            "Split larger uploads into bounded batches with stable event_id values",
        ));
    }

    let mut raw_bytes = input.dataset_id.len();
    let mut events = Vec::with_capacity(input.events.len());
    for event in input.events {
        events.push(normalize_event(event, now, &mut raw_bytes)?);
    }
    let data = serde_json::to_string(&events).map_err(|_| encoding_error())?;
    if data.len() > MAX_DATA_JSON_BYTES {
        return Err(PublicError::invalid_input(
            "encoded CAPI events exceed the 40 KiB safety limit",
            "Send fewer events or remove unused matching and custom fields",
        ));
    }

    let mut form = vec![("data".to_owned(), data)];
    let test_mode = input.test_event_code.is_some();
    if let Some(partner_agent) = input.partner_agent {
        let partner_agent = bounded_text(
            partner_agent,
            MAX_PARTNER_AGENT_CHARS,
            "partner_agent",
            &mut raw_bytes,
        )?;
        form.push(("partner_agent".to_owned(), partner_agent));
    }
    if let Some(code) = input.test_event_code {
        charge_raw(&mut raw_bytes, code.len())?;
        let code = bounded_ascii_identifier(
            code,
            MAX_TEST_CODE_CHARS,
            "test_event_code",
            "Use the code shown in Events Manager Test Events",
        )?;
        if !code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(PublicError::invalid_input(
                "test_event_code contains unsupported characters",
                "Use the code shown in Events Manager Test Events",
            ));
        }
        form.push(("test_event_code".to_owned(), code));
    }

    Ok(CapiRequest {
        endpoint: format!("{dataset_id}/events"),
        form,
        dataset_id,
        submitted_events: u16::try_from(events.len()).expect("event limit fits in u16"),
        test_mode,
    })
}

fn normalize_event(
    input: CapiEventInput,
    now: u64,
    raw_bytes: &mut usize,
) -> Result<NormalizedEvent, PublicError> {
    let event_name = bounded_text(
        input.event_name,
        MAX_EVENT_NAME_CHARS,
        "event_name",
        raw_bytes,
    )?;
    validate_event_time(input.event_time, now)?;
    let event_source_url = input
        .event_source_url
        .map(|url| normalize_https_url(url, "event_source_url", raw_bytes))
        .transpose()?;
    if input.action_source == CapiActionSource::Website && event_source_url.is_none() {
        return Err(PublicError::invalid_input(
            "event_source_url is required for website events",
            "Provide the credential-free HTTPS page URL where the event occurred",
        ));
    }
    let referrer_url = input
        .referrer_url
        .map(|url| normalize_https_url(url, "referrer_url", raw_bytes))
        .transpose()?;
    let event_id = input
        .event_id
        .map(|value| bounded_text(value, MAX_EVENT_ID_CHARS, "event_id", raw_bytes))
        .transpose()?;
    let user_data = normalize_user_data(input.user_data, raw_bytes)?;
    let custom_data = input
        .custom_data
        .map(|data| normalize_custom_data(data, raw_bytes))
        .transpose()?;
    let app_data = normalize_app_data(input.app_data, input.action_source, raw_bytes)?;
    let (data_processing_options, data_processing_options_country, data_processing_options_state) =
        match input.data_processing {
            Some(CapiDataProcessing::Ldu(location)) => {
                let (country, state) = location.wire_codes();
                (Some(["LDU"]), Some(country), Some(state))
            }
            None => (None, None, None),
        };

    Ok(NormalizedEvent {
        event_name,
        event_time: input.event_time,
        action_source: input.action_source,
        event_source_url,
        referrer_url,
        event_id,
        user_data,
        custom_data,
        app_data,
        opt_out: input.opt_out,
        data_processing_options,
        data_processing_options_country,
        data_processing_options_state,
    })
}

fn normalize_app_data(
    input: Option<Map<String, Value>>,
    action_source: CapiActionSource,
    raw_bytes: &mut usize,
) -> Result<Option<Map<String, Value>>, PublicError> {
    let Some(app_data) = input else {
        return Ok(None);
    };
    if action_source != CapiActionSource::App {
        return Err(PublicError::invalid_input(
            "app_data is valid only when action_source is app",
            "Use action_source app or omit app_data",
        ));
    }
    let encoded = encode_nonempty_object(&app_data, "app_data")?;
    if contains_forbidden_app_data(&app_data) {
        return Err(PublicError::invalid_input(
            "app_data contains credential-like, customer-matching, or unsupported messaging data",
            "Use current Meta App Data fields, typed user_data for matching, and no credentials",
        ));
    }
    charge_raw(raw_bytes, encoded.len())?;
    Ok(Some(app_data))
}

fn contains_forbidden_app_data(object: &Map<String, Value>) -> bool {
    let mut stack = Vec::with_capacity(object.len());
    for (key, value) in object {
        if forbidden_app_data_key(key) {
            return true;
        }
        stack.push(value);
    }
    while let Some(value) = stack.pop() {
        match value {
            Value::Object(object) => {
                for (key, value) in object {
                    if forbidden_app_data_key(key) {
                        return true;
                    }
                    stack.push(value);
                }
            }
            Value::Array(values) => stack.extend(values),
            Value::String(value) if forbidden_app_data_value(value) => return true,
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
    false
}

fn forbidden_app_data_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace('-', "_");
    key.contains("whatsapp")
        || key.contains("whats_app")
        || key.contains("ctwa")
        || key == "messaging_channel"
        || matches!(
            key.as_str(),
            "user_data"
                | "email"
                | "emails"
                | "em"
                | "phone"
                | "phones"
                | "ph"
                | "first_name"
                | "first_names"
                | "fn"
                | "last_name"
                | "last_names"
                | "ln"
                | "gender"
                | "genders"
                | "ge"
                | "date_of_birth"
                | "dates_of_birth"
                | "db"
                | "city"
                | "cities"
                | "ct"
                | "state"
                | "states"
                | "st"
                | "zip"
                | "zip_code"
                | "zip_codes"
                | "zp"
                | "country"
                | "country_code"
                | "country_codes"
                | "external_id"
                | "external_ids"
                | "client_ip_address"
                | "client_user_agent"
                | "fbc"
                | "fbp"
                | "subscription_id"
                | "fb_login_id"
                | "lead_id"
        )
}

fn forbidden_app_data_value(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("whatsapp")
        || lower.contains("whats_app")
        || lower.contains("whats-app")
        || lower.contains("wa.me/")
}

pub(crate) fn normalized_user_data_json(
    input: CapiUserDataInput,
) -> Result<serde_json::Value, PublicError> {
    let normalized = normalize_user_data(input, &mut 0)?;
    serde_json::to_value(normalized).map_err(|_| {
        PublicError::invalid_input(
            "user_data could not be encoded",
            "Use the typed matching fields",
        )
    })
}

fn normalize_user_data(
    input: CapiUserDataInput,
    raw_bytes: &mut usize,
) -> Result<NormalizedUserData, PublicError> {
    let mut identifier_count = 0_usize;
    let emails = normalize_hashable_values(
        input.emails,
        PiiKind::Email,
        &mut identifier_count,
        raw_bytes,
    )?;
    let phones = normalize_hashable_values(
        input.phones,
        PiiKind::Phone,
        &mut identifier_count,
        raw_bytes,
    )?;
    let first_names = normalize_hashable_values(
        input.first_names,
        PiiKind::FirstName,
        &mut identifier_count,
        raw_bytes,
    )?;
    let last_names = normalize_hashable_values(
        input.last_names,
        PiiKind::LastName,
        &mut identifier_count,
        raw_bytes,
    )?;
    let genders = normalize_genders(input.genders, &mut identifier_count)?;
    let dates_of_birth = normalize_hashable_values(
        input.dates_of_birth,
        PiiKind::DateOfBirth,
        &mut identifier_count,
        raw_bytes,
    )?;
    let cities = normalize_hashable_values(
        input.cities,
        PiiKind::City,
        &mut identifier_count,
        raw_bytes,
    )?;
    let states = normalize_hashable_values(
        input.states,
        PiiKind::State,
        &mut identifier_count,
        raw_bytes,
    )?;
    let zip_codes = normalize_hashable_values(
        input.zip_codes,
        PiiKind::Zip,
        &mut identifier_count,
        raw_bytes,
    )?;
    let country_codes = normalize_hashable_values(
        input.country_codes,
        PiiKind::Country,
        &mut identifier_count,
        raw_bytes,
    )?;
    let external_ids = normalize_hashable_values(
        input.external_ids,
        PiiKind::ExternalId,
        &mut identifier_count,
        raw_bytes,
    )?;

    let client_ip_address = input
        .client_ip_address
        .map(|value| normalize_ip(value, raw_bytes))
        .transpose()?;
    let client_user_agent = input
        .client_user_agent
        .map(|value| {
            bounded_printable_text(value, MAX_USER_AGENT_CHARS, "client_user_agent", raw_bytes)
        })
        .transpose()?;
    let fbc = normalize_plain_identifier(input.fbc, "fbc", raw_bytes)?;
    let fbp = normalize_plain_identifier(input.fbp, "fbp", raw_bytes)?;
    let subscription_id =
        normalize_plain_identifier(input.subscription_id, "subscription_id", raw_bytes)?;
    let fb_login_id = normalize_plain_identifier(input.fb_login_id, "fb_login_id", raw_bytes)?;
    let lead_id = normalize_plain_identifier(input.lead_id, "lead_id", raw_bytes)?;

    identifier_count = identifier_count
        .saturating_add(usize::from(client_ip_address.is_some()))
        .saturating_add(usize::from(client_user_agent.is_some()))
        .saturating_add(usize::from(fbc.is_some()))
        .saturating_add(usize::from(fbp.is_some()))
        .saturating_add(usize::from(subscription_id.is_some()))
        .saturating_add(usize::from(fb_login_id.is_some()))
        .saturating_add(usize::from(lead_id.is_some()));
    if identifier_count == 0 || identifier_count > MAX_IDENTIFIERS_PER_EVENT {
        return Err(PublicError::invalid_input(
            "user_data must contain 1 through 20 matching identifiers",
            "Provide only identifiers needed to match this event",
        ));
    }

    Ok(NormalizedUserData {
        emails,
        phones,
        first_names,
        last_names,
        genders,
        dates_of_birth,
        cities,
        states,
        zip_codes,
        country_codes,
        external_ids,
        client_ip_address,
        client_user_agent,
        fbc,
        fbp,
        subscription_id,
        fb_login_id,
        lead_id,
    })
}

fn normalize_custom_data(
    input: CapiCustomDataInput,
    raw_bytes: &mut usize,
) -> Result<NormalizedCustomData, PublicError> {
    if input.value.is_some() != input.currency.is_some() {
        return Err(PublicError::invalid_input(
            "custom_data value and currency must be provided together",
            "Provide both fields for monetary events or omit both",
        ));
    }
    if input
        .value
        .is_some_and(|value| !value.is_finite() || !(0.0..=MAX_MONETARY_VALUE).contains(&value))
    {
        return Err(invalid_custom_data());
    }
    let currency = input
        .currency
        .map(|value| normalize_currency(value, raw_bytes))
        .transpose()?;
    let content_name =
        normalize_optional_custom_text(input.content_name, "content_name", raw_bytes)?;
    let content_category =
        normalize_optional_custom_text(input.content_category, "content_category", raw_bytes)?;
    let content_ids = input
        .content_ids
        .map(|values| normalize_content_ids(values, raw_bytes))
        .transpose()?;
    let contents = input
        .contents
        .map(|values| normalize_contents(values, raw_bytes))
        .transpose()?;
    let order_id = normalize_optional_custom_text(input.order_id, "order_id", raw_bytes)?;
    let search_string =
        normalize_optional_custom_text(input.search_string, "search_string", raw_bytes)?;
    let status = normalize_optional_custom_text(input.status, "status", raw_bytes)?;
    if input
        .num_items
        .is_some_and(|value| value == 0 || value > MAX_NUM_ITEMS)
    {
        return Err(invalid_custom_data());
    }

    let populated = input.value.is_some()
        || content_name.is_some()
        || content_category.is_some()
        || content_ids.is_some()
        || contents.is_some()
        || input.content_type.is_some()
        || order_id.is_some()
        || input.num_items.is_some()
        || search_string.is_some()
        || status.is_some();
    if !populated {
        return Err(PublicError::invalid_input(
            "custom_data cannot be empty",
            "Omit custom_data or provide a supported business field",
        ));
    }

    Ok(NormalizedCustomData {
        value: input.value,
        currency,
        content_name,
        content_category,
        content_ids,
        contents,
        content_type: input.content_type,
        order_id,
        num_items: input.num_items,
        search_string,
        status,
    })
}

fn normalize_contents(
    values: Vec<CapiContentInput>,
    raw_bytes: &mut usize,
) -> Result<Vec<NormalizedContent>, PublicError> {
    if values.is_empty() || values.len() > MAX_CONTENTS {
        return Err(invalid_custom_data());
    }
    values
        .into_iter()
        .map(|content| {
            if content
                .quantity
                .is_some_and(|value| value == 0 || value > MAX_NUM_ITEMS)
                || content.item_price.is_some_and(|value| {
                    !value.is_finite() || !(0.0..=MAX_MONETARY_VALUE).contains(&value)
                })
            {
                return Err(invalid_custom_data());
            }
            Ok(NormalizedContent {
                product_id: bounded_text(
                    content.product_id,
                    MAX_CONTENT_ID_CHARS,
                    "contents.product_id",
                    raw_bytes,
                )?,
                quantity: content.quantity,
                item_price: content.item_price,
                delivery_category: content.delivery_category,
            })
        })
        .collect()
}

fn normalize_content_ids(
    values: Vec<String>,
    raw_bytes: &mut usize,
) -> Result<Vec<String>, PublicError> {
    if values.is_empty() || values.len() > MAX_CONTENTS {
        return Err(invalid_custom_data());
    }
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let value = bounded_text(value, MAX_CONTENT_ID_CHARS, "content_ids", raw_bytes)?;
        if !normalized.contains(&value) {
            normalized.push(value);
        }
    }
    Ok(normalized)
}

#[derive(Clone, Copy)]
enum PiiKind {
    Email,
    Phone,
    FirstName,
    LastName,
    DateOfBirth,
    City,
    State,
    Zip,
    Country,
    ExternalId,
}

fn normalize_hashable_values(
    values: Option<Vec<String>>,
    kind: PiiKind,
    identifier_count: &mut usize,
    raw_bytes: &mut usize,
) -> Result<Option<Vec<String>>, PublicError> {
    let Some(values) = values else {
        return Ok(None);
    };
    if values.is_empty() || values.len() > MAX_VALUES_PER_FIELD {
        return Err(invalid_user_data());
    }
    *identifier_count = identifier_count.saturating_add(values.len());
    if *identifier_count > MAX_IDENTIFIERS_PER_EVENT {
        return Err(invalid_user_data());
    }
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        charge_raw(raw_bytes, value.len())?;
        if value.chars().count() > MAX_IDENTIFIER_CHARS {
            return Err(invalid_user_data());
        }
        let value = normalize_and_hash(kind, &value)?;
        if !normalized.contains(&value) {
            normalized.push(value);
        }
    }
    Ok(Some(normalized))
}

fn normalize_genders(
    values: Option<Vec<CapiGender>>,
    identifier_count: &mut usize,
) -> Result<Option<Vec<String>>, PublicError> {
    let Some(values) = values else {
        return Ok(None);
    };
    if values.is_empty() || values.len() > MAX_VALUES_PER_FIELD {
        return Err(invalid_user_data());
    }
    *identifier_count = identifier_count.saturating_add(values.len());
    if *identifier_count > MAX_IDENTIFIERS_PER_EVENT {
        return Err(invalid_user_data());
    }
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let digest = sha256_hex(match value {
            CapiGender::Male => b"m",
            CapiGender::Female => b"f",
        });
        if !normalized.contains(&digest) {
            normalized.push(digest);
        }
    }
    Ok(Some(normalized))
}

fn normalize_and_hash(kind: PiiKind, raw: &str) -> Result<String, PublicError> {
    let trimmed =
        raw.trim_matches(|character: char| character.is_whitespace() || character == '\0');
    if trimmed.is_empty() {
        return Err(invalid_user_data());
    }
    if is_sha256_hex(trimmed) {
        return Ok(trimmed.to_ascii_lowercase());
    }

    let normalized = match kind {
        PiiKind::Email => normalize_email(trimmed)?,
        PiiKind::Phone => normalize_phone(trimmed)?,
        PiiKind::FirstName | PiiKind::LastName | PiiKind::ExternalId => trimmed.to_lowercase(),
        PiiKind::DateOfBirth => normalize_date_of_birth(trimmed)?,
        PiiKind::City | PiiKind::State => trimmed
            .chars()
            .flat_map(char::to_lowercase)
            .filter(|character| character.is_alphabetic())
            .collect(),
        PiiKind::Zip => {
            let compact = trimmed
                .chars()
                .filter(|character| !character.is_whitespace())
                .flat_map(char::to_lowercase)
                .collect::<String>();
            let postal_code = compact.split('-').next().unwrap_or_default();
            if !postal_code.chars().all(char::is_alphanumeric) {
                return Err(invalid_user_data());
            }
            postal_code.to_owned()
        }
        PiiKind::Country => {
            let country = trimmed.to_ascii_lowercase();
            if country.len() != 2 || !country.bytes().all(|byte| byte.is_ascii_alphabetic()) {
                return Err(invalid_user_data());
            }
            country
        }
    };
    if normalized.is_empty() {
        return Err(invalid_user_data());
    }
    Ok(sha256_hex(normalized.as_bytes()))
}

fn normalize_email(raw: &str) -> Result<String, PublicError> {
    let email = raw.to_ascii_lowercase();
    let Some((local, domain)) = email.split_once('@') else {
        return Err(invalid_user_data());
    };
    if local.is_empty()
        || domain.starts_with('.')
        || domain.ends_with('.')
        || !domain.contains('.')
        || email.matches('@').count() != 1
        || !email.is_ascii()
        || email.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        return Err(invalid_user_data());
    }
    Ok(email)
}

fn normalize_phone(raw: &str) -> Result<String, PublicError> {
    let digits = raw
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(char::from)
        .collect::<String>();
    let digits = digits.trim_start_matches('0');
    if !(7..=15).contains(&digits.len()) {
        return Err(invalid_user_data());
    }
    Ok(digits.to_owned())
}

fn normalize_date_of_birth(raw: &str) -> Result<String, PublicError> {
    let digits = raw
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(char::from)
        .collect::<String>();
    if digits.len() != 8 {
        return Err(invalid_user_data());
    }
    let year = digits[0..4]
        .parse::<u16>()
        .map_err(|_| invalid_user_data())?;
    let month = digits[4..6]
        .parse::<u8>()
        .map_err(|_| invalid_user_data())?;
    let day = digits[6..8]
        .parse::<u8>()
        .map_err(|_| invalid_user_data())?;
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return Err(invalid_user_data()),
    };
    if !(1900..=2100).contains(&year) || day == 0 || day > max_day {
        return Err(invalid_user_data());
    }
    Ok(digits)
}

fn normalize_plain_identifier(
    value: Option<String>,
    field: &str,
    raw_bytes: &mut usize,
) -> Result<Option<String>, PublicError> {
    value
        .map(|value| {
            charge_raw(raw_bytes, value.len())?;
            bounded_ascii_identifier(
                value,
                MAX_TECHNICAL_ID_CHARS,
                field,
                "Use the unmodified first-party identifier supplied for this event",
            )
        })
        .transpose()
}

fn normalize_ip(value: String, raw_bytes: &mut usize) -> Result<String, PublicError> {
    charge_raw(raw_bytes, value.len())?;
    value
        .trim()
        .parse::<IpAddr>()
        .map(|ip| ip.to_string())
        .map_err(|_| {
            PublicError::invalid_input(
                "client_ip_address is not a valid IPv4 or IPv6 address",
                "Provide the originating client address without a port or proxy chain",
            )
        })
}

fn normalize_currency(value: String, raw_bytes: &mut usize) -> Result<String, PublicError> {
    charge_raw(raw_bytes, value.len())?;
    let currency = value.trim().to_ascii_lowercase();
    if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return Err(invalid_custom_data());
    }
    Ok(currency)
}

fn normalize_optional_custom_text(
    value: Option<String>,
    field: &str,
    raw_bytes: &mut usize,
) -> Result<Option<String>, PublicError> {
    value
        .map(|value| bounded_text(value, MAX_CUSTOM_TEXT_CHARS, field, raw_bytes))
        .transpose()
}

fn normalize_https_url(
    value: String,
    field: &str,
    raw_bytes: &mut usize,
) -> Result<String, PublicError> {
    charge_raw(raw_bytes, value.len())?;
    if value.chars().count() > MAX_URL_CHARS {
        return Err(invalid_url(field));
    }
    let url = Url::parse(value.trim()).map_err(|_| invalid_url(field))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || credential_value(&value)
    {
        return Err(invalid_url(field));
    }
    Ok(url.to_string())
}

fn bounded_text(
    value: String,
    max_chars: usize,
    field: &str,
    raw_bytes: &mut usize,
) -> Result<String, PublicError> {
    charge_raw(raw_bytes, value.len())?;
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > max_chars
        || trimmed.chars().any(char::is_control)
    {
        return Err(PublicError::invalid_input(
            format!("{field} must contain 1 through {max_chars} printable characters"),
            format!("Provide a bounded {field}"),
        ));
    }
    if trimmed.len() == value.len() {
        Ok(value)
    } else {
        Ok(trimmed.to_owned())
    }
}

fn bounded_printable_text(
    value: String,
    max_chars: usize,
    field: &str,
    raw_bytes: &mut usize,
) -> Result<String, PublicError> {
    let value = bounded_text(value, max_chars, field, raw_bytes)?;
    if value.chars().any(char::is_control) {
        return Err(invalid_user_data());
    }
    Ok(value)
}

fn bounded_ascii_identifier(
    value: String,
    max_chars: usize,
    field: &str,
    action: &str,
) -> Result<String, PublicError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > max_chars
        || !trimmed.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(PublicError::invalid_input(
            format!("{field} is empty, oversized, or contains whitespace"),
            action,
        ));
    }
    if trimmed.len() == value.len() {
        Ok(value)
    } else {
        Ok(trimmed.to_owned())
    }
}

fn validate_event_time(event_time: u64, now: u64) -> Result<(), PublicError> {
    if event_time == 0
        || event_time > MAX_UNIX_TIME
        || event_time < now.saturating_sub(MAX_EVENT_AGE_SECONDS)
        || event_time > now.saturating_add(MAX_CLOCK_SKEW_SECONDS)
    {
        return Err(PublicError::invalid_input(
            "event_time must be within the last seven days and not in the future",
            "Provide the Unix timestamp when the conversion occurred",
        ));
    }
    Ok(())
}

fn parse_ack(
    payload: &Value,
    dataset_id: String,
    submitted_events: u16,
    test_mode: bool,
) -> Result<SentCapiEvents, PublicError> {
    let received = payload
        .get("events_received")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| *value <= submitted_events)
        .ok_or_else(ambiguous_ack)?;
    let provider_message_count = match payload.get("messages") {
        None | Some(Value::Null) => 0,
        Some(Value::Array(messages)) if messages.len() <= MAX_PROVIDER_MESSAGES => {
            u16::try_from(messages.len()).expect("provider-message limit fits in u16")
        }
        _ => return Err(ambiguous_ack()),
    };
    Ok(SentCapiEvents {
        dataset_id,
        submitted_events,
        events_received: received,
        test_mode,
        accepted: received == submitted_events,
        provider_message_count,
    })
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn sha256_hex(value: &[u8]) -> String {
    let digest = Sha256::digest(value);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn charge_raw(total: &mut usize, bytes: usize) -> Result<(), PublicError> {
    *total = total.checked_add(bytes).ok_or_else(input_too_large)?;
    if *total > MAX_RAW_INPUT_BYTES {
        return Err(input_too_large());
    }
    Ok(())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn private_capi_graph_error(error: GraphError) -> PublicError {
    let mut error = mutation_error_without_blind_retry(
        error,
        "Check Events Manager using event_id or the test-event code before resubmitting",
    );
    error.message = match error.code.as_str() {
        "AUTH_REQUIRED" => "Meta authentication is not configured".to_owned(),
        "AUTH_EXPIRED" => "Meta authentication expired during the CAPI request".to_owned(),
        "INVALID_INPUT" => "The CAPI request failed local safety validation".to_owned(),
        "RESPONSE_TOO_LARGE" => "Meta returned an oversized CAPI response".to_owned(),
        "META_UNAVAILABLE" => "The CAPI request did not complete".to_owned(),
        _ => "Meta rejected the CAPI event batch".to_owned(),
    };
    error.retryable = false;
    error.action = Some(
        if matches!(error.code.as_str(), "AUTH_REQUIRED" | "AUTH_EXPIRED") {
            "Configure or refresh Meta authentication before sending a new event batch".to_owned()
        } else {
            "Check Events Manager using event_id or the test-event code before resubmitting"
                .to_owned()
        },
    );
    error
}

fn ambiguous_ack() -> PublicError {
    ambiguous_mutation_result(
        "Meta did not return a valid CAPI receipt count",
        "Check Events Manager using event_id or the test-event code before resubmitting",
    )
}

fn invalid_user_data() -> PublicError {
    PublicError::invalid_input(
        "user_data contains an invalid, empty, or oversized identifier",
        "Check Meta's field-specific normalization requirements",
    )
}

fn invalid_custom_data() -> PublicError {
    PublicError::invalid_input(
        "custom_data contains an invalid, empty, or oversized value",
        "Use the bounded standard commerce fields",
    )
}

fn invalid_url(field: &str) -> PublicError {
    PublicError::invalid_input(
        format!("{field} must be a credential-free HTTPS URL without a fragment"),
        "Use the public page URL and remove embedded credentials",
    )
}

fn input_too_large() -> PublicError {
    PublicError::invalid_input(
        "raw CAPI input exceeds the 128 KiB safety limit",
        "Send fewer events or identifiers",
    )
}

fn encoding_error() -> PublicError {
    PublicError::invalid_input(
        "CAPI events could not be encoded",
        "Use only supported typed values",
    )
}

#[cfg(test)]
mod tests {
    use rmcp::schemars::schema_for;
    use serde_json::{Map, Value, json};

    use super::{
        CapiActionSource, CapiContentInput, CapiContentType, CapiCustomDataInput,
        CapiDataProcessing, CapiDeliveryCategory, CapiEventInput, CapiGender, CapiLduLocation,
        CapiUserDataInput, SendCapiEventsInput, build_request, parse_ack, private_capi_graph_error,
        sha256_hex,
    };
    use crate::error::GraphError;

    const NOW: u64 = 1_800_000_000;

    fn user_with_email(email: &str) -> CapiUserDataInput {
        CapiUserDataInput {
            emails: Some(vec![email.to_owned()]),
            phones: None,
            first_names: None,
            last_names: None,
            genders: None,
            dates_of_birth: None,
            cities: None,
            states: None,
            zip_codes: None,
            country_codes: None,
            external_ids: None,
            client_ip_address: None,
            client_user_agent: None,
            fbc: None,
            fbp: None,
            subscription_id: None,
            fb_login_id: None,
            lead_id: None,
        }
    }

    fn website_event(user_data: CapiUserDataInput) -> CapiEventInput {
        CapiEventInput {
            event_name: "Purchase".to_owned(),
            event_time: NOW - 10,
            action_source: CapiActionSource::Website,
            event_source_url: Some("https://shop.example/orders/42".to_owned()),
            referrer_url: None,
            event_id: Some("order-42".to_owned()),
            user_data,
            custom_data: None,
            app_data: None,
            opt_out: None,
            data_processing: None,
        }
    }

    fn request_with_event(event: CapiEventInput) -> SendCapiEventsInput {
        SendCapiEventsInput {
            dataset_id: "123".to_owned(),
            events: vec![event],
            test_event_code: None,
            partner_agent: None,
        }
    }

    #[test]
    fn builds_exact_private_payload_and_compact_form() {
        let mut user = user_with_email("  Alice@Example.COM ");
        user.phones = Some(vec!["+1 (555) 123-4567".to_owned()]);
        user.client_ip_address = Some("203.0.113.8".to_owned());
        user.client_user_agent = Some("ExampleBrowser/1.0".to_owned());
        user.subscription_id = Some("sub_abc123".to_owned());
        let mut event = website_event(user);
        event.custom_data = Some(CapiCustomDataInput {
            value: Some(29.99),
            currency: Some("USD".to_owned()),
            content_name: None,
            content_category: None,
            content_ids: Some(vec!["sku-1".to_owned()]),
            contents: Some(vec![CapiContentInput {
                product_id: "sku-1".to_owned(),
                quantity: Some(2),
                item_price: Some(14.995),
                delivery_category: Some(CapiDeliveryCategory::HomeDelivery),
            }]),
            content_type: Some(CapiContentType::Product),
            order_id: Some("order-42".to_owned()),
            num_items: Some(2),
            search_string: None,
            status: None,
        });
        let mut input = request_with_event(event);
        input.test_event_code = Some("TEST_42".to_owned());

        let request = build_request(input, NOW).unwrap();
        assert_eq!(request.endpoint, "123/events");
        assert_eq!(request.form.len(), 2);
        assert_eq!(
            request.form[1],
            ("test_event_code".to_owned(), "TEST_42".to_owned())
        );
        let data: Value = serde_json::from_str(&request.form[0].1).unwrap();
        assert_eq!(
            data,
            json!([{
                "event_name": "Purchase",
                "event_time": NOW - 10,
                "action_source": "website",
                "event_source_url": "https://shop.example/orders/42",
                "event_id": "order-42",
                "user_data": {
                    "em": ["ff8d9819fc0e12bf0d24892e45987e249a28dce836a85cad60e28eaaa8c6d976"],
                    "ph": ["d6736136ea896c1bfdc553e0e86e702c70d060d805696ca3e4e9e0961353860a"],
                    "client_ip_address": "203.0.113.8",
                    "client_user_agent": "ExampleBrowser/1.0",
                    "subscription_id": "sub_abc123"
                },
                "custom_data": {
                    "value": 29.99,
                    "currency": "usd",
                    "content_ids": ["sku-1"],
                    "contents": [{
                        "id": "sku-1",
                        "quantity": 2,
                        "item_price": 14.995,
                        "delivery_category": "home_delivery"
                    }],
                    "content_type": "product",
                    "order_id": "order-42",
                    "num_items": 2
                }
            }])
        );
        let encoded = &request.form[0].1;
        assert!(!encoded.contains("Alice"));
        assert!(!encoded.contains("555"));
    }

    #[test]
    fn emits_exact_v26_action_source_partner_and_ldu_wire_fields() {
        let mut app = website_event(user_with_email("a@b.com"));
        app.action_source = CapiActionSource::App;
        app.event_source_url = None;
        app.event_id = Some("app-event".to_owned());
        app.app_data = Some(
            serde_json::from_value(json!({
                "advertiser_tracking_enabled": true,
                "url_schemes": ["armavita"]
            }))
            .unwrap(),
        );
        app.data_processing = Some(CapiDataProcessing::Ldu(CapiLduLocation::Automatic));

        let mut chat = website_event(user_with_email("a@b.com"));
        chat.action_source = CapiActionSource::Chat;
        chat.event_source_url = None;
        chat.event_id = Some("chat-event".to_owned());

        let mut business_messaging = website_event(user_with_email("a@b.com"));
        business_messaging.action_source = CapiActionSource::BusinessMessaging;
        business_messaging.event_source_url = None;
        business_messaging.event_id = Some("business-message-event".to_owned());
        business_messaging.data_processing =
            Some(CapiDataProcessing::Ldu(CapiLduLocation::RhodeIsland));

        let input = SendCapiEventsInput {
            dataset_id: "123".to_owned(),
            events: vec![app, chat, business_messaging],
            test_event_code: None,
            partner_agent: Some("  armavita-rust/1.0  ".to_owned()),
        };
        let request = build_request(input, NOW).unwrap();

        assert_eq!(
            request.form[1],
            ("partner_agent".to_owned(), "armavita-rust/1.0".to_owned())
        );
        let email = sha256_hex(b"a@b.com");
        let data: Value = serde_json::from_str(&request.form[0].1).unwrap();
        assert_eq!(
            data,
            json!([
                {
                    "event_name": "Purchase",
                    "event_time": NOW - 10,
                    "action_source": "app",
                    "event_id": "app-event",
                    "user_data": {"em": [email]},
                    "app_data": {
                        "advertiser_tracking_enabled": true,
                        "url_schemes": ["armavita"]
                    },
                    "data_processing_options": ["LDU"],
                    "data_processing_options_country": 0,
                    "data_processing_options_state": 0
                },
                {
                    "event_name": "Purchase",
                    "event_time": NOW - 10,
                    "action_source": "chat",
                    "event_id": "chat-event",
                    "user_data": {"em": [email]}
                },
                {
                    "event_name": "Purchase",
                    "event_time": NOW - 10,
                    "action_source": "business_messaging",
                    "event_id": "business-message-event",
                    "user_data": {"em": [email]},
                    "data_processing_options": ["LDU"],
                    "data_processing_options_country": 1,
                    "data_processing_options_state": 1013
                }
            ])
        );
        assert!(!request.form[0].1.contains("a@b.com"));
    }

    #[test]
    fn maps_every_closed_ldu_location_to_a_coherent_wire_pair() {
        let parsed = serde_json::from_value::<CapiDataProcessing>(json!({
            "option": "ldu",
            "location": "california"
        }))
        .unwrap();
        assert!(matches!(
            parsed,
            CapiDataProcessing::Ldu(CapiLduLocation::California)
        ));

        let locations = [
            (CapiLduLocation::Automatic, (0, 0)),
            (CapiLduLocation::California, (1, 1000)),
            (CapiLduLocation::Colorado, (1, 1001)),
            (CapiLduLocation::Connecticut, (1, 1002)),
            (CapiLduLocation::Florida, (1, 1003)),
            (CapiLduLocation::Oregon, (1, 1004)),
            (CapiLduLocation::Texas, (1, 1005)),
            (CapiLduLocation::Montana, (1, 1006)),
            (CapiLduLocation::Delaware, (1, 1007)),
            (CapiLduLocation::Nebraska, (1, 1008)),
            (CapiLduLocation::NewHampshire, (1, 1009)),
            (CapiLduLocation::NewJersey, (1, 1010)),
            (CapiLduLocation::Minnesota, (1, 1011)),
            (CapiLduLocation::Maryland, (1, 1012)),
            (CapiLduLocation::RhodeIsland, (1, 1013)),
        ];

        for (location, expected) in locations {
            assert_eq!(location.wire_codes(), expected);
        }
    }

    #[test]
    fn keeps_provider_app_data_bounded_private_and_app_only() {
        let app_data = |value: Value| serde_json::from_value::<Map<String, Value>>(value).unwrap();

        let mut wrong_source = website_event(user_with_email("a@b.com"));
        wrong_source.app_data = Some(app_data(json!({"vendor_id": "device-1"})));
        assert!(build_request(request_with_event(wrong_source), NOW).is_err());

        let mut empty = website_event(user_with_email("a@b.com"));
        empty.action_source = CapiActionSource::App;
        empty.event_source_url = None;
        empty.app_data = Some(Map::new());
        assert!(build_request(request_with_event(empty), NOW).is_err());

        let rejected = [
            json!({"nested": {"access_token": "secret"}}),
            json!({"receipt_data": "Bearer abcdefghijklmnopqrstuvwxyz"}),
            json!({"email": "raw@example.com"}),
            json!({"messaging_channel": "whatsapp"}),
            json!({"ctwa_clid": "click-id"}),
            json!({"receipt_data": "x".repeat(crate::bounded_json::MAX_STRING_CHARS + 1)}),
        ];
        for app_data_value in rejected {
            let mut event = website_event(user_with_email("a@b.com"));
            event.action_source = CapiActionSource::App;
            event.event_source_url = None;
            event.app_data = Some(app_data(app_data_value));
            assert!(build_request(request_with_event(event), NOW).is_err());
        }
    }

    #[test]
    fn preserves_plain_identifiers_and_lowercases_prehashed_values() {
        let digest = sha256_hex(b"alice@example.com").to_ascii_uppercase();
        let mut user = user_with_email(&digest);
        user.subscription_id = Some("sub_CaseSensitive".to_owned());
        let request = build_request(request_with_event(website_event(user)), NOW).unwrap();
        let data: Value = serde_json::from_str(&request.form[0].1).unwrap();
        assert_eq!(data[0]["user_data"]["em"][0], digest.to_ascii_lowercase());
        assert_eq!(data[0]["user_data"]["subscription_id"], "sub_CaseSensitive");
    }

    #[test]
    fn applies_field_specific_normalization_before_hashing() {
        let mut user = user_with_email("a@b.com");
        user.genders = Some(vec![CapiGender::Female]);
        user.dates_of_birth = Some(vec!["1990-05-21".to_owned()]);
        user.cities = Some(vec!["San Francisco".to_owned()]);
        user.states = Some(vec!["New York!".to_owned()]);
        user.zip_codes = Some(vec![" 94105-1234 ".to_owned()]);
        user.country_codes = Some(vec!["US".to_owned()]);
        let request = build_request(request_with_event(website_event(user)), NOW).unwrap();
        let data: Value = serde_json::from_str(&request.form[0].1).unwrap();
        let user = &data[0]["user_data"];
        assert_eq!(user["ge"][0], sha256_hex(b"f"));
        assert_eq!(user["db"][0], sha256_hex(b"19900521"));
        assert_eq!(user["ct"][0], sha256_hex(b"sanfrancisco"));
        assert_eq!(user["st"][0], sha256_hex(b"newyork"));
        assert_eq!(user["zp"][0], sha256_hex(b"94105"));
        assert_eq!(user["country"][0], sha256_hex(b"us"));
    }

    #[test]
    fn rejects_invalid_or_ambiguous_event_inputs() {
        for url in [
            "https://user:synthetic-secret@example.test/",
            "https://example.test/?access_token=synthetic-secret",
            "https://example.test/?%61ccess_token=synthetic-secret",
        ] {
            let mut event = website_event(user_with_email("a@b.com"));
            event.event_source_url = Some(url.to_owned());
            assert!(build_request(request_with_event(event), NOW).is_err());
        }

        let mut missing_url = website_event(user_with_email("a@b.com"));
        missing_url.event_source_url = None;
        assert!(build_request(request_with_event(missing_url), NOW).is_err());

        let mut stale = website_event(user_with_email("a@b.com"));
        stale.event_time = NOW - 7 * 24 * 60 * 60 - 1;
        assert!(build_request(request_with_event(stale), NOW).is_err());

        let mut empty_user = user_with_email("a@b.com");
        empty_user.emails = None;
        assert!(build_request(request_with_event(website_event(empty_user)), NOW).is_err());

        let mut invalid_ip = user_with_email("a@b.com");
        invalid_ip.client_ip_address = Some("203.0.113.8:443".to_owned());
        assert!(build_request(request_with_event(website_event(invalid_ip)), NOW).is_err());

        let mut invalid_test_code = request_with_event(website_event(user_with_email("a@b.com")));
        invalid_test_code.test_event_code = Some("TEST code".to_owned());
        assert!(build_request(invalid_test_code, NOW).is_err());

        let mut oversized_partner = request_with_event(website_event(user_with_email("a@b.com")));
        oversized_partner.partner_agent = Some("p".repeat(257));
        assert!(build_request(oversized_partner, NOW).is_err());

        let incoherent_ldu = json!({
            "dataset_id": "123",
            "events": [{
                "event_name": "Purchase",
                "event_time": NOW - 10,
                "action_source": "app",
                "user_data": {"emails": ["a@b.com"]},
                "data_processing": {
                    "option": "ldu",
                    "location": "automatic",
                    "country": 1,
                    "state": 1000
                }
            }]
        });
        assert!(serde_json::from_value::<SendCapiEventsInput>(incoherent_ldu).is_err());

        let unknown = json!({
            "dataset_id": "123",
            "events": [],
            "access_token": "secret"
        });
        assert!(serde_json::from_value::<SendCapiEventsInput>(unknown).is_err());
    }

    #[test]
    fn enforces_event_and_encoded_body_limits() {
        let too_many = SendCapiEventsInput {
            dataset_id: "123".to_owned(),
            events: (0..51)
                .map(|index| {
                    let mut event = website_event(user_with_email("a@b.com"));
                    event.event_id = Some(format!("event-{index}"));
                    event
                })
                .collect(),
            test_event_code: None,
            partner_agent: None,
        };
        assert!(build_request(too_many, NOW).is_err());

        let oversized = SendCapiEventsInput {
            dataset_id: "123".to_owned(),
            events: (0..50)
                .map(|index| {
                    let mut user = user_with_email("a@b.com");
                    user.client_user_agent = Some("a".repeat(1_024));
                    let mut event = website_event(user);
                    event.event_id = Some(format!("event-{index}"));
                    event
                })
                .collect(),
            test_event_code: None,
            partner_agent: None,
        };
        assert!(build_request(oversized, NOW).is_err());
    }

    #[test]
    fn returns_only_aggregate_acknowledgement_data() {
        let ack = parse_ack(
            &json!({
                "events_received": 1,
                "messages": ["provider detail that must not be returned"],
                "fbtrace_id": "trace"
            }),
            "123".to_owned(),
            1,
            true,
        )
        .unwrap();
        let encoded = serde_json::to_string(&ack).unwrap();
        assert_eq!(ack.events_received, 1);
        assert_eq!(ack.provider_message_count, 1);
        assert!(!encoded.contains("provider detail"));
        assert!(!encoded.contains("trace"));
    }

    #[test]
    fn scrubs_customer_data_from_provider_errors() {
        let error = private_capi_graph_error(GraphError::Api {
            status: 400,
            code: Some(100),
            message: "alice@example.com was rejected".to_owned(),
            retryable: false,
        });
        assert!(!error.message.contains("alice"));
        assert!(!error.retryable);
        assert!(error.action.unwrap().contains("Events Manager"));
    }

    #[test]
    fn schema_is_closed_bounded_and_channel_neutral() {
        let schema = serde_json::to_value(schema_for!(SendCapiEventsInput)).unwrap();
        assert_eq!(schema["properties"]["partner_agent"]["maxLength"], 256);
        assert_eq!(
            schema["$defs"]["CapiActionSource"]["enum"],
            json!([
                "website",
                "app",
                "chat",
                "business_messaging",
                "email",
                "phone_call",
                "physical_store",
                "system_generated",
                "other"
            ])
        );
        assert_eq!(
            schema["$defs"]["CapiLduLocation"]["enum"],
            json!([
                "automatic",
                "california",
                "colorado",
                "connecticut",
                "florida",
                "oregon",
                "texas",
                "montana",
                "delaware",
                "nebraska",
                "new_hampshire",
                "new_jersey",
                "minnesota",
                "maryland",
                "rhode_island"
            ])
        );
        let schema = schema.to_string();
        assert!(
            schema.len() < 16_000,
            "input schema is {} bytes",
            schema.len()
        );
        assert!(!schema.to_ascii_lowercase().contains("whatsapp"));
        assert!(!schema.contains("messaging_channel"));
        assert!(!schema.contains("ctwa_clid"));
        assert!(schema.contains("app_data"));
        assert!(!schema.contains("meta_access_token"));
        assert!(schema.contains("partner_agent"));
        assert!(schema.contains("business_messaging"));
        assert!(schema.contains("new_hampshire"));
        assert!(schema.contains("rhode_island"));
        assert!(schema.contains("additionalProperties\":false"));
    }
}
