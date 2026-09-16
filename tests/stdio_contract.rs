use std::{fs, path::Path, process::Stdio};

use rmcp::{
    ClientHandler, ServiceExt,
    model::{
        CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation, ProtocolVersion,
        RequestMetaObject, ResultType,
    },
    transport::{ConfigureCommandExt, TokioChildProcess},
};

fn transport(empty_home: &Path) -> TokioChildProcess {
    TokioChildProcess::new(
        tokio::process::Command::new(env!("CARGO_BIN_EXE_armavita-meta-ads-mcp")).configure(
            |command| {
                command
                    .arg("--transport=stdio")
                    .current_dir(empty_home)
                    .env_clear()
                    .env("HOME", empty_home)
                    .env("XDG_CONFIG_HOME", empty_home)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null());
            },
        ),
    )
    .unwrap()
}

#[test]
fn invalid_cli_arguments_are_not_echoed_into_diagnostics() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_armavita-meta-ads-mcp"))
        .env_clear()
        .arg("--access-token=synthetic-sensitive-value")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let diagnostics = String::from_utf8(output.stderr).unwrap();
    assert!(diagnostics.contains("unknown argument"));
    assert!(!diagnostics.contains("synthetic-sensitive-value"));
}

#[derive(Clone)]
struct CurrentClient;

impl ClientHandler for CurrentClient {
    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        info.protocol_version = ProtocolVersion::V_2026_07_28;
        info
    }
}

#[tokio::test]
async fn binary_negotiates_lists_tools_and_returns_structured_errors() {
    let empty_home = std::env::temp_dir().join(format!(
        "armavita-meta-ads-mcp-current-stdio-{}",
        std::process::id()
    ));
    fs::create_dir_all(&empty_home).unwrap();
    let client = ().serve(transport(&empty_home)).await.unwrap();

    let tools = client.list_all_tools().await.unwrap();
    let names = tools
        .iter()
        .map(|tool| tool.name.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(names.len(), 77);
    assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
    for required in [
        "apply_mutation_plan",
        "build_mutation_plan",
        "create_ad",
        "create_ad_creative",
        "create_ad_set",
        "grant_branded_content_ad_permission",
        "revoke_branded_content_ad_permission",
        "create_reach_frequency_prediction",
        "create_threads_account",
        "discard_mutation_plan",
        "get_mutation_plan",
        "upload_ad_image_asset",
        "upload_ad_video_asset",
        "batch_products",
        "upsert_product",
        "send_capi_events",
        "read_dataset_quality",
    ] {
        assert!(names.contains(&required));
    }
    assert!(names.iter().all(|name| !name.contains("whatsapp")));

    let result = client
        .call_tool(CallToolRequestParams::new("list_ad_accounts"))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["status"], "error");
    assert_eq!(structured["error"]["code"], "AUTH_REQUIRED");

    client.cancel().await.unwrap();
    fs::remove_dir_all(empty_home).unwrap();
}

#[tokio::test]
async fn binary_supports_current_discovery_and_result_shape() {
    let empty_home = std::env::temp_dir().join(format!(
        "armavita-meta-ads-mcp-discovery-stdio-{}",
        std::process::id()
    ));
    fs::create_dir_all(&empty_home).unwrap();
    let client = CurrentClient.serve(transport(&empty_home)).await.unwrap();
    let mut metadata = RequestMetaObject::new();
    metadata.set_protocol_version(ProtocolVersion::V_2026_07_28);
    metadata.set_client_info(Implementation::new("contract-test", "0.1.0"));
    metadata.set_client_capabilities(ClientCapabilities::default());
    let discovery = client.discover(metadata).await.unwrap();
    let server_info = discovery.server_info().unwrap();
    assert_eq!(server_info.name, "armavita-meta-ads-mcp");

    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 77);

    let result = client
        .call_tool(CallToolRequestParams::new("list_ad_accounts"))
        .await
        .unwrap();
    assert_eq!(result.result_type, Some(ResultType::COMPLETE));
    assert_eq!(result.is_error, Some(true));

    client.cancel().await.unwrap();
    fs::remove_dir_all(empty_home).unwrap();
}
