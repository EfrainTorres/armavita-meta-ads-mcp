// SPDX-License-Identifier: AGPL-3.0-only

//! `batch_read`: up to 50 read-only Graph GETs in one request.

use std::collections::BTreeMap;

use rmcp::{
    Json,
    handler::server::wrapper::Parameters,
    schemars::{self, JsonSchema},
    tool, tool_router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::{PublicError, ToolResponse},
    graph::{BatchItem, GraphClient},
    server::MetaAdsServer,
};

const MAX_REQUESTS: usize = 50;
const MAX_FIELDS: usize = 50;
const MAX_PARAMS: usize = 20;
const MAX_FIELD_CHARS: usize = 64;
const MAX_PARAM_VALUE_CHARS: usize = 2_048;

/// Validated (endpoint, query) pairs ready for `GraphClient::get_batch`.
type BatchRequests = Vec<(String, Vec<(String, String)>)>;

/// One GET in the batch.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchReadRequest {
    /// Relative Graph path such as `act_123/campaigns` or `120200000000000`.
    #[schemars(length(min = 1, max = 512))]
    pub endpoint: String,
    /// Simple field names, joined into `fields`. No field expansion.
    #[schemars(length(min = 1, max = 50))]
    pub fields: Option<Vec<String>>,
    /// Other query parameters, e.g. `{"limit": "25", "date_preset": "last_7d"}`.
    /// Credential parameters are refused.
    pub params: Option<BTreeMap<String, String>>,
}

/// Read up to 50 Graph objects or edges in one request. Read-only: every item
/// is a GET. Each item still counts against Meta's rate limits.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchReadInput {
    #[schemars(length(min = 1, max = 50))]
    pub requests: Vec<BatchReadRequest>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BatchItemError {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_subcode: Option<i64>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fbtrace_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BatchReadItem {
    pub index: usize,
    pub endpoint: String,
    /// False when Meta did not finish this item in time; read it again.
    pub completed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    /// Meta's error codes for a failed item. Provider prose is never forwarded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<BatchItemError>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BatchReadResult {
    pub items: Vec<BatchReadItem>,
    pub succeeded: usize,
    pub failed: usize,
    pub incomplete: usize,
}

fn invalid(message: impl Into<String>) -> PublicError {
    PublicError::invalid_input(
        message,
        "Use relative Graph paths, simple field names and plain parameters",
    )
}

fn valid_field(field: &str) -> bool {
    let mut bytes = field.bytes();
    field.len() <= MAX_FIELD_CHARS
        && bytes
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn build_requests(input: &BatchReadInput) -> Result<BatchRequests, PublicError> {
    if input.requests.is_empty() || input.requests.len() > MAX_REQUESTS {
        return Err(invalid(format!(
            "requests must hold 1 through {MAX_REQUESTS} items"
        )));
    }
    input
        .requests
        .iter()
        .map(|request| {
            let endpoint = request.endpoint.trim().trim_matches('/').to_owned();
            if endpoint.is_empty() || endpoint.contains(['?', '&', '{', '}']) {
                return Err(invalid(
                    "endpoint must be a relative Graph path without a query",
                ));
            }
            let mut query = Vec::new();
            if let Some(fields) = &request.fields {
                if fields.is_empty()
                    || fields.len() > MAX_FIELDS
                    || !fields.iter().all(|field| valid_field(field))
                {
                    return Err(invalid("fields must be 1 through 50 simple field names"));
                }
                query.push(("fields".to_owned(), fields.join(",")));
            }
            if let Some(params) = &request.params {
                if params.len() > MAX_PARAMS {
                    return Err(invalid(format!(
                        "params accepts at most {MAX_PARAMS} entries"
                    )));
                }
                for (key, value) in params {
                    if key == "fields" || value.chars().count() > MAX_PARAM_VALUE_CHARS {
                        return Err(invalid(
                            "pass fields through `fields` and keep values under 2,048 characters",
                        ));
                    }
                    query.push((key.clone(), value.clone()));
                }
            }
            Ok((endpoint, query))
        })
        .collect()
}

fn item_error(body: &Value) -> Option<BatchItemError> {
    let error = body.get("error")?;
    Some(BatchItemError {
        code: error.get("code").and_then(Value::as_i64),
        error_subcode: error.get("error_subcode").and_then(Value::as_i64),
        error_type: error
            .get("type")
            .and_then(Value::as_str)
            .filter(|text| {
                text.len() <= 64
                    && text
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            })
            .map(str::to_owned),
        fbtrace_id: error
            .get("fbtrace_id")
            .and_then(Value::as_str)
            .filter(|text| {
                text.len() <= 64
                    && text
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            })
            .map(str::to_owned),
    })
}

fn summarize(endpoints: Vec<String>, items: Vec<Option<BatchItem>>) -> BatchReadResult {
    let mut results = Vec::with_capacity(endpoints.len());
    let (mut succeeded, mut failed, mut incomplete) = (0, 0, 0);
    for (index, endpoint) in endpoints.into_iter().enumerate() {
        match items.get(index).and_then(Option::as_ref) {
            Some(item) => {
                let error = (!(200..300).contains(&item.code)).then(|| {
                    item_error(&item.body).unwrap_or(BatchItemError {
                        code: None,
                        error_subcode: None,
                        error_type: None,
                        fbtrace_id: None,
                    })
                });
                if error.is_some() {
                    failed += 1;
                } else {
                    succeeded += 1;
                }
                results.push(BatchReadItem {
                    index,
                    endpoint,
                    completed: true,
                    status: Some(item.code),
                    data: error.is_none().then(|| item.body.clone()),
                    error,
                });
            }
            None => {
                incomplete += 1;
                results.push(BatchReadItem {
                    index,
                    endpoint,
                    completed: false,
                    status: None,
                    data: None,
                    error: None,
                });
            }
        }
    }
    BatchReadResult {
        items: results,
        succeeded,
        failed,
        incomplete,
    }
}

pub(crate) async fn batch_read(
    graph: &GraphClient,
    input: BatchReadInput,
) -> ToolResponse<BatchReadResult> {
    let requests = match build_requests(&input) {
        Ok(requests) => requests,
        Err(error) => return ToolResponse::error(error),
    };
    let endpoints = requests
        .iter()
        .map(|(endpoint, _)| endpoint.clone())
        .collect();
    match graph.get_batch(&requests).await {
        Ok(items) => ToolResponse::success(summarize(endpoints, items)),
        Err(error) => ToolResponse::error(error),
    }
}

#[tool_router(router = batch_read_router, vis = "pub(crate)")]
impl MetaAdsServer {
    #[tool(
        name = "batch_read",
        description = "Read up to 50 Graph objects or edges in one request (Graph Batch API, GET only). Each item returns its own status and data, or Meta's error codes. Every item still counts against rate limits.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn batch_read(
        &self,
        Parameters(input): Parameters<BatchReadInput>,
    ) -> Result<Json<ToolResponse<BatchReadResult>>, Json<ToolResponse<BatchReadResult>>> {
        batch_read(&self.graph, input).await.into_mcp_result()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::{BatchReadInput, batch_read, build_requests};
    use crate::{config::MetaConfig, error::ToolResponse, graph::GraphClient};

    fn input(value: serde_json::Value) -> BatchReadInput {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn validates_paths_fields_and_parameters() {
        let ok = build_requests(&input(json!({"requests": [
            {"endpoint": "/act_1/campaigns/", "fields": ["id", "name"], "params": {"limit": "5"}},
            {"endpoint": "120"}
        ]})))
        .unwrap();
        assert_eq!(ok[0].0, "act_1/campaigns");
        assert_eq!(
            ok[0].1,
            [
                ("fields".to_owned(), "id,name".to_owned()),
                ("limit".to_owned(), "5".to_owned())
            ]
        );
        for bad in [
            json!({"requests": []}),
            json!({"requests": [{"endpoint": "act_1?fields=id"}]}),
            json!({"requests": [{"endpoint": "1", "fields": ["insights{spend}"]}]}),
            json!({"requests": [{"endpoint": "1", "params": {"fields": "id"}}]}),
        ] {
            assert!(build_requests(&input(bad.clone())).is_err(), "{bad}");
        }
        let too_many = json!({"requests": (0..51).map(|i| json!({"endpoint": i.to_string()})).collect::<Vec<_>>()});
        assert!(build_requests(&input(too_many)).is_err());
    }

    #[tokio::test]
    async fn sends_one_batch_and_maps_each_item_without_provider_prose() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let read = socket.read(&mut buffer).await.unwrap();
                request.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&request);
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let body = r#"[{"code":200,"body":"{\"id\":\"1\",\"name\":\"A\",\"access_token\":\"leak\"}"},{"code":400,"body":"{\"error\":{\"code\":100,\"error_subcode\":33,\"message\":\"private text\",\"type\":\"OAuthException\",\"fbtrace_id\":\"AbC-1\"}}"},null]"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(200), listener.accept())
                    .await
                    .is_err()
            );
            String::from_utf8(request).unwrap()
        });
        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        ))
        .unwrap();
        let response = batch_read(
            &graph,
            input(json!({"requests": [
                {"endpoint": "1", "fields": ["id", "name"]},
                {"endpoint": "2"},
                {"endpoint": "3"}
            ]})),
        )
        .await;
        let ToolResponse::Success { data } = response else {
            panic!("batch should succeed");
        };
        assert_eq!((data.succeeded, data.failed, data.incomplete), (1, 1, 1));
        assert_eq!(data.items[0].data.as_ref().unwrap()["name"], "A");
        assert_ne!(data.items[0].data.as_ref().unwrap()["access_token"], "leak");
        let error = data.items[1].error.as_ref().unwrap();
        assert_eq!((error.code, error.error_subcode), (Some(100), Some(33)));
        assert_eq!(error.fbtrace_id.as_deref(), Some("AbC-1"));
        assert!(
            !serde_json::to_string(&data)
                .unwrap()
                .contains("private text")
        );
        assert!(!data.items[2].completed);

        let request = server.await.unwrap();
        assert!(request.starts_with("POST / HTTP/1.1"));
        assert!(request.contains("include_headers=false"));
        assert!(request.contains("relative_url"));
        assert!(
            !request
                .split("\r\n\r\n")
                .nth(1)
                .unwrap()
                .contains("test-access-token")
        );
    }
}
