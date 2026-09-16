// Copyright (C) 2025 ArmaVita LLC
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
    ad_automation::acknowledge,
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    graph_tools::{self, GraphData, Params, ReadOptions},
    node_identity::verify_object,
    safety::validate_removal_acknowledgement,
    server::MetaAdsServer,
};

const CANVAS_FIELDS: &[&str] = &[
    "background_color",
    "body_element_ids",
    "enable_swipe_to_open",
    "hero_asset_facebook_post_id",
    "hero_asset_instagram_media_id",
    "is_hidden",
    "is_published",
    "name",
    "source_template_id",
];
const SUMMARY: &str = "id,name,is_published,is_hidden,canvas_link,update_time";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExperienceReadInput {
    pub page_id: String,
    pub operation: ExperienceRead,
    #[serde(default)]
    pub options: ReadOptions,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ExperienceRead {
    List {
        is_hidden: Option<bool>,
        is_published: Option<bool>,
    },
    Read {
        experience_id: String,
    },
    ListElements,
    ReadElement {
        element_id: String,
    },
    /// Returns bounded preview HTML; does not notify other users.
    Preview {
        experience_id: String,
    },
    /// Read the template's document ID, then read that document's body_elements.
    Template {
        template_id: String,
    },
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExperienceChangeInput {
    pub page_id: String,
    pub operation: ExperienceChange,
    #[schemars(
        length(min = 27, max = 27),
        regex(pattern = "^APPLY_LIVE_META_ADS_CHANGES$")
    )]
    pub apply_acknowledgement: String,
    /// Required to hide an experience or delete an element.
    #[schemars(
        length(min = 25, max = 25),
        regex(pattern = "^CONFIRM_META_ADS_REMOVALS$")
    )]
    pub removal_acknowledgement: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ExperienceChange {
    /// Creates a draft. Publish it separately after reviewing the preview.
    Create {
        /// name, background_color, body_element_ids, enable_swipe_to_open, hero_asset_facebook_post_id, hero_asset_instagram_media_id, source_template_id.
        fields: Map<String, Value>,
    },
    /// Only unpublished experiences can be edited.
    Update {
        experience_id: String,
        fields: Map<String, Value>,
    },
    Publish {
        experience_id: String,
    },
    /// Meta supports hiding an experience, not deleting the whole Canvas.
    SetHidden {
        experience_id: String,
        hidden: bool,
    },
    CreateElement {
        kind: CanvasElementKind,
        fields: Map<String, Value>,
    },
    /// Supported by the Instant Experiences guide; may be restricted if the element is in use.
    DeleteElement {
        element_id: String,
    },
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CanvasElementKind {
    Button,
    Carousel,
    ExistingPost,
    Footer,
    Header,
    LeadForm,
    Photo,
    ProductList,
    ProductSet,
    StoreLocator,
    TemplateVideo,
    Text,
    Video,
}

impl CanvasElementKind {
    fn field(&self) -> &'static str {
        match self {
            Self::Button => "canvas_button",
            Self::Carousel => "canvas_carousel",
            Self::ExistingPost => "canvas_existing_post",
            Self::Footer => "canvas_footer",
            Self::Header => "canvas_header",
            Self::LeadForm => "canvas_lead_form",
            Self::Photo => "canvas_photo",
            Self::ProductList => "canvas_product_list",
            Self::ProductSet => "canvas_product_set",
            Self::StoreLocator => "canvas_store_locator",
            Self::TemplateVideo => "canvas_template_video",
            Self::Text => "canvas_text",
            Self::Video => "canvas_video",
        }
    }
}

fn read_request(input: &ExperienceReadInput) -> Result<(String, Params), PublicError> {
    let page = graph_tools::id(&input.page_id, "page_id")?;
    let (endpoint, defaults) = match &input.operation {
        ExperienceRead::List { .. } => (format!("{page}/canvases"), SUMMARY),
        ExperienceRead::Read { experience_id } => {
            (graph_tools::id(experience_id, "experience_id")?, SUMMARY)
        }
        ExperienceRead::ListElements => (format!("{page}/canvas_elements"), "element"),
        ExperienceRead::ReadElement { element_id } => (
            graph_tools::id(element_id, "element_id")?,
            "id,name,element_type",
        ),
        ExperienceRead::Preview { experience_id } => (
            format!(
                "{}/preview",
                graph_tools::id(experience_id, "experience_id")?
            ),
            "body",
        ),
        ExperienceRead::Template { template_id } => {
            (graph_tools::id(template_id, "template_id")?, "id,document")
        }
    };
    let mut params = graph_tools::read_params(&input.options, defaults)?;
    if let ExperienceRead::List {
        is_hidden,
        is_published,
    } = input.operation
    {
        for (key, value) in [("is_hidden", is_hidden), ("is_published", is_published)] {
            if let Some(value) = value {
                params.push((key.into(), value.to_string()));
            }
        }
    }
    Ok((endpoint, params))
}

struct ExperienceWrite {
    endpoint: String,
    params: Params,
    check: ExperienceCheck,
}

enum ExperienceCheck {
    None,
    Draft,
    Canvas,
    DeleteElement,
}

fn write_request(input: &ExperienceChangeInput) -> Result<ExperienceWrite, PublicError> {
    let page = graph_tools::id(&input.page_id, "page_id")?;
    acknowledge(true, Some(&input.apply_acknowledgement))?;
    let destructive = matches!(
        input.operation,
        ExperienceChange::DeleteElement { .. } | ExperienceChange::SetHidden { hidden: true, .. }
    );
    validate_removal_acknowledgement(destructive, input.removal_acknowledgement.as_deref())?;
    let (endpoint, params, check) = match &input.operation {
        ExperienceChange::Create { fields } => {
            if !fields.contains_key("name") {
                return Err(invalid("name is required"));
            }
            if fields
                .get("is_published")
                .is_some_and(|v| v != &Value::Bool(false))
            {
                return Err(invalid(
                    "Create a draft, then use publish after reviewing it",
                ));
            }
            let mut fields = fields.clone();
            fields.insert("is_published".into(), Value::Bool(false));
            (
                format!("{page}/canvases"),
                canvas_params(&fields)?,
                ExperienceCheck::None,
            )
        }
        ExperienceChange::Update {
            experience_id,
            fields,
        } => {
            if fields.contains_key("is_published") || fields.contains_key("is_hidden") {
                return Err(invalid(
                    "Use publish or set_hidden to change publication or visibility",
                ));
            }
            (
                graph_tools::id(experience_id, "experience_id")?,
                canvas_params(fields)?,
                ExperienceCheck::Draft,
            )
        }
        ExperienceChange::Publish { experience_id } => (
            graph_tools::id(experience_id, "experience_id")?,
            vec![("is_published".into(), "true".into())],
            ExperienceCheck::Draft,
        ),
        ExperienceChange::SetHidden {
            experience_id,
            hidden,
        } => (
            graph_tools::id(experience_id, "experience_id")?,
            vec![("is_hidden".into(), hidden.to_string())],
            ExperienceCheck::Canvas,
        ),
        ExperienceChange::CreateElement { kind, fields } => {
            if fields.is_empty() {
                return Err(invalid("Element fields cannot be empty"));
            }
            let body = Map::from_iter([(kind.field().to_owned(), Value::Object(fields.clone()))]);
            (
                format!("{page}/canvas_elements"),
                graph_tools::form_fields(&body, &[kind.field()])?,
                ExperienceCheck::None,
            )
        }
        ExperienceChange::DeleteElement { element_id } => (
            graph_tools::id(element_id, "element_id")?,
            Vec::new(),
            ExperienceCheck::DeleteElement,
        ),
    };
    Ok(ExperienceWrite {
        endpoint,
        params,
        check,
    })
}

fn canvas_params(fields: &Map<String, Value>) -> Result<Params, PublicError> {
    if fields.is_empty() {
        return Err(invalid("fields must include at least one change"));
    }
    if let Some(name) = fields.get("name") {
        graph_tools::text(
            name.as_str()
                .ok_or_else(|| invalid("name must be a string"))?,
            "name",
            256,
        )?;
    }
    for key in ["is_hidden", "is_published", "enable_swipe_to_open"] {
        if fields.get(key).is_some_and(|v| !v.is_boolean()) {
            return Err(invalid("Visibility and swipe options must be booleans"));
        }
    }
    for key in [
        "hero_asset_facebook_post_id",
        "hero_asset_instagram_media_id",
        "source_template_id",
    ] {
        if let Some(value) = fields.get(key) {
            graph_tools::id(
                value
                    .as_str()
                    .ok_or_else(|| invalid("Asset and template IDs must be numeric strings"))?,
                key,
            )?;
        }
    }
    if let Some(value) = fields.get("body_element_ids") {
        let values = value
            .as_array()
            .filter(|v| !v.is_empty() && v.len() <= 100)
            .ok_or_else(|| invalid("body_element_ids must contain 1–100 numeric IDs"))?;
        for value in values {
            graph_tools::id(
                value
                    .as_str()
                    .ok_or_else(|| invalid("body_element_ids must contain numeric strings"))?,
                "body_element_ids",
            )?;
        }
    }
    if let Some(value) = fields.get("background_color")
        && !value
            .as_str()
            .is_some_and(|v| matches!(v.len(), 6 | 8) && v.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(invalid(
            "background_color must be six or eight hexadecimal digits",
        ));
    }
    graph_tools::form_fields(fields, CANVAS_FIELDS)
}

fn unpublished(payload: &Value) -> Result<(), PublicError> {
    match payload.get("is_published").and_then(Value::as_bool) {
        Some(false) => Ok(()),
        Some(true) => Err(invalid(
            "Published Instant Experiences cannot be edited; create a new draft",
        )),
        None => Err(PublicError::invalid_upstream(
            "Meta did not return the publication state",
        )),
    }
}

async fn write_experience(
    graph: &GraphClient,
    page_id: &str,
    request: ExperienceWrite,
) -> ToolResponse<GraphData> {
    match request.check {
        ExperienceCheck::None => {}
        ExperienceCheck::Draft | ExperienceCheck::Canvas => {
            let payload = match verify_object(
                graph,
                &request.endpoint,
                "canvas_link,is_published,owner",
                &["canvas_link", "is_published", "owner"],
            )
            .await
            {
                Ok(value) => value,
                Err(error) => return ToolResponse::error(error),
            };
            if payload.pointer("/owner/id").and_then(Value::as_str) != Some(page_id) {
                return ToolResponse::error(invalid(
                    "The Instant Experience belongs to a different Page",
                ));
            }
            if matches!(request.check, ExperienceCheck::Draft)
                && let Err(error) = unpublished(&payload)
            {
                return ToolResponse::error(error);
            }
        }
        ExperienceCheck::DeleteElement => {
            if let Err(error) =
                verify_object(graph, &request.endpoint, "element_type", &["element_type"]).await
            {
                return ToolResponse::error(error);
            }
            return graph_tools::delete(graph, &request.endpoint, request.params).await;
        }
    }
    graph_tools::write(graph, &request.endpoint, request.params).await
}

fn invalid(message: &str) -> PublicError {
    PublicError::invalid_input(
        message,
        "Use a Page you manage and the documented Instant Experience fields",
    )
}

#[tool_router(router = experiences_router, vis = "pub(crate)")]
impl MetaAdsServer {
    #[tool(
        name = "read_instant_experiences",
        description = "List Page Instant Experiences, inspect elements/templates, or return preview HTML. Uses Page authorization internally.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn read_instant_experiences(
        &self,
        Parameters(input): Parameters<ExperienceReadInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        let (endpoint, params) = match read_request(&input) {
            Ok(value) => value,
            Err(error) => return ToolResponse::<GraphData>::error(error).into_mcp_result(),
        };
        let page_id = graph_tools::id(&input.page_id, "page_id").expect("validated Page ID");
        let graph = match self.graph.for_page(&page_id).await {
            Ok(graph) => graph,
            Err(error) => return ToolResponse::<GraphData>::error(error).into_mcp_result(),
        };
        graph_tools::read(&graph, &endpoint, params)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "manage_instant_experiences",
        description = "Create drafts/elements, edit unpublished experiences, publish, hide or delete elements. Whole-experience deletion is not supported by Meta.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn manage_instant_experiences(
        &self,
        Parameters(input): Parameters<ExperienceChangeInput>,
    ) -> Result<Json<ToolResponse<GraphData>>, Json<ToolResponse<GraphData>>> {
        let request = match write_request(&input) {
            Ok(value) => value,
            Err(error) => return ToolResponse::<GraphData>::error(error).into_mcp_result(),
        };
        let page = match graph_tools::id(&input.page_id, "page_id") {
            Ok(value) => value,
            Err(error) => return ToolResponse::<GraphData>::error(error).into_mcp_result(),
        };
        let graph = match self.graph.for_page(&page).await {
            Ok(graph) => graph,
            Err(error) => return ToolResponse::<GraphData>::error(error).into_mcp_result(),
        };
        write_experience(&graph, &page, request)
            .await
            .into_mcp_result()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input(operation: Value) -> ExperienceChangeInput {
        serde_json::from_value(json!({"page_id":"10","apply_acknowledgement":"APPLY_LIVE_META_ADS_CHANGES","operation":operation})).unwrap()
    }

    #[test]
    fn canvas_lifecycle_is_draft_first_and_elements_use_their_documented_edge() {
        let request = write_request(&input(
            json!({"action":"create","fields":{"name":"Offer","body_element_ids":["2"]}}),
        ))
        .unwrap();
        assert_eq!(request.endpoint, "10/canvases");
        assert!(
            request
                .params
                .contains(&("is_published".into(), "false".into()))
        );
        assert!(
            write_request(&input(
                json!({"action":"create","fields":{"name":"Offer","is_published":true}})
            ))
            .is_err()
        );
        let element = write_request(&input(json!({"action":"create_element","kind":"photo","fields":{"photo_id":"2","style":"FIT_TO_WIDTH"}}))).unwrap();
        assert_eq!(element.endpoint, "10/canvas_elements");
        assert_eq!(element.params[0].0, "canvas_photo");
        assert!(write_request(&input(json!({"action":"create_element","kind":"photo","fields":{"url":"https://example.com/?access_token=secret"}}))).is_err());
        assert!(
            write_request(&input(json!({"action":"delete_element","element_id":"2"}))).is_err()
        );
        assert!(unpublished(&json!({"is_published":true})).is_err());
        assert!(unpublished(&json!({})).is_err());
        assert!(unpublished(&json!({"is_published":false})).is_ok());
    }
}
