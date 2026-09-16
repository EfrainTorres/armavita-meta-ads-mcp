// Copyright (C) 2025 ArmaVita LLC
// SPDX-License-Identifier: AGPL-3.0-only

use std::fmt::Write as _;

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::{
    error::{GraphError, PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{
        ad_account as normalize_ad_account_id, numeric_owned as normalize_numeric_id,
        numeric_value as normalize_numeric_value,
    },
    mutation_result::{ambiguous_mutation_result, mutation_error_without_blind_retry},
    node_identity::{MetaNodeKind, verify_meta_node},
    safety::validate_removal_acknowledgement,
};

const MAX_NAME_CHARS: usize = 256;
const MAX_DESCRIPTION_CHARS: usize = 1_000;
const MAX_SCHEMA_COLUMNS: usize = 15;
const MAX_POST_ROWS: usize = 1_000;
const MAX_DELETE_ROWS: usize = 100;
const MAX_APP_IDS: usize = 50;
const MAX_CELL_CHARS: usize = 512;
const MAX_RAW_DATA_BYTES: usize = 128 * 1024;
// Form encoding can expand each JSON byte threefold. This keeps the encoded
// `payload` below GraphClient's 128 KiB mutation limit.
const MAX_PAYLOAD_JSON_BYTES: usize = 40 * 1024;
const MAX_UPSTREAM_COUNT: u64 = 1_000_000_000;

/// Create an empty customer-list audience. Non-destructive and non-idempotent:
/// repeating an ambiguous request can create a duplicate audience.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCustomAudienceInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    pub ad_account_id: String,
    /// Audience name, from 1 through 256 characters.
    pub name: String,
    /// How the customer information was collected.
    pub customer_file_source: CustomerFileSource,
    /// Optional audience description, up to 1,000 characters.
    pub description: Option<String>,
}

/// Update mutable customer-list metadata. Non-destructive and idempotent for an
/// unchanged input.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateCustomAudienceInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
    pub ad_account_id: String,
    /// Numeric Meta custom-audience ID.
    pub custom_audience_id: String,
    /// New audience name, from 1 through 256 characters.
    pub name: Option<String>,
    /// New description, up to 1,000 characters. Use an empty string to clear it.
    pub description: Option<String>,
}

/// Permanently delete an audience. Destructive but idempotent in final-state
/// semantics; Meta may report not-found on a repeat. Verify state after any
/// ambiguous transport failure.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteCustomAudienceInput {
    /// Numeric Meta ad-account ID that owns the audience, with or without `act_`.
    #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
    pub ad_account_id: String,
    /// Numeric Meta custom-audience ID.
    pub custom_audience_id: String,
    /// Exact destructive-action acknowledgement required after operator approval.
    #[schemars(
        length(min = 25, max = 25),
        regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
    )]
    pub removal_acknowledgement: String,
}

/// Add, remove, or replace customer-list members. This input intentionally does
/// not derive `Debug`: it can contain raw customer identifiers. This slice does
/// not accept LDU/data-processing-option columns; do not use it when those
/// compliance fields are required.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManageCustomAudienceUsersInput {
    /// Numeric Meta ad-account ID that owns the audience, with or without `act_`.
    #[schemars(length(min = 1, max = 68), regex(pattern = "^(act_)?[0-9]{1,64}$"))]
    pub ad_account_id: String,
    /// Numeric Meta custom-audience ID.
    pub custom_audience_id: String,
    /// Membership operation. Replace and remove are destructive; the combined
    /// tool must be annotated destructive and non-idempotent.
    pub operation: AudienceUserOperation,
    /// Required for REMOVE or REPLACE after operator approval; omit for ADD.
    #[schemars(
        length(min = 25, max = 25),
        regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
    )]
    pub removal_acknowledgement: Option<String>,
    /// Ordered identifier columns. Scalar EMAIL_SHA256, PHONE_SHA256,
    /// MOBILE_ADVERTISER_ID, and UID modes must be used alone; all other
    /// entries form a multi-key schema. Duplicate columns are rejected.
    #[schemars(length(min = 1, max = 15))]
    pub schema: Vec<UserIdentifierSchema>,
    /// Rows whose values exactly match the schema order. PII is normalized and
    /// SHA-256 hashed locally. Meta identifiers, EXTERN_ID, and LOOKALIKE_VALUE
    /// use the plaintext wire shown by Meta's tagged v26 examples; verify the
    /// documented SDK/example conflict on the target account.
    #[schemars(
        length(min = 1, max = 1_000),
        inner(length(min = 1, max = 15), inner(length(max = 512)))
    )]
    pub data: Vec<Vec<String>>,
    /// Optional numeric app IDs for app-scoped identifiers; required for UID.
    #[schemars(
        length(min = 1, max = 50),
        inner(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))
    )]
    pub app_ids: Option<Vec<String>>,
    /// Required only for REPLACE. Reuse one unique session ID for all sequential
    /// batches and set `last_batch` on the final batch.
    pub replace_session: Option<ReplaceAudienceSession>,
}

/// Create a country-based lookalike. Non-destructive and non-idempotent: a
/// repeated ambiguous request can create a duplicate audience.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateLookalikeAudienceInput {
    /// Numeric Meta ad-account ID, with or without the `act_` prefix.
    pub ad_account_id: String,
    /// Lookalike name, from 1 through 256 characters.
    pub name: String,
    /// Numeric seed custom-audience ID.
    pub origin_audience_id: String,
    /// Typed country-based lookalike definition.
    pub lookalike_spec: LookalikeSpecInput,
    /// Optional description, up to 1,000 characters.
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CustomerFileSource {
    UserProvidedOnly,
    PartnerProvidedOnly,
    BothUserAndPartnerProvided,
}

impl CustomerFileSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::UserProvidedOnly => "USER_PROVIDED_ONLY",
            Self::PartnerProvidedOnly => "PARTNER_PROVIDED_ONLY",
            Self::BothUserAndPartnerProvided => "BOTH_USER_AND_PARTNER_PROVIDED",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AudienceUserOperation {
    Add,
    Remove,
    Replace,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplaceAudienceSession {
    /// Advertiser-generated positive 64-bit identifier, unique within the ad account.
    pub session_id: u64,
    /// Positive batch sequence. A new session must start at 1.
    pub batch_sequence: u32,
    /// True only for the final batch in this replace session.
    pub last_batch: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UserIdentifierSchema {
    EmailSha256,
    PhoneSha256,
    MobileAdvertiserId,
    Uid,
    Email,
    Phone,
    Gen,
    Doby,
    Dobm,
    Dobd,
    LastName,
    FirstName,
    FirstInitial,
    City,
    State,
    Zip,
    Country,
    Madid,
    Appuid,
    ExternId,
    LookalikeValue,
}

impl UserIdentifierSchema {
    const fn as_str(self) -> &'static str {
        match self {
            Self::EmailSha256 => "EMAIL_SHA256",
            Self::PhoneSha256 => "PHONE_SHA256",
            Self::MobileAdvertiserId => "MOBILE_ADVERTISER_ID",
            Self::Uid => "UID",
            Self::Email => "EMAIL",
            Self::Phone => "PHONE",
            Self::Gen => "GEN",
            Self::Doby => "DOBY",
            Self::Dobm => "DOBM",
            Self::Dobd => "DOBD",
            Self::LastName => "LN",
            Self::FirstName => "FN",
            Self::FirstInitial => "FI",
            Self::City => "CT",
            Self::State => "ST",
            Self::Zip => "ZIP",
            Self::Country => "COUNTRY",
            Self::Madid => "MADID",
            Self::Appuid => "APPUID",
            Self::ExternId => "EXTERN_ID",
            Self::LookalikeValue => "LOOKALIKE_VALUE",
        }
    }

    const fn is_scalar(self) -> bool {
        matches!(
            self,
            Self::EmailSha256 | Self::PhoneSha256 | Self::MobileAdvertiserId | Self::Uid
        )
    }

    const fn requires_hash(self) -> bool {
        !matches!(
            self,
            Self::MobileAdvertiserId
                | Self::Uid
                | Self::Madid
                | Self::Appuid
                | Self::ExternId
                | Self::LookalikeValue
        )
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LookalikeAudienceType {
    Similarity,
    Reach,
}

impl LookalikeAudienceType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Similarity => "similarity",
            Self::Reach => "reach",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LookalikeSpecInput {
    /// Two-letter country code.
    pub country: String,
    /// Meta preset. Provide exactly one of `audience_type` or `ratio`.
    pub audience_type: Option<LookalikeAudienceType>,
    /// Custom top percentage as a fraction, 0.01 through 0.20 in 0.01 steps.
    pub ratio: Option<f64>,
    /// Optional range start in 0.01 steps; requires `ratio` and must be smaller.
    pub starting_ratio: Option<f64>,
    /// Permit Meta to find seed members outside the requested country.
    pub allow_international_seeds: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CreatedCustomAudience {
    pub custom_audience_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CustomAudienceMutationAck {
    pub custom_audience_id: String,
    pub accepted: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ManagedAudienceUsers {
    pub custom_audience_id: String,
    pub operation: AudienceUserOperation,
    pub submitted_rows: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub received_rows: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid_rows: Option<u64>,
    pub accepted: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MutationMethod {
    Post,
    Delete,
}

struct MutationRequest {
    method: MutationMethod,
    endpoint: String,
    form: Vec<(String, String)>,
}

struct UsersMutationRequest {
    request: MutationRequest,
    ad_account_id: String,
    custom_audience_id: String,
    operation: AudienceUserOperation,
    submitted_rows: u16,
}

pub(crate) async fn create_custom_audience(
    graph: &GraphClient,
    input: CreateCustomAudienceInput,
) -> ToolResponse<CreatedCustomAudience> {
    let request = match build_create_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(mutation_error_without_blind_retry(
                error,
                "Check Audiences in Meta Ads Manager before creating another audience",
            ));
        }
    };
    match created_id(&payload, "custom audience") {
        Ok(custom_audience_id) => {
            ToolResponse::success(CreatedCustomAudience { custom_audience_id })
        }
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn update_custom_audience(
    graph: &GraphClient,
    input: UpdateCustomAudienceInput,
) -> ToolResponse<CustomAudienceMutationAck> {
    let request = match build_update_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let custom_audience_id = request.endpoint.clone();
    if let Err(error) = verify_meta_node(
        graph,
        &custom_audience_id,
        MetaNodeKind::CustomAudience,
        Some(&input.ad_account_id),
    )
    .await
    {
        return ToolResponse::error(error);
    }
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    match confirmed_ack(&payload, &custom_audience_id, "update") {
        Ok(()) => ToolResponse::success(CustomAudienceMutationAck {
            custom_audience_id,
            accepted: true,
        }),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn delete_custom_audience(
    graph: &GraphClient,
    input: DeleteCustomAudienceInput,
) -> ToolResponse<CustomAudienceMutationAck> {
    let request = match build_delete_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let custom_audience_id = request.endpoint.clone();
    if let Err(error) = verify_meta_node(
        graph,
        &custom_audience_id,
        MetaNodeKind::CustomAudience,
        Some(&input.ad_account_id),
    )
    .await
    {
        return ToolResponse::error(error);
    }
    let payload = match graph.delete_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(mutation_error_without_blind_retry(
                error,
                "Verify whether the audience still exists before trying again",
            ));
        }
    };
    match confirmed_ack(&payload, &custom_audience_id, "deletion") {
        Ok(()) => ToolResponse::success(CustomAudienceMutationAck {
            custom_audience_id,
            accepted: true,
        }),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn manage_custom_audience_users(
    graph: &GraphClient,
    input: ManageCustomAudienceUsersInput,
) -> ToolResponse<ManagedAudienceUsers> {
    let request = match build_users_request(input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    if let Err(error) = verify_meta_node(
        graph,
        &request.custom_audience_id,
        MetaNodeKind::CustomAudience,
        Some(&request.ad_account_id),
    )
    .await
    {
        return ToolResponse::error(error);
    }
    let result = match request.request.method {
        MutationMethod::Post => {
            graph
                .post_form_json(&request.request.endpoint, &request.request.form)
                .await
        }
        MutationMethod::Delete => {
            graph
                .delete_json(&request.request.endpoint, &request.request.form)
                .await
        }
    };
    let payload = match result {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(private_data_graph_error(
                error,
                "Inspect the audience population status before submitting these rows again",
            ));
        }
    };
    match parse_users_ack(
        &payload,
        &request.custom_audience_id,
        request.operation,
        request.submitted_rows,
    ) {
        Ok(ack) => ToolResponse::success(ack),
        Err(error) => ToolResponse::error(error),
    }
}

pub(crate) async fn create_lookalike_audience(
    graph: &GraphClient,
    input: CreateLookalikeAudienceInput,
) -> ToolResponse<CreatedCustomAudience> {
    let request = match build_lookalike_request(&input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.post_form_json(&request.endpoint, &request.form).await {
        Ok(payload) => payload,
        Err(error) => {
            return ToolResponse::error(mutation_error_without_blind_retry(
                error,
                "Check Audiences in Meta Ads Manager before creating another lookalike",
            ));
        }
    };
    match created_id(&payload, "lookalike audience") {
        Ok(custom_audience_id) => {
            ToolResponse::success(CreatedCustomAudience { custom_audience_id })
        }
        Err(error) => ToolResponse::error(error),
    }
}

fn build_create_request(input: &CreateCustomAudienceInput) -> Result<MutationRequest, PublicError> {
    let account_id = normalize_ad_account_id(&input.ad_account_id).ok_or_else(|| {
        PublicError::invalid_input(
            "ad_account_id must be a numeric Meta account ID",
            "Use an ID such as `act_123456789` or `123456789`",
        )
    })?;
    let name = required_text(&input.name, MAX_NAME_CHARS, "name")?;
    let mut form = vec![
        ("name".to_owned(), name),
        ("subtype".to_owned(), "CUSTOM".to_owned()),
        (
            "customer_file_source".to_owned(),
            input.customer_file_source.as_str().to_owned(),
        ),
    ];
    if let Some(description) = optional_nonempty_text(
        input.description.as_deref(),
        MAX_DESCRIPTION_CHARS,
        "description",
    )? {
        form.push(("description".to_owned(), description));
    }
    Ok(MutationRequest {
        method: MutationMethod::Post,
        endpoint: format!("{account_id}/customaudiences"),
        form,
    })
}

fn build_update_request(input: &UpdateCustomAudienceInput) -> Result<MutationRequest, PublicError> {
    let custom_audience_id =
        normalize_numeric_id(&input.custom_audience_id).ok_or_else(invalid_audience_id)?;
    let mut form = Vec::with_capacity(2);
    if let Some(name) = &input.name {
        form.push((
            "name".to_owned(),
            required_text(name, MAX_NAME_CHARS, "name")?,
        ));
    }
    if let Some(description) = &input.description {
        form.push((
            "description".to_owned(),
            bounded_text_allow_empty(description, MAX_DESCRIPTION_CHARS, "description")?,
        ));
    }
    if form.is_empty() {
        return Err(PublicError::invalid_input(
            "at least one audience update is required",
            "Provide name and/or description",
        ));
    }
    Ok(MutationRequest {
        method: MutationMethod::Post,
        endpoint: custom_audience_id,
        form,
    })
}

fn build_delete_request(input: &DeleteCustomAudienceInput) -> Result<MutationRequest, PublicError> {
    let custom_audience_id =
        normalize_numeric_id(&input.custom_audience_id).ok_or_else(invalid_audience_id)?;
    validate_removal_acknowledgement(true, Some(&input.removal_acknowledgement))?;
    Ok(MutationRequest {
        method: MutationMethod::Delete,
        endpoint: custom_audience_id,
        form: Vec::new(),
    })
}

fn build_users_request(
    input: ManageCustomAudienceUsersInput,
) -> Result<UsersMutationRequest, PublicError> {
    let custom_audience_id =
        normalize_numeric_id(&input.custom_audience_id).ok_or_else(invalid_audience_id)?;
    validate_removal_acknowledgement(
        matches!(
            input.operation,
            AudienceUserOperation::Remove | AudienceUserOperation::Replace
        ),
        input.removal_acknowledgement.as_deref(),
    )?;
    let ad_account_id = normalize_ad_account_id(&input.ad_account_id).ok_or_else(|| {
        PublicError::invalid_input(
            "ad_account_id must be a numeric Meta account ID",
            "Use an ID such as `act_123456789` or `123456789`",
        )
    })?;
    validate_schema(&input.schema)?;
    let scalar_schema = input
        .schema
        .first()
        .copied()
        .filter(|field| field.is_scalar());
    let max_rows = if input.operation == AudienceUserOperation::Remove {
        MAX_DELETE_ROWS
    } else {
        MAX_POST_ROWS
    };
    let normalized_data = normalize_user_rows(&input.schema, input.data, max_rows)?;
    let submitted_rows = normalized_data.len();
    let app_ids = normalize_app_ids(input.app_ids)?;
    if scalar_schema == Some(UserIdentifierSchema::Uid) && app_ids.is_none() {
        return Err(PublicError::invalid_input(
            "app_ids is required when schema is UID",
            "Provide at least one numeric Meta app ID",
        ));
    }
    let mut payload = Map::new();
    if let Some(field) = scalar_schema {
        payload.insert("schema".to_owned(), json!(field.as_str()));
        payload.insert("is_raw".to_owned(), Value::Bool(false));
        payload.insert(
            "data".to_owned(),
            Value::Array(
                normalized_data
                    .into_iter()
                    .map(|row| {
                        Value::String(
                            row.into_iter()
                                .next()
                                .expect("validated scalar row has one value"),
                        )
                    })
                    .collect(),
            ),
        );
    } else {
        payload.insert(
            "schema".to_owned(),
            json!(
                input
                    .schema
                    .iter()
                    .map(|field| field.as_str())
                    .collect::<Vec<_>>()
            ),
        );
        payload.insert("is_raw".to_owned(), Value::Bool(true));
        payload.insert(
            "data".to_owned(),
            Value::Array(normalized_data.into_iter().map(|row| json!(row)).collect()),
        );
    }
    if let Some(app_ids) = app_ids {
        payload.insert("app_ids".to_owned(), json!(app_ids));
    }
    let payload = encode_bounded_json(&Value::Object(payload), "customer-list payload")?;
    let mut form = vec![("payload".to_owned(), payload)];

    let (method, endpoint) = match input.operation {
        AudienceUserOperation::Add => {
            reject_replace_session(input.replace_session)?;
            (MutationMethod::Post, format!("{custom_audience_id}/users"))
        }
        AudienceUserOperation::Remove => {
            reject_replace_session(input.replace_session)?;
            (
                MutationMethod::Delete,
                format!("{custom_audience_id}/users"),
            )
        }
        AudienceUserOperation::Replace => {
            let session = input.replace_session.ok_or_else(|| {
                PublicError::invalid_input(
                    "replace_session is required for REPLACE",
                    "Provide a unique session_id, sequential batch_sequence, and last_batch",
                )
            })?;
            validate_replace_session(session)?;
            form.push((
                "session".to_owned(),
                encode_bounded_json(
                    &json!({
                        "session_id": session.session_id,
                        "batch_seq": session.batch_sequence,
                        "last_batch_flag": session.last_batch,
                    }),
                    "replace session",
                )?,
            ));
            (
                MutationMethod::Post,
                format!("{custom_audience_id}/usersreplace"),
            )
        }
    };
    Ok(UsersMutationRequest {
        request: MutationRequest {
            method,
            endpoint,
            form,
        },
        ad_account_id,
        custom_audience_id,
        operation: input.operation,
        submitted_rows: u16::try_from(submitted_rows).expect("customer-list row limit fits in u16"),
    })
}

fn build_lookalike_request(
    input: &CreateLookalikeAudienceInput,
) -> Result<MutationRequest, PublicError> {
    let account_id = normalize_ad_account_id(&input.ad_account_id).ok_or_else(|| {
        PublicError::invalid_input(
            "ad_account_id must be a numeric Meta account ID",
            "Use an ID such as `act_123456789` or `123456789`",
        )
    })?;
    let origin_audience_id = normalize_numeric_id(&input.origin_audience_id).ok_or_else(|| {
        PublicError::invalid_input(
            "origin_audience_id must be a numeric Meta audience ID",
            "Use an ID returned by list_custom_audiences",
        )
    })?;
    let name = required_text(&input.name, MAX_NAME_CHARS, "name")?;
    let lookalike_spec = encode_lookalike_spec(&input.lookalike_spec)?;
    let mut form = vec![
        ("name".to_owned(), name),
        ("subtype".to_owned(), "LOOKALIKE".to_owned()),
        ("origin_audience_id".to_owned(), origin_audience_id),
        ("lookalike_spec".to_owned(), lookalike_spec),
    ];
    if let Some(description) = optional_nonempty_text(
        input.description.as_deref(),
        MAX_DESCRIPTION_CHARS,
        "description",
    )? {
        form.push(("description".to_owned(), description));
    }
    Ok(MutationRequest {
        method: MutationMethod::Post,
        endpoint: format!("{account_id}/customaudiences"),
        form,
    })
}

fn validate_schema(schema: &[UserIdentifierSchema]) -> Result<(), PublicError> {
    if schema.is_empty() || schema.len() > MAX_SCHEMA_COLUMNS {
        return Err(PublicError::invalid_input(
            "schema must contain 1 through 15 identifier columns",
            "Use only the identifiers needed for matching",
        ));
    }
    for (index, field) in schema.iter().enumerate() {
        if schema[..index].contains(field) {
            return Err(PublicError::invalid_input(
                "schema contains a duplicate identifier column",
                "Include each identifier type at most once",
            ));
        }
    }
    if schema.iter().any(|field| field.is_scalar()) && schema.len() != 1 {
        return Err(PublicError::invalid_input(
            "scalar identifier schemas must be used alone",
            "Use exactly one of EMAIL_SHA256, PHONE_SHA256, MOBILE_ADVERTISER_ID, or UID",
        ));
    }
    if schema == [UserIdentifierSchema::LookalikeValue] {
        return Err(PublicError::invalid_input(
            "LOOKALIKE_VALUE requires at least one matching identifier",
            "Pair LOOKALIKE_VALUE with EMAIL, PHONE, MADID, APPUID, or another multi-key identifier",
        ));
    }
    Ok(())
}

fn normalize_user_rows(
    schema: &[UserIdentifierSchema],
    data: Vec<Vec<String>>,
    max_rows: usize,
) -> Result<Vec<Vec<String>>, PublicError> {
    if data.is_empty() || data.len() > max_rows {
        return Err(PublicError::invalid_input(
            format!("data must contain 1 through {max_rows} rows for this operation"),
            "Split large uploads into bounded requests",
        ));
    }
    let mut raw_bytes = 0_usize;
    let mut normalized = Vec::with_capacity(data.len());
    for row in data {
        if row.len() != schema.len() {
            return Err(PublicError::invalid_input(
                "every data row must have exactly one value per schema column",
                "Align each row with the ordered schema",
            ));
        }
        let mut normalized_row = Vec::with_capacity(row.len());
        let mut populated = false;
        for (field, value) in schema.iter().copied().zip(row) {
            if value.chars().count() > MAX_CELL_CHARS {
                return Err(invalid_user_data());
            }
            raw_bytes = raw_bytes
                .checked_add(value.len())
                .ok_or_else(invalid_user_data)?;
            if raw_bytes > MAX_RAW_DATA_BYTES {
                return Err(PublicError::invalid_input(
                    "raw customer data exceeds the 128 KiB safety limit",
                    "Send fewer rows per request",
                ));
            }
            let value = normalize_identifier(field, &value)?;
            populated |= field != UserIdentifierSchema::LookalikeValue && !value.is_empty();
            normalized_row.push(value);
        }
        if !populated {
            return Err(PublicError::invalid_input(
                "each customer row must contain at least one identifier",
                "Remove empty rows or provide a matching identifier",
            ));
        }
        normalized.push(normalized_row);
    }
    Ok(normalized)
}

fn normalize_identifier(field: UserIdentifierSchema, raw: &str) -> Result<String, PublicError> {
    let trimmed =
        raw.trim_matches(|character: char| character.is_whitespace() || character == '\0');
    if trimmed.is_empty() {
        if field == UserIdentifierSchema::LookalikeValue {
            return Err(invalid_user_data());
        }
        return Ok(String::new());
    }
    if field.requires_hash() && is_sha256_hex(trimmed) {
        return Ok(trimmed.to_ascii_lowercase());
    }

    let normalized = match field {
        UserIdentifierSchema::EmailSha256 | UserIdentifierSchema::Email => trimmed.to_lowercase(),
        UserIdentifierSchema::PhoneSha256 | UserIdentifierSchema::Phone => {
            let digits = trimmed
                .bytes()
                .filter(u8::is_ascii_digit)
                .map(char::from)
                .collect::<String>();
            digits.trim_start_matches('0').to_owned()
        }
        UserIdentifierSchema::Gen => {
            let value = trimmed.to_ascii_lowercase();
            if !matches!(value.as_str(), "m" | "f") {
                return Err(invalid_user_data());
            }
            value
        }
        UserIdentifierSchema::Doby => normalize_fixed_digits(trimmed, 4, 1900, 9999)?,
        UserIdentifierSchema::Dobm => normalize_fixed_digits(trimmed, 2, 1, 12)?,
        UserIdentifierSchema::Dobd => normalize_fixed_digits(trimmed, 2, 1, 31)?,
        UserIdentifierSchema::LastName
        | UserIdentifierSchema::FirstName
        | UserIdentifierSchema::City
        | UserIdentifierSchema::State => trimmed
            .chars()
            .flat_map(char::to_lowercase)
            .filter(|character| character.is_alphabetic())
            .collect(),
        UserIdentifierSchema::FirstInitial => trimmed
            .chars()
            .flat_map(char::to_lowercase)
            .find(|character| character.is_alphabetic())
            .map_or_else(String::new, |character| character.to_string()),
        UserIdentifierSchema::Zip => trimmed
            .chars()
            .filter(|character| !character.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect(),
        UserIdentifierSchema::Country => {
            let value = trimmed.to_ascii_lowercase();
            if value.len() != 2 || !value.bytes().all(|byte| byte.is_ascii_alphabetic()) {
                return Err(invalid_user_data());
            }
            value
        }
        UserIdentifierSchema::MobileAdvertiserId
        | UserIdentifierSchema::Uid
        | UserIdentifierSchema::Appuid
        | UserIdentifierSchema::ExternId => trimmed.to_owned(),
        UserIdentifierSchema::Madid => {
            let value = trimmed.to_ascii_lowercase();
            if value.len() > 64
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
            {
                return Err(invalid_user_data());
            }
            value
        }
        UserIdentifierSchema::LookalikeValue => {
            if !trimmed
                .parse::<f64>()
                .is_ok_and(|value| value.is_finite() && value >= 0.0)
            {
                return Err(invalid_user_data());
            }
            trimmed.to_owned()
        }
    };
    if normalized.is_empty() {
        return Err(invalid_user_data());
    }
    if !field.requires_hash() {
        return Ok(normalized);
    }
    let digest = Sha256::digest(normalized.as_bytes());
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(encoded)
}

fn normalize_app_ids(raw: Option<Vec<String>>) -> Result<Option<Vec<String>>, PublicError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.is_empty() || raw.len() > MAX_APP_IDS {
        return Err(PublicError::invalid_input(
            "app_ids must contain 1 through 50 numeric app IDs",
            "Provide only the app IDs used by this customer-list payload",
        ));
    }
    let mut normalized = Vec::with_capacity(raw.len());
    for value in raw {
        let id = normalize_numeric_id(&value).ok_or_else(|| {
            PublicError::invalid_input(
                "app_ids must contain numeric Meta app IDs",
                "Use app IDs returned by Meta",
            )
        })?;
        if normalized.contains(&id) {
            return Err(PublicError::invalid_input(
                "app_ids contains a duplicate ID",
                "Include each app ID once",
            ));
        }
        normalized.push(id);
    }
    Ok(Some(normalized))
}

fn normalize_fixed_digits(
    raw: &str,
    width: usize,
    minimum: u16,
    maximum: u16,
) -> Result<String, PublicError> {
    let digits = raw
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(char::from)
        .collect::<String>();
    let value = digits
        .parse::<u16>()
        .ok()
        .filter(|value| (minimum..=maximum).contains(value))
        .ok_or_else(invalid_user_data)?;
    Ok(format!("{value:0width$}"))
}

fn encode_lookalike_spec(input: &LookalikeSpecInput) -> Result<String, PublicError> {
    let country = normalize_country(&input.country)?;
    let mut spec = Map::new();
    spec.insert("country".to_owned(), Value::String(country));
    match (input.audience_type, input.ratio) {
        (Some(audience_type), None) => {
            if input.starting_ratio.is_some() {
                return Err(PublicError::invalid_input(
                    "starting_ratio requires a custom ratio",
                    "Omit starting_ratio or replace audience_type with ratio",
                ));
            }
            spec.insert(
                "type".to_owned(),
                Value::String(audience_type.as_str().to_owned()),
            );
        }
        (None, Some(ratio)) if valid_ratio(ratio) => {
            if let Some(starting_ratio) = input.starting_ratio {
                if !valid_starting_ratio(starting_ratio) || starting_ratio >= ratio {
                    return Err(PublicError::invalid_input(
                        "starting_ratio must be a 0.01 step smaller than ratio",
                        "Use a range such as starting_ratio=0.01 and ratio=0.02",
                    ));
                }
                spec.insert("starting_ratio".to_owned(), json!(starting_ratio));
            }
            spec.insert("ratio".to_owned(), json!(ratio));
        }
        _ => {
            return Err(PublicError::invalid_input(
                "provide exactly one valid audience_type or ratio",
                "Use similarity/reach or a ratio from 0.01 through 0.20",
            ));
        }
    }
    if let Some(allow) = input.allow_international_seeds {
        spec.insert("allow_international_seeds".to_owned(), Value::Bool(allow));
    }
    encode_bounded_json(&Value::Object(spec), "lookalike_spec")
}

fn valid_ratio(value: f64) -> bool {
    value.is_finite()
        && (0.01..=0.20).contains(&value)
        && ((value * 100.0).round() - value * 100.0).abs() < 1e-9
}

fn valid_starting_ratio(value: f64) -> bool {
    value.is_finite()
        && (0.0..=0.19).contains(&value)
        && ((value * 100.0).round() - value * 100.0).abs() < 1e-9
}

fn validate_replace_session(session: ReplaceAudienceSession) -> Result<(), PublicError> {
    if session.session_id == 0
        || session.session_id > i64::MAX as u64
        || session.batch_sequence == 0
    {
        return Err(PublicError::invalid_input(
            "replace_session requires positive bounded IDs and sequence numbers",
            "Start a unique session with batch_sequence=1",
        ));
    }
    Ok(())
}

fn reject_replace_session(session: Option<ReplaceAudienceSession>) -> Result<(), PublicError> {
    if session.is_some() {
        return Err(PublicError::invalid_input(
            "replace_session is valid only for REPLACE",
            "Omit replace_session for ADD and REMOVE",
        ));
    }
    Ok(())
}

fn parse_users_ack(
    payload: &Value,
    custom_audience_id: &str,
    operation: AudienceUserOperation,
    submitted_rows: u16,
) -> Result<ManagedAudienceUsers, PublicError> {
    let object = payload.as_object().ok_or_else(|| {
        ambiguous_result("Meta did not return a customer-list upload acknowledgement")
    })?;
    if object.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(ambiguous_result(
            "Meta did not acknowledge the customer-list mutation",
        ));
    }
    if let Some(value) = object.get("audience_id")
        && normalize_numeric_value(value).as_deref() != Some(custom_audience_id)
    {
        return Err(ambiguous_result(
            "Meta acknowledged a different custom audience",
        ));
    }
    let received_rows = bounded_count(object.get("num_received"))?;
    let invalid_rows = bounded_count(object.get("num_invalid_entries"))?;
    if invalid_rows
        .zip(received_rows)
        .is_some_and(|(invalid, received)| invalid > received)
    {
        return Err(ambiguous_result(
            "Meta returned inconsistent customer-list counts",
        ));
    }
    let acknowledged = object.get("success").and_then(Value::as_bool) == Some(true)
        || object.get("audience_id").is_some()
        || received_rows.is_some();
    if !acknowledged {
        return Err(ambiguous_result(
            "Meta did not confirm the customer-list mutation",
        ));
    }
    Ok(ManagedAudienceUsers {
        custom_audience_id: custom_audience_id.to_owned(),
        operation,
        submitted_rows,
        received_rows,
        invalid_rows,
        accepted: true,
    })
}

fn bounded_count(value: Option<&Value>) -> Result<Option<u64>, PublicError> {
    let Some(value) = value else {
        return Ok(None);
    };
    value
        .as_u64()
        .filter(|count| *count <= MAX_UPSTREAM_COUNT)
        .map(Some)
        .ok_or_else(|| ambiguous_result("Meta returned an invalid customer-list count"))
}

fn created_id(payload: &Value, resource: &str) -> Result<String, PublicError> {
    if payload.get("success").and_then(Value::as_bool) == Some(false)
        || payload.get("error").is_some()
    {
        return Err(ambiguous_result(format!(
            "Meta did not confirm the new {resource}"
        )));
    }
    payload
        .get("id")
        .and_then(normalize_numeric_value)
        .ok_or_else(|| ambiguous_result(format!("Meta did not confirm the new {resource} ID")))
}

fn confirmed_ack(payload: &Value, expected_id: &str, operation: &str) -> Result<(), PublicError> {
    if payload.get("success").and_then(Value::as_bool) == Some(false)
        || payload.get("error").is_some()
    {
        return Err(ambiguous_result(format!(
            "Meta did not confirm the custom-audience {operation}"
        )));
    }
    if payload.as_bool() == Some(true)
        || payload.get("success").and_then(Value::as_bool) == Some(true)
        || payload
            .get("id")
            .and_then(normalize_numeric_value)
            .is_some_and(|id| id == expected_id)
    {
        return Ok(());
    }
    Err(ambiguous_result(format!(
        "Meta did not confirm the custom-audience {operation}"
    )))
}

fn encode_bounded_json(value: &Value, field: &str) -> Result<String, PublicError> {
    let encoded = serde_json::to_string(value).map_err(|_| {
        PublicError::invalid_input(
            format!("{field} could not be encoded"),
            "Use only supported typed values",
        )
    })?;
    if encoded.len() > MAX_PAYLOAD_JSON_BYTES {
        return Err(PublicError::invalid_input(
            format!("{field} exceeds the 40 KiB safety limit"),
            "Send fewer customer rows per request",
        ));
    }
    Ok(encoded)
}

fn required_text(raw: &str, max_chars: usize, field: &str) -> Result<String, PublicError> {
    let value = raw.trim();
    if value.is_empty() || value.chars().count() > max_chars || value.chars().any(char::is_control)
    {
        return Err(PublicError::invalid_input(
            format!("{field} must contain 1 through {max_chars} characters without controls"),
            format!("Provide bounded plain-text {field}"),
        ));
    }
    Ok(value.to_owned())
}

fn optional_nonempty_text(
    raw: Option<&str>,
    max_chars: usize,
    field: &str,
) -> Result<Option<String>, PublicError> {
    raw.map(|value| required_text(value, max_chars, field))
        .transpose()
}

fn bounded_text_allow_empty(
    raw: &str,
    max_chars: usize,
    field: &str,
) -> Result<String, PublicError> {
    let value = raw.trim();
    if value.chars().count() > max_chars || value.chars().any(char::is_control) {
        return Err(PublicError::invalid_input(
            format!("{field} exceeds {max_chars} characters or contains controls"),
            format!("Use bounded plain-text {field}"),
        ));
    }
    Ok(value.to_owned())
}

fn normalize_country(raw: &str) -> Result<String, PublicError> {
    let country = raw.trim().to_ascii_uppercase();
    if country.len() != 2 || !country.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return Err(PublicError::invalid_input(
            "country must be a two-letter country code",
            "Use an ISO 3166-1 alpha-2 code such as US",
        ));
    }
    Ok(country)
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn invalid_audience_id() -> PublicError {
    PublicError::invalid_input(
        "custom_audience_id must be a numeric Meta audience ID",
        "Use an ID returned by list_custom_audiences",
    )
}

fn invalid_user_data() -> PublicError {
    PublicError::invalid_input(
        "customer data contains an invalid or oversized identifier",
        "Check the schema-specific Meta normalization requirements",
    )
}

fn private_data_graph_error(error: GraphError, action: &str) -> PublicError {
    let mut error = mutation_error_without_blind_retry(error, action);
    error.message = match error.code.as_str() {
        "AUTH_REQUIRED" => "Meta authentication is not configured".to_owned(),
        "AUTH_EXPIRED" => "Meta authentication expired during the customer-list request".to_owned(),
        "INVALID_INPUT" => "The customer-list request failed local safety validation".to_owned(),
        "RESPONSE_TOO_LARGE" => "Meta returned an oversized customer-list response".to_owned(),
        "META_UNAVAILABLE" => "The customer-list request did not complete".to_owned(),
        _ => "Meta rejected the customer-list request".to_owned(),
    };
    error
}

fn ambiguous_result(message: impl Into<String>) -> PublicError {
    ambiguous_mutation_result(
        message,
        "Verify the audience state in Meta Ads Manager before retrying",
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rmcp::schemars::schema_for;
    use serde_json::{Value, json};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    use crate::{
        config::MetaConfig,
        error::{GraphError, ToolResponse},
        graph::GraphClient,
        safety::REMOVAL_ACKNOWLEDGEMENT,
    };

    use super::{
        AudienceUserOperation, CreateCustomAudienceInput, CreateLookalikeAudienceInput,
        CustomerFileSource, DeleteCustomAudienceInput, LookalikeSpecInput,
        ManageCustomAudienceUsersInput, MutationMethod, ReplaceAudienceSession,
        UpdateCustomAudienceInput, UserIdentifierSchema, build_create_request,
        build_delete_request, build_lookalike_request, build_update_request, build_users_request,
        delete_custom_audience, manage_custom_audience_users, normalize_identifier,
        parse_users_ack, private_data_graph_error, update_custom_audience,
    };

    async fn scripted_graph(
        responses: Vec<&'static str>,
    ) -> (GraphClient, tokio::task::JoinHandle<Vec<Vec<u8>>>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut requests = Vec::with_capacity(responses.len());
            for body in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                requests.push(read_http_request(&mut socket).await);
                write_json(&mut socket, body).await;
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(200), listener.accept())
                    .await
                    .is_err(),
                "audience operation emitted an unexpected request"
            );
            requests
        });
        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        ))
        .unwrap();
        (graph, server)
    }

    async fn read_http_request(socket: &mut TcpStream) -> Vec<u8> {
        let mut request = Vec::with_capacity(2 * 1024);
        let mut buffer = [0_u8; 1024];
        loop {
            let bytes_read = socket.read(&mut buffer).await.unwrap();
            if bytes_read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..bytes_read]);
            let Some(headers_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let body_start = headers_end + 4;
            let headers = String::from_utf8_lossy(&request[..headers_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or_default();
            if request.len() >= body_start + content_length {
                break;
            }
        }
        request
    }

    async fn write_json(socket: &mut TcpStream, body: &str) {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    }

    #[test]
    fn direct_audience_schemas_require_a_bounded_account_id() {
        for schema in [
            serde_json::to_value(schema_for!(UpdateCustomAudienceInput)).unwrap(),
            serde_json::to_value(schema_for!(DeleteCustomAudienceInput)).unwrap(),
            serde_json::to_value(schema_for!(ManageCustomAudienceUsersInput)).unwrap(),
        ] {
            assert!(
                schema["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("ad_account_id"))
            );
            let account = &schema["properties"]["ad_account_id"];
            assert_eq!(account["minLength"], 1);
            assert_eq!(account["maxLength"], 68);
            assert_eq!(account["pattern"], "^(act_)?[0-9]{1,64}$");
        }
    }

    #[tokio::test]
    async fn wrong_node_type_blocks_every_direct_audience_write() {
        for operation in ["update", "delete", "users_remove"] {
            let (graph, server) = scripted_graph(vec![
                r#"{"id":"42","account_id":"123","objective":"OUTCOME_SALES"}"#,
            ])
            .await;
            let failed = match operation {
                "update" => matches!(
                    update_custom_audience(
                        &graph,
                        UpdateCustomAudienceInput {
                            ad_account_id: "123".to_owned(),
                            custom_audience_id: "42".to_owned(),
                            name: Some("Renamed".to_owned()),
                            description: None,
                        },
                    )
                    .await,
                    ToolResponse::Error { .. }
                ),
                "delete" => matches!(
                    delete_custom_audience(
                        &graph,
                        DeleteCustomAudienceInput {
                            ad_account_id: "123".to_owned(),
                            custom_audience_id: "42".to_owned(),
                            removal_acknowledgement: REMOVAL_ACKNOWLEDGEMENT.to_owned(),
                        },
                    )
                    .await,
                    ToolResponse::Error { .. }
                ),
                "users_remove" => matches!(
                    manage_custom_audience_users(
                        &graph,
                        ManageCustomAudienceUsersInput {
                            ad_account_id: "123".to_owned(),
                            custom_audience_id: "42".to_owned(),
                            operation: AudienceUserOperation::Remove,
                            removal_acknowledgement: Some(REMOVAL_ACKNOWLEDGEMENT.to_owned()),
                            schema: vec![UserIdentifierSchema::ExternId],
                            data: vec![vec!["synthetic-id".to_owned()]],
                            app_ids: None,
                            replace_session: None,
                        },
                    )
                    .await,
                    ToolResponse::Error { .. }
                ),
                _ => unreachable!("fixed operation table"),
            };
            assert!(failed, "{operation} accepted a wrong node type");

            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 1, "{operation} reached a write endpoint");
            let preflight = String::from_utf8(requests.into_iter().next().unwrap()).unwrap();
            assert!(
                preflight
                    .starts_with("GET /42?fields=id%2Cname%2Caccount_id%2Csubtype HTTP/1.1\r\n")
            );
        }
    }

    #[tokio::test]
    async fn account_mismatch_blocks_direct_audience_delete() {
        let (graph, server) = scripted_graph(vec![
            r#"{"id":"42","name":"Audience","account_id":"999","subtype":"CUSTOM"}"#,
        ])
        .await;
        assert!(matches!(
            delete_custom_audience(
                &graph,
                DeleteCustomAudienceInput {
                    ad_account_id: "123".to_owned(),
                    custom_audience_id: "42".to_owned(),
                    removal_acknowledgement: REMOVAL_ACKNOWLEDGEMENT.to_owned(),
                },
            )
            .await,
            ToolResponse::Error { .. }
        ));

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 1, "account mismatch reached DELETE");
        assert!(
            String::from_utf8(requests[0].clone())
                .unwrap()
                .starts_with("GET /42?fields=id%2Cname%2Caccount_id%2Csubtype HTTP/1.1\r\n")
        );
    }

    #[tokio::test]
    async fn valid_node_preflight_precedes_every_exact_direct_audience_write() {
        for (operation, mutation_response) in [
            ("update", r#"{"success":true}"#),
            ("delete", r#"{"success":true}"#),
            ("users_remove", r#"{"audience_id":"42","num_received":1}"#),
        ] {
            let (graph, server) = scripted_graph(vec![
                r#"{"id":"42","name":"Audience","account_id":"123","subtype":"CUSTOM"}"#,
                mutation_response,
            ])
            .await;
            let succeeded = match operation {
                "update" => matches!(
                    update_custom_audience(
                        &graph,
                        UpdateCustomAudienceInput {
                            ad_account_id: "123".to_owned(),
                            custom_audience_id: "42".to_owned(),
                            name: Some("Renamed".to_owned()),
                            description: None,
                        },
                    )
                    .await,
                    ToolResponse::Success { .. }
                ),
                "delete" => matches!(
                    delete_custom_audience(
                        &graph,
                        DeleteCustomAudienceInput {
                            ad_account_id: "123".to_owned(),
                            custom_audience_id: "42".to_owned(),
                            removal_acknowledgement: REMOVAL_ACKNOWLEDGEMENT.to_owned(),
                        },
                    )
                    .await,
                    ToolResponse::Success { .. }
                ),
                "users_remove" => matches!(
                    manage_custom_audience_users(
                        &graph,
                        ManageCustomAudienceUsersInput {
                            ad_account_id: "123".to_owned(),
                            custom_audience_id: "42".to_owned(),
                            operation: AudienceUserOperation::Remove,
                            removal_acknowledgement: Some(REMOVAL_ACKNOWLEDGEMENT.to_owned()),
                            schema: vec![UserIdentifierSchema::ExternId],
                            data: vec![vec!["synthetic-id".to_owned()]],
                            app_ids: None,
                            replace_session: None,
                        },
                    )
                    .await,
                    ToolResponse::Success { .. }
                ),
                _ => unreachable!("fixed operation table"),
            };
            assert!(succeeded, "{operation} rejected a valid custom audience");

            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 2);
            let preflight = String::from_utf8(requests[0].clone()).unwrap();
            let mutation = String::from_utf8(requests[1].clone()).unwrap();
            assert!(
                preflight
                    .starts_with("GET /42?fields=id%2Cname%2Caccount_id%2Csubtype HTTP/1.1\r\n")
            );
            let request_line = mutation.lines().next().unwrap();
            match operation {
                "update" => {
                    assert_eq!(request_line, "POST /42 HTTP/1.1");
                    assert!(mutation.ends_with("name=Renamed"));
                }
                "delete" => assert_eq!(request_line, "DELETE /42 HTTP/1.1"),
                "users_remove" => assert_eq!(
                    request_line,
                    "DELETE /42/users?payload=%7B%22data%22%3A%5B%5B%22synthetic-id%22%5D%5D%2C%22is_raw%22%3Atrue%2C%22schema%22%3A%5B%22EXTERN_ID%22%5D%7D HTTP/1.1"
                ),
                _ => unreachable!("fixed operation table"),
            }
        }
    }

    #[test]
    fn builds_exact_customer_list_create_and_update_requests() {
        let create = build_create_request(&CreateCustomAudienceInput {
            ad_account_id: " act_123 ".to_owned(),
            name: " Seed ".to_owned(),
            customer_file_source: CustomerFileSource::UserProvidedOnly,
            description: Some(" Current customers ".to_owned()),
        })
        .unwrap();
        assert!(create.method == MutationMethod::Post);
        assert_eq!(create.endpoint, "act_123/customaudiences");
        assert_eq!(
            create.form,
            [
                ("name".to_owned(), "Seed".to_owned()),
                ("subtype".to_owned(), "CUSTOM".to_owned()),
                (
                    "customer_file_source".to_owned(),
                    "USER_PROVIDED_ONLY".to_owned()
                ),
                ("description".to_owned(), "Current customers".to_owned()),
            ]
        );

        let update = build_update_request(&UpdateCustomAudienceInput {
            ad_account_id: "123".to_owned(),
            custom_audience_id: " 456 ".to_owned(),
            name: Some(" Renamed ".to_owned()),
            description: Some(" ".to_owned()),
        })
        .unwrap();
        assert_eq!(update.endpoint, "456");
        assert_eq!(
            update.form,
            [
                ("name".to_owned(), "Renamed".to_owned()),
                ("description".to_owned(), String::new()),
            ]
        );

        let delete = build_delete_request(&DeleteCustomAudienceInput {
            ad_account_id: "123".to_owned(),
            custom_audience_id: " 789 ".to_owned(),
            removal_acknowledgement: REMOVAL_ACKNOWLEDGEMENT.to_owned(),
        })
        .unwrap();
        assert!(delete.method == MutationMethod::Delete);
        assert_eq!(delete.endpoint, "789");
        assert!(delete.form.is_empty());
        assert!(
            build_delete_request(&DeleteCustomAudienceInput {
                ad_account_id: "123".to_owned(),
                custom_audience_id: "789".to_owned(),
                removal_acknowledgement: "yes".to_owned(),
            })
            .is_err()
        );
    }

    #[test]
    fn encodes_exact_multikey_wire_and_preserves_meta_identifiers() {
        let request = build_users_request(ManageCustomAudienceUsersInput {
            ad_account_id: "123".to_owned(),
            custom_audience_id: "42".to_owned(),
            operation: AudienceUserOperation::Add,
            removal_acknowledgement: None,
            schema: vec![
                UserIdentifierSchema::Email,
                UserIdentifierSchema::Madid,
                UserIdentifierSchema::Appuid,
                UserIdentifierSchema::ExternId,
                UserIdentifierSchema::LookalikeValue,
            ],
            data: vec![vec![
                " VALUE ".to_owned(),
                "B67385F8-9A82-4670-8C0A-6F9EA7513F5F".to_owned(),
                " app-user-7 ".to_owned(),
                "synthetic-id".to_owned(),
                " 44.5 ".to_owned(),
            ]],
            app_ids: Some(vec![" 123 ".to_owned()]),
            replace_session: None,
        })
        .unwrap();
        assert!(request.request.method == MutationMethod::Post);
        assert_eq!(request.request.endpoint, "42/users");
        let payload = request
            .request
            .form
            .iter()
            .find(|(key, _)| key == "payload")
            .map(|(_, value)| serde_json::from_str::<Value>(value).unwrap())
            .unwrap();
        assert_eq!(payload["is_raw"], true);
        assert_eq!(
            payload["schema"],
            json!(["EMAIL", "MADID", "APPUID", "EXTERN_ID", "LOOKALIKE_VALUE"])
        );
        assert_eq!(payload["app_ids"], json!(["123"]));
        assert_eq!(
            payload["data"][0],
            json!([
                "cd42404d52ad55ccfa9aca4adc828aa5800ad9d385a0671fbcbf724118320619",
                "b67385f8-9a82-4670-8c0a-6f9ea7513f5f",
                "app-user-7",
                "synthetic-id",
                "44.5"
            ])
        );
        assert!(!request.request.form[0].1.contains(" VALUE "));
    }

    #[test]
    fn encodes_exact_scalar_wires() {
        for (schema, raw, expected, app_id) in [
            (
                UserIdentifierSchema::EmailSha256,
                " VALUE ",
                "cd42404d52ad55ccfa9aca4adc828aa5800ad9d385a0671fbcbf724118320619",
                None,
            ),
            (
                UserIdentifierSchema::PhoneSha256,
                " +1 (555) 123-4567 ",
                "d6736136ea896c1bfdc553e0e86e702c70d060d805696ca3e4e9e0961353860a",
                None,
            ),
            (
                UserIdentifierSchema::MobileAdvertiserId,
                " B67385F8-9A82-4670-8C0A-6F9EA7513F5F ",
                "B67385F8-9A82-4670-8C0A-6F9EA7513F5F",
                None,
            ),
            (
                UserIdentifierSchema::Uid,
                " app-user-token ",
                "app-user-token",
                Some("123"),
            ),
        ] {
            let request = build_users_request(ManageCustomAudienceUsersInput {
                ad_account_id: "123".to_owned(),
                custom_audience_id: "42".to_owned(),
                operation: AudienceUserOperation::Add,
                removal_acknowledgement: None,
                schema: vec![schema],
                data: vec![vec![raw.to_owned()]],
                app_ids: app_id.map(|id| vec![id.to_owned()]),
                replace_session: None,
            })
            .unwrap();
            let payload = request
                .request
                .form
                .iter()
                .find(|(key, _)| key == "payload")
                .map(|(_, value)| serde_json::from_str::<Value>(value).unwrap())
                .unwrap();
            assert_eq!(payload["schema"], schema.as_str());
            assert_eq!(payload["is_raw"], false);
            assert_eq!(payload["data"], json!([expected]));
            if let Some(app_id) = app_id {
                assert_eq!(payload["app_ids"], json!([app_id]));
            } else {
                assert!(payload.get("app_ids").is_none());
            }
        }
    }

    #[test]
    fn rejects_invalid_scalar_and_lookalike_value_modes() {
        let build = |schema, data, app_ids| {
            build_users_request(ManageCustomAudienceUsersInput {
                ad_account_id: "123".to_owned(),
                custom_audience_id: "42".to_owned(),
                operation: AudienceUserOperation::Add,
                removal_acknowledgement: None,
                schema,
                data,
                app_ids,
                replace_session: None,
            })
        };

        assert!(
            build(
                vec![
                    UserIdentifierSchema::EmailSha256,
                    UserIdentifierSchema::Email
                ],
                vec![vec!["value".to_owned(), "value".to_owned()]],
                None,
            )
            .is_err()
        );
        assert!(
            build(
                vec![UserIdentifierSchema::Uid],
                vec![vec!["app-user-token".to_owned()]],
                None,
            )
            .is_err()
        );
        assert!(
            build(
                vec![UserIdentifierSchema::LookalikeValue],
                vec![vec!["1".to_owned()]],
                None,
            )
            .is_err()
        );
        for invalid in ["", "-1", "NaN", "1e309"] {
            assert!(
                build(
                    vec![
                        UserIdentifierSchema::ExternId,
                        UserIdentifierSchema::LookalikeValue
                    ],
                    vec![vec!["synthetic-id".to_owned(), invalid.to_owned()]],
                    None,
                )
                .is_err()
            );
        }
        assert!(
            build(
                vec![
                    UserIdentifierSchema::ExternId,
                    UserIdentifierSchema::LookalikeValue
                ],
                vec![vec![String::new(), "1".to_owned()]],
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn builds_exact_remove_and_session_backed_replace_requests() {
        assert!(
            build_users_request(ManageCustomAudienceUsersInput {
                ad_account_id: "123".to_owned(),
                custom_audience_id: "42".to_owned(),
                operation: AudienceUserOperation::Remove,
                removal_acknowledgement: None,
                schema: vec![UserIdentifierSchema::ExternId],
                data: vec![vec!["synthetic-id".to_owned()]],
                app_ids: None,
                replace_session: None,
            })
            .is_err()
        );

        let remove = build_users_request(ManageCustomAudienceUsersInput {
            ad_account_id: "123".to_owned(),
            custom_audience_id: "42".to_owned(),
            operation: AudienceUserOperation::Remove,
            removal_acknowledgement: Some(REMOVAL_ACKNOWLEDGEMENT.to_owned()),
            schema: vec![UserIdentifierSchema::ExternId],
            data: vec![vec!["synthetic-id".to_owned()]],
            app_ids: None,
            replace_session: None,
        })
        .unwrap();
        assert!(remove.request.method == MutationMethod::Delete);
        assert_eq!(remove.request.endpoint, "42/users");

        let replace = build_users_request(ManageCustomAudienceUsersInput {
            ad_account_id: "123".to_owned(),
            custom_audience_id: "42".to_owned(),
            operation: AudienceUserOperation::Replace,
            removal_acknowledgement: Some(REMOVAL_ACKNOWLEDGEMENT.to_owned()),
            schema: vec![UserIdentifierSchema::ExternId],
            data: vec![vec!["synthetic-id".to_owned()]],
            app_ids: None,
            replace_session: Some(ReplaceAudienceSession {
                session_id: 9_778_993,
                batch_sequence: 1,
                last_batch: true,
            }),
        })
        .unwrap();
        assert_eq!(replace.request.endpoint, "42/usersreplace");
        let session = replace
            .request
            .form
            .iter()
            .find(|(key, _)| key == "session")
            .map(|(_, value)| serde_json::from_str::<Value>(value).unwrap())
            .unwrap();
        assert_eq!(
            session,
            json!({
                "session_id": 9_778_993,
                "batch_seq": 1,
                "last_batch_flag": true
            })
        );
    }

    #[test]
    fn builds_a_typed_country_lookalike_spec() {
        let request = build_lookalike_request(&CreateLookalikeAudienceInput {
            ad_account_id: "123".to_owned(),
            name: " Similar ".to_owned(),
            origin_audience_id: "456".to_owned(),
            lookalike_spec: LookalikeSpecInput {
                country: "us".to_owned(),
                audience_type: None,
                ratio: Some(0.02),
                starting_ratio: Some(0.01),
                allow_international_seeds: Some(false),
            },
            description: None,
        })
        .unwrap();
        assert_eq!(request.endpoint, "act_123/customaudiences");
        let spec = request
            .form
            .iter()
            .find(|(key, _)| key == "lookalike_spec")
            .map(|(_, value)| serde_json::from_str::<Value>(value).unwrap())
            .unwrap();
        assert_eq!(
            spec,
            json!({
                "country": "US",
                "starting_ratio": 0.01,
                "ratio": 0.02,
                "allow_international_seeds": false
            })
        );
    }

    #[test]
    fn rejects_ambiguous_or_unbounded_inputs() {
        let control_name = build_create_request(&CreateCustomAudienceInput {
            ad_account_id: "123".to_owned(),
            name: "Bad\0Name".to_owned(),
            customer_file_source: CustomerFileSource::UserProvidedOnly,
            description: None,
        });
        assert!(control_name.is_err());

        let control_description = build_update_request(&UpdateCustomAudienceInput {
            ad_account_id: "123".to_owned(),
            custom_audience_id: "42".to_owned(),
            name: None,
            description: Some("Bad\0description".to_owned()),
        });
        assert!(control_description.is_err());

        let no_update = build_update_request(&UpdateCustomAudienceInput {
            ad_account_id: "123".to_owned(),
            custom_audience_id: "42".to_owned(),
            name: None,
            description: None,
        });
        assert!(no_update.is_err());

        let duplicate_schema = build_users_request(ManageCustomAudienceUsersInput {
            ad_account_id: "123".to_owned(),
            custom_audience_id: "42".to_owned(),
            operation: AudienceUserOperation::Add,
            removal_acknowledgement: None,
            schema: vec![UserIdentifierSchema::Email, UserIdentifierSchema::Email],
            data: vec![vec!["value".to_owned(), "value".to_owned()]],
            app_ids: None,
            replace_session: None,
        });
        assert!(duplicate_schema.is_err());

        let mismatched_row = build_users_request(ManageCustomAudienceUsersInput {
            ad_account_id: "123".to_owned(),
            custom_audience_id: "42".to_owned(),
            operation: AudienceUserOperation::Add,
            removal_acknowledgement: None,
            schema: vec![UserIdentifierSchema::Email, UserIdentifierSchema::Phone],
            data: vec![vec!["value".to_owned()]],
            app_ids: None,
            replace_session: None,
        });
        assert!(mismatched_row.is_err());

        let oversized_remove = build_users_request(ManageCustomAudienceUsersInput {
            ad_account_id: "123".to_owned(),
            custom_audience_id: "42".to_owned(),
            operation: AudienceUserOperation::Remove,
            removal_acknowledgement: Some(REMOVAL_ACKNOWLEDGEMENT.to_owned()),
            schema: vec![UserIdentifierSchema::ExternId],
            data: vec![vec!["synthetic-id".to_owned()]; 101],
            app_ids: None,
            replace_session: None,
        });
        assert!(oversized_remove.is_err());

        let oversized_payload = build_users_request(ManageCustomAudienceUsersInput {
            ad_account_id: "123".to_owned(),
            custom_audience_id: "42".to_owned(),
            operation: AudienceUserOperation::Add,
            removal_acknowledgement: None,
            schema: vec![UserIdentifierSchema::ExternId],
            data: vec![vec!["x".repeat(50)]; 1_000],
            app_ids: None,
            replace_session: None,
        });
        assert!(oversized_payload.is_err());

        let missing_session = build_users_request(ManageCustomAudienceUsersInput {
            ad_account_id: "123".to_owned(),
            custom_audience_id: "42".to_owned(),
            operation: AudienceUserOperation::Replace,
            removal_acknowledgement: Some(REMOVAL_ACKNOWLEDGEMENT.to_owned()),
            schema: vec![UserIdentifierSchema::ExternId],
            data: vec![vec!["synthetic-id".to_owned()]],
            app_ids: None,
            replace_session: None,
        });
        assert!(missing_session.is_err());

        let bad_ratio = build_lookalike_request(&CreateLookalikeAudienceInput {
            ad_account_id: "123".to_owned(),
            name: "Similar".to_owned(),
            origin_audience_id: "456".to_owned(),
            lookalike_spec: LookalikeSpecInput {
                country: "USA".to_owned(),
                audience_type: None,
                ratio: Some(0.015),
                starting_ratio: None,
                allow_international_seeds: None,
            },
            description: None,
        });
        assert!(bad_ratio.is_err());

        let sensitive_marker = "raw-sensitive-marker";
        let error = normalize_identifier(UserIdentifierSchema::Madid, sensitive_marker)
            .expect_err("invalid MADID must fail locally");
        assert!(!format!("{error:?}").contains(sensitive_marker));
    }

    #[test]
    fn acknowledgements_never_expose_provider_samples_or_identifiers() {
        let payload = json!({
            "audience_id": "42",
            "num_received": 1,
            "num_invalid_entries": 1,
            "invalid_entry_samples": ["must-not-leak"]
        });
        let ack = parse_users_ack(&payload, "42", AudienceUserOperation::Add, 1).unwrap();
        let output = serde_json::to_string(&ack).unwrap();
        assert!(!output.contains("must-not-leak"));
        assert!(!output.contains("payload"));
        assert_eq!(ack.received_rows, Some(1));
        assert_eq!(ack.invalid_rows, Some(1));

        let error = private_data_graph_error(
            GraphError::Api {
                status: 400,
                code: Some(100),
                message: "provider echoed raw-sensitive-marker".to_owned(),
                retryable: false,
            },
            "Verify state",
        );
        assert!(!format!("{error:?}").contains("raw-sensitive-marker"));

        let uncertain = private_data_graph_error(
            GraphError::Transport {
                message: "raw-sensitive-marker".to_owned(),
            },
            "Verify state",
        );
        assert!(!format!("{uncertain:?}").contains("raw-sensitive-marker"));
        assert!(!uncertain.retryable);
        assert_eq!(uncertain.action.as_deref(), Some("Verify state"));
    }
}
