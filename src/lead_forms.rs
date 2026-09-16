// Copyright (C) 2026 ArmaVita LLC
// SPDX-License-Identifier: AGPL-3.0-only

use rmcp::{
    Json,
    handler::server::wrapper::Parameters,
    schemars::{self, JsonSchema},
    tool, tool_router,
};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    graph_tools::{self, GraphData, Params, ReadOptions},
    safety::validate_removal_acknowledgement,
    server::MetaAdsServer,
};

const FORM_FIELDS: &str = "id,name,status,created_time,leads_count";
const FORM_DETAIL_FIELDS: &str = "id,name,status,page_id,locale,questions,privacy_policy_url,thank_you_page,is_optimized_for_quality";
const LEAD_FIELDS: &str = "id,created_time,form_id,ad_id,is_organic";
const LEAD_DETAIL_FIELDS: &str =
    "id,created_time,form_id,ad_id,field_data,custom_disclaimer_responses";
// Meta Business SDK v26.0.1 Page.create_lead_gen_form. File parts are separate inputs.
const CREATE_FIELDS: &[&str] = &[
    "allow_organic_lead_retrieval",
    "block_display_for_non_targeted_viewer",
    "context_card",
    "custom_disclaimer",
    "follow_up_action_url",
    "is_for_canvas",
    "is_lead_capture_ai_agent_enabled",
    "is_optimized_for_quality",
    "is_phone_sms_verify_enabled",
    "locale",
    "name",
    "privacy_policy",
    "question_page_custom_headline",
    "questions",
    "should_enforce_work_email",
    "thank_you_page",
    "tracking_parameters",
];

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadLeadDataInput {
    pub target: LeadReadTarget,
    /// Select extra fields, including field_data for submitted answers; default pages are compact.
    pub options: Option<ReadOptions>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum LeadReadTarget {
    Forms {
        page_id: String,
    },
    Form {
        form_id: String,
    },
    /// Read leads from one form or ad. Requires leads_retrieval and Page lead access.
    Leads {
        source: LeadSource,
        object_id: String,
    },
    Lead {
        lead_id: String,
    },
    TestLeads {
        form_id: String,
    },
    /// Lists the apps subscribed to this Page; no callback hosting is provided here.
    Subscriptions {
        page_id: String,
    },
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LeadSource {
    Form,
    Ad,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateLeadFormInput {
    pub page_id: String,
    /// v26 form fields: name, locale, questions, privacy_policy, plus optional context_card,
    /// custom_disclaimer, thank_you_page, tracking_parameters and quality/consent settings.
    pub fields: Map<String, Value>,
    /// Optional JPEG/PNG below 15 MiB, relative to META_MEDIA_ROOT. Requires pages_manage_posts.
    #[schemars(length(min = 1, max = 1024))]
    pub cover_photo_relative_path: Option<String>,
    /// Optional PDF/JPEG/PNG below 20 MiB, relative to META_MEDIA_ROOT. Requires thank_you_page.button_type=VIEW_ON_FACEBOOK.
    #[schemars(length(min = 1, max = 1024))]
    pub gated_file_relative_path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetLeadFormStatusInput {
    pub page_id: String,
    pub form_id: String,
    pub status: LeadFormStatus,
    /// Required when archiving a form.
    #[schemars(
        length(min = 25, max = 25),
        regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
    )]
    pub removal_acknowledgement: Option<String>,
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum LeadFormStatus {
    Active,
    Archived,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateTestLeadInput {
    pub form_id: String,
    /// Optional answers using question keys. Omit to use Meta's synthetic defaults.
    #[schemars(length(min = 1, max = 25))]
    pub field_data: Option<Vec<TestLeadField>>,
    #[schemars(length(min = 1, max = 25))]
    pub custom_disclaimer_responses: Option<Vec<TestLeadConsent>>,
}

#[derive(Deserialize, serde::Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TestLeadField {
    #[schemars(length(min = 1, max = 256))]
    pub name: String,
    #[schemars(length(min = 1, max = 25), inner(length(min = 1, max = 2048)))]
    pub values: Vec<String>,
}

#[derive(Deserialize, serde::Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TestLeadConsent {
    #[schemars(length(min = 1, max = 256))]
    pub checkbox_key: String,
    pub is_checked: bool,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeleteTestLeadInput {
    /// The lead must appear on this form's test_leads edge before deletion is allowed.
    pub form_id: String,
    pub lead_id: String,
    #[schemars(
        length(min = 25, max = 25),
        regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
    )]
    pub removal_acknowledgement: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManageLeadSubscriptionInput {
    pub page_id: String,
    pub action: LeadSubscriptionAction,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum LeadSubscriptionAction {
    /// Subscribe the configured app to leadgen. Requires a previously configured webhook callback.
    Subscribe,
    /// Disconnect the configured app from this Page, including ALL its other subscribed fields.
    UnsubscribeApp {
        #[schemars(
            length(min = 25, max = 25),
            regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
        )]
        removal_acknowledgement: String,
    },
}

fn invalid(message: &str) -> PublicError {
    PublicError::invalid_input(
        message,
        "Use the documented lead form fields and bounded numeric IDs",
    )
}

fn read_request(input: &ReadLeadDataInput) -> Result<(String, Params), PublicError> {
    let (endpoint, defaults, single) = match &input.target {
        LeadReadTarget::Forms { page_id } => (
            format!("{}/leadgen_forms", graph_tools::id(page_id, "page_id")?),
            FORM_FIELDS,
            false,
        ),
        LeadReadTarget::Form { form_id } => (
            graph_tools::id(form_id, "form_id")?,
            FORM_DETAIL_FIELDS,
            true,
        ),
        LeadReadTarget::Leads { source, object_id } => (
            format!(
                "{}/leads",
                graph_tools::id(
                    object_id,
                    match source {
                        LeadSource::Form => "form_id",
                        LeadSource::Ad => "ad_id",
                    }
                )?
            ),
            LEAD_FIELDS,
            false,
        ),
        LeadReadTarget::Lead { lead_id } => (
            graph_tools::id(lead_id, "lead_id")?,
            LEAD_DETAIL_FIELDS,
            true,
        ),
        LeadReadTarget::TestLeads { form_id } => (
            format!("{}/test_leads", graph_tools::id(form_id, "form_id")?),
            LEAD_FIELDS,
            false,
        ),
        LeadReadTarget::Subscriptions { page_id } => (
            format!("{}/subscribed_apps", graph_tools::id(page_id, "page_id")?),
            "id,name,subscribed_fields",
            false,
        ),
    };
    let default_options = ReadOptions::default();
    let options = input.options.as_ref().unwrap_or(&default_options);
    if single && (options.page_cursor.is_some() || options.page_size.is_some()) {
        return Err(invalid("Pagination applies only to list operations"));
    }
    let mut params = graph_tools::read_params(options, defaults)?;
    if single {
        params.retain(|(key, _)| key != "limit");
    }
    Ok((endpoint, params))
}

fn create_request(input: &CreateLeadFormInput) -> Result<(String, Params), PublicError> {
    let page_id = graph_tools::id(&input.page_id, "page_id")?;
    for key in ["name", "locale"] {
        let value = input
            .fields
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("name and locale must be nonempty strings"))?;
        graph_tools::text(value, key, if key == "name" { 255 } else { 10 })?;
    }
    let questions = input
        .fields
        .get("questions")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("questions must contain 1 through 25 question objects"))?;
    if questions.is_empty()
        || questions.len() > 25
        || questions.iter().any(|q| {
            !q.is_object()
                || q.get("type")
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
        })
    {
        return Err(invalid(
            "questions must contain 1 through 25 objects with a type",
        ));
    }
    let privacy = input
        .fields
        .get("privacy_policy")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("privacy_policy must contain a url"))?;
    let url = privacy
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("privacy_policy must contain a url"))?;
    validate_https(url)?;
    if input.gated_file_relative_path.is_some()
        && input
            .fields
            .get("thank_you_page")
            .and_then(|page| page.get("button_type"))
            .and_then(Value::as_str)
            != Some("VIEW_ON_FACEBOOK")
    {
        return Err(invalid(
            "A gated file requires thank_you_page.button_type=VIEW_ON_FACEBOOK",
        ));
    }
    for key in ["follow_up_action_url"] {
        if let Some(value) = input.fields.get(key) {
            validate_https(
                value
                    .as_str()
                    .ok_or_else(|| invalid("Form URLs must be strings"))?,
            )?;
        }
    }
    let params = graph_tools::form_fields(&input.fields, CREATE_FIELDS)?;
    Ok((format!("{page_id}/leadgen_forms"), params))
}

async fn create_form(
    graph: &GraphClient,
    media_root: Option<&std::path::Path>,
    input: CreateLeadFormInput,
) -> ToolResponse<GraphData> {
    let (endpoint, params) = match create_request(&input) {
        Ok(value) => value,
        Err(error) => return ToolResponse::error(error),
    };
    let mut files = Vec::new();
    for (path, field, kind, max) in [
        (
            input.cover_photo_relative_path.as_deref(),
            "cover_photo",
            crate::media_uploads::LocalMediaKind::Image,
            15 * 1024 * 1024,
        ),
        (
            input.gated_file_relative_path.as_deref(),
            "upload_gated_file",
            crate::media_uploads::LocalMediaKind::LeadDocument,
            20 * 1024 * 1024,
        ),
    ] {
        if let Some(path) = path {
            let opened =
                match crate::media_uploads::open_local_media(media_root, path, kind, max).await {
                    Ok(value) => value,
                    Err(error) => return ToolResponse::error(error),
                };
            files.push(crate::graph::UploadFile {
                field,
                file: opened.file,
                size: opened.size,
                name: opened.file_name.into(),
                mime: opened.mime_type,
            });
        }
    }
    let graph = match graph.for_page(input.page_id.trim()).await {
        Ok(graph) => graph,
        Err(error) => return ToolResponse::error(error),
    };
    if files.is_empty() {
        return graph_tools::write(&graph, &endpoint, params).await;
    }
    let result = graph.post_multipart_files_json(&endpoint,files,params).await
        .map_err(|error| crate::mutation_result::mutation_error_without_blind_retry(error,"Read the Page's lead forms before retrying; Meta may already have created this form"))
        .and_then(graph_tools::normalize_write);
    graph_tools::response(result)
}

fn validate_https(raw: &str) -> Result<(), PublicError> {
    let url = reqwest::Url::parse(raw)
        .map_err(|_| invalid("Form links must be credential-free HTTPS URLs"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || crate::bounded_json::credential_value(raw)
    {
        return Err(invalid("Form links must be credential-free HTTPS URLs"));
    }
    Ok(())
}

fn test_lead_request(input: &CreateTestLeadInput) -> Result<(String, Params), PublicError> {
    let form_id = graph_tools::id(&input.form_id, "form_id")?;
    let mut params = Vec::new();
    if let Some(fields) = &input.field_data {
        if fields.is_empty() || fields.len() > 25 {
            return Err(invalid("field_data must contain 1 through 25 answers"));
        }
        for field in fields {
            graph_tools::text(&field.name, "field_data.name", 256)?;
            if field.values.is_empty() || field.values.len() > 25 {
                return Err(invalid("Each answer must contain 1 through 25 values"));
            }
            for value in &field.values {
                graph_tools::text(value, "field_data.values", 2048)?;
            }
        }
        params.push((
            "field_data".into(),
            graph_tools::json(
                &serde_json::to_value(fields).map_err(|_| invalid("Invalid test lead answers"))?,
                "field_data",
            )?,
        ));
    }
    if let Some(consents) = &input.custom_disclaimer_responses {
        if consents.is_empty() || consents.len() > 25 {
            return Err(invalid(
                "custom_disclaimer_responses must contain 1 through 25 entries",
            ));
        }
        for consent in consents {
            graph_tools::text(&consent.checkbox_key, "checkbox_key", 256)?;
        }
        params.push((
            "custom_disclaimer_responses".into(),
            graph_tools::json(
                &serde_json::to_value(consents)
                    .map_err(|_| invalid("Invalid test lead consents"))?,
                "custom_disclaimer_responses",
            )?,
        ));
    }
    Ok((format!("{form_id}/test_leads"), params))
}

async fn set_status(graph: &GraphClient, input: SetLeadFormStatusInput) -> ToolResponse<GraphData> {
    let prepared = (|| {
        let page_id = graph_tools::id(&input.page_id, "page_id")?;
        let form_id = graph_tools::id(&input.form_id, "form_id")?;
        validate_removal_acknowledgement(
            matches!(input.status, LeadFormStatus::Archived),
            input.removal_acknowledgement.as_deref(),
        )?;
        Ok::<_, PublicError>((page_id, form_id))
    })();
    let (page_id, form_id) = match prepared {
        Ok(value) => value,
        Err(error) => return ToolResponse::error(error),
    };
    let current = match graph
        .get_json(&form_id, &[("fields".into(), "id,page_id".into())])
        .await
    {
        Ok(value) => value,
        Err(error) => return ToolResponse::error(error),
    };
    if current
        .get("id")
        .and_then(crate::meta_ids::numeric_value)
        .as_deref()
        != Some(&form_id)
        || current
            .get("page_id")
            .and_then(crate::meta_ids::numeric_value)
            .as_deref()
            != Some(&page_id)
    {
        return ToolResponse::error(invalid("The form does not belong to the supplied Page"));
    }
    let status = match input.status {
        LeadFormStatus::Active => "ACTIVE",
        LeadFormStatus::Archived => "ARCHIVED",
    };
    graph_tools::write(graph, &form_id, vec![("status".into(), status.into())]).await
}

async fn delete_test(graph: &GraphClient, input: DeleteTestLeadInput) -> ToolResponse<GraphData> {
    let prepared = (|| {
        let form_id = graph_tools::id(&input.form_id, "form_id")?;
        let lead_id = graph_tools::id(&input.lead_id, "lead_id")?;
        validate_removal_acknowledgement(true, Some(&input.removal_acknowledgement))?;
        Ok::<_, PublicError>((form_id, lead_id))
    })();
    let (form_id, lead_id) = match prepared {
        Ok(value) => value,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph
        .get_json(
            &format!("{form_id}/test_leads"),
            &[
                ("fields".into(), "id".into()),
                ("limit".into(), "25".into()),
            ],
        )
        .await
    {
        Ok(value) => value,
        Err(error) => return ToolResponse::error(error),
    };
    let Some(leads) = payload.get("data").and_then(Value::as_array) else {
        return ToolResponse::error(PublicError::invalid_upstream(
            "Meta returned an invalid test lead list",
        ));
    };
    if leads.len() > 25
        || !leads.iter().any(|lead| {
            lead.get("id")
                .and_then(crate::meta_ids::numeric_value)
                .as_deref()
                == Some(&lead_id)
        })
    {
        return ToolResponse::error(invalid(
            "The lead is not a test lead for this form; deletion was refused",
        ));
    }
    graph_tools::delete(graph, &lead_id, Vec::new()).await
}

async fn form_graph(graph: &GraphClient, form_id: &str) -> Result<GraphClient, PublicError> {
    let form =
        crate::node_identity::verify_object(graph, form_id, "id,page_id", &["page_id"]).await?;
    let page_id = form
        .get("page_id")
        .and_then(crate::meta_ids::numeric_value)
        .ok_or_else(|| invalid("Meta did not return the form's owning Page"))?;
    graph.for_page(&page_id).await.map_err(PublicError::from)
}

async fn create_test(graph: &GraphClient, input: CreateTestLeadInput) -> ToolResponse<GraphData> {
    let (endpoint, params) = match test_lead_request(&input) {
        Ok(value) => value,
        Err(error) => return ToolResponse::error(error),
    };
    match form_graph(graph, &input.form_id).await {
        Ok(graph) => graph_tools::write(&graph, &endpoint, params).await,
        Err(error) => ToolResponse::error(error),
    }
}

#[tool_router(router = lead_forms_router, vis = "pub(crate)")]
impl MetaAdsServer {
    #[tool(
        name = "read_lead_data",
        description = "Read Instant Forms, submitted leads, test leads, or Page webhook subscriptions. Compact pages omit lead answers unless fields requests field_data; single-lead reads include answers. Requires Page/lead access.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn read_lead_data(
        &self,
        Parameters(input): Parameters<ReadLeadDataInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        let response = match read_request(&input) {
            Ok((endpoint, params)) => {
                let graph = match &input.target {
                    LeadReadTarget::Forms { page_id }
                    | LeadReadTarget::Subscriptions { page_id } => {
                        match self.graph.for_page(page_id.trim()).await {
                            Ok(graph) => graph,
                            Err(error) => {
                                return ToolResponse::<GraphData>::error(error).into_mcp_result();
                            }
                        }
                    }
                    LeadReadTarget::TestLeads { form_id } => {
                        match form_graph(&self.graph, form_id).await {
                            Ok(graph) => graph,
                            Err(error) => {
                                return ToolResponse::<GraphData>::error(error).into_mcp_result();
                            }
                        }
                    }
                    _ => self.graph.clone(),
                };
                graph_tools::read(&graph, &endpoint, params).await
            }
            Err(error) => ToolResponse::error(error),
        };
        response.into_mcp_result()
    }

    #[tool(
        name = "create_lead_form",
        description = "Create an Instant Form on a Page using bounded v26 questions, privacy policy, thank-you page and quality settings. Requires Page advertising access; content changes require a new form.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_lead_form(
        &self,
        Parameters(input): Parameters<CreateLeadFormInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        create_form(&self.graph, self.media_root.as_deref(), input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "set_lead_form_status",
        description = "Archive or reactivate a Page's Instant Form. Archiving requires CONFIRM_META_ADS_REMOVALS. Meta does not support deleting forms or editing their content.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn set_lead_form_status(
        &self,
        Parameters(input): Parameters<SetLeadFormStatusInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        let checked = graph_tools::id(&input.page_id, "page_id")
            .and_then(|_| graph_tools::id(&input.form_id, "form_id"))
            .and_then(|_| {
                validate_removal_acknowledgement(
                    matches!(input.status, LeadFormStatus::Archived),
                    input.removal_acknowledgement.as_deref(),
                )
            });
        if let Err(error) = checked {
            return ToolResponse::<GraphData>::error(error).into_mcp_result();
        }
        match self.graph.for_page(input.page_id.trim()).await {
            Ok(graph) => set_status(&graph, input).await,
            Err(error) => ToolResponse::error(error),
        }
        .into_mcp_result()
    }

    #[tool(
        name = "create_test_lead",
        description = "Create one synthetic lead for an Instant Form to test lead retrieval and webhooks. Meta allows one test lead per form; delete the existing test lead before creating another.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_test_lead(
        &self,
        Parameters(input): Parameters<CreateTestLeadInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        create_test(&self.graph, input).await.into_mcp_result()
    }

    #[tool(
        name = "delete_test_lead",
        description = "Delete a lead only after verifying it belongs to the form's test_leads edge. Requires CONFIRM_META_ADS_REMOVALS; never deletes submitted production leads.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn delete_test_lead(
        &self,
        Parameters(input): Parameters<DeleteTestLeadInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        let checked = graph_tools::id(&input.form_id, "form_id")
            .and_then(|_| graph_tools::id(&input.lead_id, "lead_id"))
            .and_then(|_| {
                validate_removal_acknowledgement(true, Some(&input.removal_acknowledgement))
            });
        if let Err(error) = checked {
            return ToolResponse::<GraphData>::error(error).into_mcp_result();
        }
        match form_graph(&self.graph, &input.form_id).await {
            Ok(graph) => delete_test(&graph, input).await,
            Err(error) => ToolResponse::error(error),
        }
        .into_mcp_result()
    }

    #[tool(
        name = "manage_lead_subscription",
        description = "Subscribe the configured app to a Page's leadgen webhook, or disconnect the app and ALL its Page subscriptions with removal acknowledgement. Requires Page metadata permission and an existing hosted webhook callback.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn manage_lead_subscription(
        &self,
        Parameters(input): Parameters<ManageLeadSubscriptionInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        let page_id = match graph_tools::id(&input.page_id, "page_id") {
            Ok(value) => value,
            Err(error) => return ToolResponse::<GraphData>::error(error).into_mcp_result(),
        };
        let endpoint = format!("{page_id}/subscribed_apps");
        if let LeadSubscriptionAction::UnsubscribeApp {
            removal_acknowledgement,
        } = &input.action
            && let Err(error) =
                validate_removal_acknowledgement(true, Some(removal_acknowledgement))
        {
            return ToolResponse::<GraphData>::error(error).into_mcp_result();
        }
        let graph = match self.graph.for_page(&page_id).await {
            Ok(graph) => graph,
            Err(error) => return ToolResponse::<GraphData>::error(error).into_mcp_result(),
        };
        let response = match input.action {
            LeadSubscriptionAction::Subscribe => {
                graph_tools::write(
                    &graph,
                    &endpoint,
                    vec![("subscribed_fields".into(), "[\"leadgen\"]".into())],
                )
                .await
            }
            LeadSubscriptionAction::UnsubscribeApp {
                removal_acknowledgement,
            } => match validate_removal_acknowledgement(true, Some(&removal_acknowledgement)) {
                Ok(()) => graph_tools::delete(&graph, &endpoint, Vec::new()).await,
                Err(error) => ToolResponse::error(error),
            },
        };
        response.into_mcp_result()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MetaConfig, safety::REMOVAL_ACKNOWLEDGEMENT};
    use serde_json::json;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    fn form() -> CreateLeadFormInput {
        serde_json::from_value(json!({"page_id":"10","fields":{
            "name":"Get a quote","locale":"EN_US","questions":[{"type":"EMAIL","key":"email"}],
            "privacy_policy":{"url":"https://example.com/privacy"},"is_optimized_for_quality":true
        }}))
        .unwrap()
    }

    #[test]
    fn bounded_form_contract_rejects_credentials_files_and_unsupported_lifecycle() {
        let (endpoint, params) = create_request(&form()).unwrap();
        assert_eq!(endpoint, "10/leadgen_forms");
        assert!(params.contains(&("is_optimized_for_quality".into(), "true".into())));
        for (key, value) in [
            ("access_token", json!("secret")),
            ("status", json!("DELETED")),
            ("cover_photo", json!("/tmp/photo.jpg")),
            ("questions", json!([])),
        ] {
            let mut input = form();
            input.fields.insert(key.into(), value);
            assert!(create_request(&input).is_err(), "{key}");
        }
        for url in [
            "http://example.com/privacy",
            "https://secret@example.com",
            "https://example.com/?access_token=secret",
        ] {
            let mut input = form();
            input
                .fields
                .insert("privacy_policy".into(), json!({"url":url}));
            assert!(create_request(&input).is_err());
        }
        assert!(
            serde_json::from_value::<SetLeadFormStatusInput>(
                json!({"page_id":"10","form_id":"20","status":"DELETED"})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<CreateTestLeadInput>(
                json!({"form_id":"20","access_token":"secret"})
            )
            .is_err()
        );
        let input: CreateTestLeadInput = serde_json::from_value(
            json!({"form_id":"20","field_data":[{"name":"email","values":[]}]}),
        )
        .unwrap();
        assert!(test_lead_request(&input).is_err());
        let mut gated = form();
        gated.gated_file_relative_path = Some("guide.pdf".into());
        assert!(create_request(&gated).is_err());
        gated.fields.insert(
            "thank_you_page".into(),
            json!({"button_type":"VIEW_ON_FACEBOOK"}),
        );
        assert!(create_request(&gated).is_ok());
    }

    #[test]
    fn read_routes_are_bounded_and_answers_are_explicit() {
        let input: ReadLeadDataInput = serde_json::from_value(
            json!({"target":{"kind":"leads","source":"form","object_id":"20"}}),
        )
        .unwrap();
        let (endpoint, params) = read_request(&input).unwrap();
        assert_eq!(endpoint, "20/leads");
        assert!(params.contains(&("limit".into(), "25".into())));
        assert!(!params.iter().any(|(_, value)| value.contains("field_data")));
        let input: ReadLeadDataInput =
            serde_json::from_value(json!({"target":{"kind":"lead","lead_id":"30"}})).unwrap();
        let (_, params) = read_request(&input).unwrap();
        assert!(params.iter().any(|(_, value)| value.contains("field_data")));
        assert!(!params.iter().any(|(key, _)| key == "limit"));
        let input: ReadLeadDataInput =
            serde_json::from_value(json!({"target":{"kind":"forms","page_id":"10/other"}}))
                .unwrap();
        assert!(read_request(&input).is_err());
    }

    #[tokio::test]
    async fn wire_checks_prevent_wrong_page_archives_and_production_lead_deletion() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{}", listener.local_addr().unwrap()),
            Some("test-token"),
        ))
        .unwrap();
        let server = tokio::spawn(async move {
            let replies = [
                json!({"id":"20","page_id":"99"}),
                json!({"id":"20","page_id":"10"}),
                json!({"success":true}),
                json!({"data":[{"id":"30"}]}),
                json!({"data":[{"id":"30"}]}),
                json!({"success":true}),
                json!({"id":"20","page_id":"10"}),
                json!({"access_token":"test-page-token"}),
                json!({"id":"31"}),
            ];
            let mut requests = Vec::new();
            for reply in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0_u8; 1024];
                    let count = socket.read(&mut chunk).await.unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&chunk[..count]);
                    if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8(request).unwrap());
                let body = reply.to_string();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
            }
            requests
        });
        for (page_id, success) in [("10", false), ("10", true)] {
            let response = set_status(
                &graph,
                SetLeadFormStatusInput {
                    page_id: page_id.into(),
                    form_id: "20".into(),
                    status: LeadFormStatus::Archived,
                    removal_acknowledgement: Some(REMOVAL_ACKNOWLEDGEMENT.into()),
                },
            )
            .await;
            assert_eq!(matches!(response, ToolResponse::Success { .. }), success);
        }
        for (lead_id, success) in [("31", false), ("30", true)] {
            let response = delete_test(
                &graph,
                DeleteTestLeadInput {
                    form_id: "20".into(),
                    lead_id: lead_id.into(),
                    removal_acknowledgement: REMOVAL_ACKNOWLEDGEMENT.into(),
                },
            )
            .await;
            assert_eq!(matches!(response, ToolResponse::Success { .. }), success);
        }
        let response = create_test(
            &graph,
            CreateTestLeadInput {
                form_id: "20".into(),
                field_data: None,
                custom_disclaimer_responses: None,
            },
        )
        .await;
        assert!(matches!(response, ToolResponse::Success { .. }));
        let requests = server.await.unwrap();
        assert!(requests[0].starts_with("GET /20?"));
        assert!(requests[2].starts_with("POST /20 "));
        assert!(requests[2].ends_with("status=ARCHIVED"));
        assert!(requests[3].starts_with("GET /20/test_leads?"));
        assert!(requests[5].starts_with("DELETE /30 "));
        assert!(requests[6].starts_with("GET /20?fields=id%2Cpage_id "));
        assert!(requests[7].starts_with("GET /10?fields=access_token "));
        assert!(requests[8].starts_with("POST /20/test_leads "));
        assert!(
            requests[8]
                .to_ascii_lowercase()
                .contains("authorization: bearer test-page-token\r\n")
        );
    }
}
