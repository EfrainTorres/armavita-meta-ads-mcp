// SPDX-License-Identifier: AGPL-3.0-only

//! `export_insights_job`: write a finished async Insights report to a CSV file
//! under `META_MEDIA_ROOT/reports/` instead of returning every row to the model.

use std::path::{Path, PathBuf};

use rmcp::{
    Json,
    handler::server::wrapper::Parameters,
    schemars::{self, JsonSchema},
    tool, tool_router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::numeric_owned,
    server::MetaAdsServer,
};

const REPORTS_DIR: &str = "reports";
const PAGE_SIZE: &str = "500";
const DEFAULT_MAX_ROWS: u32 = 20_000;
const MAX_ROWS: u32 = 100_000;
const MAX_PAGES: usize = 400;
const MAX_FILE_NAME_CHARS: usize = 100;
const MAX_CELL_CHARS: usize = 32_000;
const MAX_CURSOR_CHARS: usize = 2_048;
const JOB_COMPLETED: &str = "Job Completed";

/// Export a completed async Insights job to CSV. The rows are written to a file
/// and only a summary is returned, so large reports do not fill the context.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportInsightsJobInput {
    /// Numeric async report-run ID from `create_insights_job`.
    pub report_run_id: String,
    /// File name inside `META_MEDIA_ROOT/reports/`, ending in `.csv`.
    /// Letters, digits, `.`, `_` and `-` only. Defaults to `insights-<id>.csv`.
    #[schemars(length(min = 5, max = 100))]
    pub file_name: Option<String>,
    /// Stop after this many rows (default 20,000, at most 100,000).
    #[schemars(range(min = 1, max = 100_000))]
    pub max_rows: Option<u32>,
    /// Replace an existing file with the same name. Defaults to false.
    pub overwrite: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ExportedInsights {
    /// Path relative to `META_MEDIA_ROOT`.
    pub relative_path: String,
    pub rows: u32,
    pub columns: Vec<String>,
    /// True when `max_rows` stopped the export before Meta's last page.
    pub truncated: bool,
    /// Fields present only after the first page; their values were not written.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dropped_fields: Vec<String>,
}

fn file_name(input: &ExportInsightsJobInput, run_id: &str) -> Result<String, PublicError> {
    let name = input.file_name.as_deref().map_or_else(
        || format!("insights-{run_id}.csv"),
        |name| name.trim().to_owned(),
    );
    let valid = name.len() <= MAX_FILE_NAME_CHARS
        && name.ends_with(".csv")
        && name.len() > ".csv".len()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if !valid {
        return Err(PublicError::invalid_input(
            "file_name must be a plain .csv name of letters, digits, '.', '_' or '-'",
            "Use a name such as `campaigns-2026-11.csv`",
        ));
    }
    Ok(name)
}

/// One CSV cell. Text that a spreadsheet would run as a formula is prefixed with
/// an apostrophe, and every cell is quoted when it needs to be.
fn csv_cell(value: Option<&Value>) -> String {
    let text = match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(Value::Number(number)) => number.to_string(),
        Some(other) => other.to_string(),
    };
    let mut text: String = text.chars().take(MAX_CELL_CHARS).collect();
    let numeric = matches!(value, Some(Value::Number(_)));
    if !numeric && text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        text.insert(0, '\'');
    }
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

fn csv_line(cells: impl Iterator<Item = String>) -> String {
    let mut line = cells.collect::<Vec<_>>().join(",");
    line.push_str("\r\n");
    line
}

fn columns_of(rows: &[Value]) -> Vec<String> {
    let mut columns = Vec::new();
    for row in rows {
        if let Some(object) = row.as_object() {
            for key in object.keys() {
                if !columns.iter().any(|column| column == key) {
                    columns.push(key.clone());
                }
            }
        }
    }
    columns
}

/// Resolve `META_MEDIA_ROOT/reports`, creating it, and refuse anything that
/// escapes the media root (for example a `reports` symlink).
async fn reports_dir(media_root: Option<&Path>) -> Result<PathBuf, PublicError> {
    let root = media_root.ok_or_else(|| {
        PublicError::invalid_input(
            "Report export needs META_MEDIA_ROOT",
            "Set META_MEDIA_ROOT to the directory where files may be written",
        )
    })?;
    let directory = root.join(REPORTS_DIR);
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|_| unwritable())?;
    let canonical = tokio::fs::canonicalize(&directory)
        .await
        .map_err(|_| unwritable())?;
    if !canonical.starts_with(root) || !canonical.is_dir() {
        return Err(PublicError::invalid_input(
            "The reports directory resolves outside META_MEDIA_ROOT",
            "Make META_MEDIA_ROOT/reports a plain directory",
        ));
    }
    Ok(canonical)
}

fn unwritable() -> PublicError {
    PublicError::invalid_input(
        "The report file could not be written under META_MEDIA_ROOT/reports",
        "Check that the media root exists and is writable",
    )
}

fn page_rows(payload: &Value) -> Result<(Vec<Value>, Option<String>), PublicError> {
    let rows = payload
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| {
            PublicError::invalid_upstream("Meta returned an unexpected Insights page")
        })?;
    let next = payload
        .pointer("/paging/next")
        .is_some()
        .then(|| {
            payload
                .pointer("/paging/cursors/after")
                .and_then(Value::as_str)
        })
        .flatten()
        .filter(|cursor| !cursor.is_empty() && cursor.len() <= MAX_CURSOR_CHARS)
        .map(str::to_owned);
    Ok((rows, next))
}

pub(crate) async fn export_insights_job(
    graph: &GraphClient,
    media_root: Option<&Path>,
    input: ExportInsightsJobInput,
) -> ToolResponse<ExportedInsights> {
    match export(graph, media_root, input).await {
        Ok(result) => ToolResponse::success(result),
        Err(error) => ToolResponse::error(error),
    }
}

async fn export(
    graph: &GraphClient,
    media_root: Option<&Path>,
    input: ExportInsightsJobInput,
) -> Result<ExportedInsights, PublicError> {
    let run_id = numeric_owned(&input.report_run_id).ok_or_else(|| {
        PublicError::invalid_input(
            "report_run_id must be a numeric async report-run ID",
            "Use the ID returned by create_insights_job",
        )
    })?;
    let name = file_name(&input, &run_id)?;
    let max_rows = input
        .max_rows
        .unwrap_or(DEFAULT_MAX_ROWS)
        .clamp(1, MAX_ROWS);
    let directory = reports_dir(media_root).await?;
    let destination = directory.join(&name);
    if !input.overwrite.unwrap_or(false)
        && tokio::fs::try_exists(&destination).await.unwrap_or(true)
    {
        return Err(PublicError::invalid_input(
            format!("reports/{name} already exists"),
            "Choose another file_name or pass overwrite=true",
        ));
    }

    let job = graph
        .get_json(&run_id, &[("fields".to_owned(), "async_status".to_owned())])
        .await?;
    if job.get("async_status").and_then(Value::as_str) != Some(JOB_COMPLETED) {
        return Err(PublicError::invalid_input(
            "The async Insights job has not completed",
            "Poll read_insights_job until async_status is `Job Completed`",
        ));
    }

    let endpoint = format!("{run_id}/insights");
    let temporary = directory.join(format!(".{name}.partial"));
    let mut file = tokio::fs::File::create(&temporary)
        .await
        .map_err(|_| unwritable())?;
    let outcome = async {
        let mut columns: Vec<String> = Vec::new();
        let mut dropped: Vec<String> = Vec::new();
        let mut written = 0_u32;
        let mut cursor: Option<String> = None;
        let mut truncated = false;
        for page in 0..MAX_PAGES {
            let mut query = vec![("limit".to_owned(), PAGE_SIZE.to_owned())];
            if let Some(after) = &cursor {
                query.push(("after".to_owned(), after.clone()));
            }
            let payload = graph.get_json(&endpoint, &query).await?;
            let (rows, next) = page_rows(&payload)?;
            if page == 0 {
                columns = columns_of(&rows);
                let header = csv_line(
                    columns
                        .iter()
                        .map(|column| csv_cell(Some(&Value::String(column.clone())))),
                );
                file.write_all(header.as_bytes())
                    .await
                    .map_err(|_| unwritable())?;
            } else {
                for key in columns_of(&rows) {
                    if !columns.contains(&key) && !dropped.contains(&key) {
                        dropped.push(key);
                    }
                }
            }
            for row in &rows {
                if written >= max_rows {
                    truncated = true;
                    break;
                }
                let line = csv_line(columns.iter().map(|column| csv_cell(row.get(column))));
                file.write_all(line.as_bytes())
                    .await
                    .map_err(|_| unwritable())?;
                written += 1;
            }
            if truncated {
                break;
            }
            match next {
                Some(after) => cursor = Some(after),
                None => break,
            }
            if page + 1 == MAX_PAGES {
                truncated = true;
            }
        }
        file.flush().await.map_err(|_| unwritable())?;
        file.sync_all().await.map_err(|_| unwritable())?;
        Ok::<_, PublicError>((columns, dropped, written, truncated))
    }
    .await;
    drop(file);
    let (columns, dropped_fields, rows, truncated) = match outcome {
        Ok(result) => result,
        Err(error) => {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(error);
        }
    };
    if tokio::fs::rename(&temporary, &destination).await.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(unwritable());
    }
    Ok(ExportedInsights {
        relative_path: format!("{REPORTS_DIR}/{name}"),
        rows,
        columns,
        truncated,
        dropped_fields,
    })
}

#[tool_router(router = insights_export_router, vis = "pub(crate)")]
impl MetaAdsServer {
    #[tool(
        name = "export_insights_job",
        description = "Write the rows of a completed async Insights job to a CSV file under META_MEDIA_ROOT/reports/ and return only the path, row count and columns. Use for large reports instead of paging read_insights_job_results.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn export_insights_job(
        &self,
        Parameters(input): Parameters<ExportInsightsJobInput>,
    ) -> Result<Json<ToolResponse<ExportedInsights>>, Json<ToolResponse<ExportedInsights>>> {
        export_insights_job(&self.graph, self.media_root.as_deref(), input)
            .await
            .into_mcp_result()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{ExportInsightsJobInput, columns_of, csv_cell, file_name, page_rows};

    fn input(name: Option<&str>) -> ExportInsightsJobInput {
        ExportInsightsJobInput {
            report_run_id: "42".into(),
            file_name: name.map(str::to_owned),
            max_rows: None,
            overwrite: None,
        }
    }

    #[test]
    fn cells_are_quoted_and_formula_text_is_neutralised() {
        assert_eq!(csv_cell(Some(&json!("plain"))), "plain");
        assert_eq!(csv_cell(Some(&json!("a,b"))), "\"a,b\"");
        assert_eq!(csv_cell(Some(&json!("say \"hi\""))), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_cell(Some(&json!("=HYPERLINK(1)"))), "'=HYPERLINK(1)");
        assert_eq!(csv_cell(Some(&json!("-5 off"))), "'-5 off");
        assert_eq!(csv_cell(Some(&json!(-5))), "-5");
        assert_eq!(csv_cell(Some(&Value::Null)), "");
        assert_eq!(csv_cell(None), "");
        assert_eq!(
            csv_cell(Some(&json!([{"action_type": "lead", "value": "3"}]))),
            "\"[{\"\"action_type\"\":\"\"lead\"\",\"\"value\"\":\"\"3\"\"}]\""
        );
    }

    #[test]
    fn file_names_stay_inside_the_reports_directory() {
        assert_eq!(file_name(&input(None), "42").unwrap(), "insights-42.csv");
        assert_eq!(
            file_name(&input(Some(" nov-2026.csv ")), "42").unwrap(),
            "nov-2026.csv"
        );
        for bad in [
            "../x.csv",
            "a/b.csv",
            ".hidden.csv",
            "report.txt",
            ".csv",
            "a b.csv",
        ] {
            assert!(file_name(&input(Some(bad)), "42").is_err(), "{bad}");
        }
    }

    #[test]
    fn pages_keep_first_seen_column_order_and_follow_cursors() {
        let payload = json!({
            "data": [{"campaign_id": "1", "spend": "10"}, {"campaign_id": "2", "clicks": "3"}],
            "paging": {"cursors": {"after": "abc"}, "next": "https://graph/next"}
        });
        let (rows, next) = page_rows(&payload).unwrap();
        assert_eq!(columns_of(&rows), ["campaign_id", "spend", "clicks"]);
        assert_eq!(next.as_deref(), Some("abc"));
        let (_, last) =
            page_rows(&json!({"data": [], "paging": {"cursors": {"after": "x"}}})).unwrap();
        assert_eq!(last, None);
        assert!(page_rows(&json!({"error": {}})).is_err());
    }
}

#[cfg(test)]
mod http_tests {
    use std::time::Duration;

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::{ExportInsightsJobInput, export};
    use crate::{config::MetaConfig, graph::GraphClient};

    async fn scripted(
        responses: Vec<&'static str>,
    ) -> (GraphClient, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut lines = Vec::new();
            for body in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = vec![0_u8; 8192];
                let read = socket.read(&mut buffer).await.unwrap();
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                lines.push(request.lines().next().unwrap_or_default().to_owned());
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(200), listener.accept())
                    .await
                    .is_err()
            );
            lines
        });
        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        ))
        .unwrap();
        (graph, server)
    }

    fn media_root(tag: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("armavita-export-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::canonicalize(root).unwrap()
    }

    fn input(overwrite: Option<bool>) -> ExportInsightsJobInput {
        ExportInsightsJobInput {
            report_run_id: "42".into(),
            file_name: Some("nov.csv".into()),
            max_rows: None,
            overwrite,
        }
    }

    #[tokio::test]
    async fn writes_every_page_to_a_csv_and_protects_existing_files() {
        let root = media_root("pages");
        let (graph, server) = scripted(vec![
            r#"{"async_status":"Job Completed","id":"42"}"#,
            r#"{"data":[{"campaign_name":"=evil","spend":"10.5"}],"paging":{"cursors":{"after":"c1"},"next":"https://next"}}"#,
            r#"{"data":[{"campaign_name":"B, Inc","spend":"2"}],"paging":{"cursors":{"before":"c1"}}}"#,
        ])
        .await;
        let result = export(&graph, Some(&root), input(None)).await.unwrap();
        assert_eq!(result.relative_path, "reports/nov.csv");
        assert_eq!(result.rows, 2);
        assert!(!result.truncated);
        let csv = std::fs::read_to_string(root.join("reports/nov.csv")).unwrap();
        assert_eq!(
            csv,
            "campaign_name,spend\r\n'=evil,10.5\r\n\"B, Inc\",2\r\n"
        );
        let lines = server.await.unwrap();
        assert!(lines[0].starts_with("GET /42?fields=async_status "));
        assert!(lines[1].starts_with("GET /42/insights?limit=500 "));
        assert!(lines[2].starts_with("GET /42/insights?limit=500&after=c1 "));

        // A second export to the same name is refused before any request.
        let (graph, server) = scripted(vec![]).await;
        assert!(export(&graph, Some(&root), input(None)).await.is_err());
        server.await.unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn refuses_unfinished_jobs_and_missing_media_root() {
        let root = media_root("pending");
        let (graph, server) = scripted(vec![r#"{"async_status":"Job Running"}"#]).await;
        let error = export(&graph, Some(&root), input(Some(true)))
            .await
            .unwrap_err();
        assert!(error.message.contains("not completed"));
        assert!(!root.join("reports/nov.csv").exists());
        server.await.unwrap();

        let (graph, server) = scripted(vec![]).await;
        assert!(export(&graph, None, input(None)).await.is_err());
        server.await.unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
