//! Read-only smoke test against the live Meta Graph API.
//!
//! Ignored by default. Run it locally with a real token:
//!
//! ```sh
//! META_ACCESS_TOKEN=... cargo test --locked --test live_smoke -- --ignored --nocapture
//! ```
//!
//! Optional: `META_SMOKE_AD_ACCOUNT_ID=act_123` picks the account; otherwise the first
//! listed account is used. The test only calls read tools and prints no response bodies.

// Test helpers outside #[test] functions unwrap and panic freely; failures fail the test.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{fs, path::Path, process::Stdio};

use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, JsonObject},
    service::{RoleClient, RunningService},
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde_json::{Value, json};

fn transport(home: &Path, token: &str) -> TokioChildProcess {
    TokioChildProcess::new(
        tokio::process::Command::new(env!("CARGO_BIN_EXE_armavita-meta-ads-mcp")).configure(
            |command| {
                command
                    .arg("--transport=stdio")
                    .current_dir(home)
                    .env_clear()
                    .env("HOME", home)
                    .env("XDG_CONFIG_HOME", home)
                    .env("META_ACCESS_TOKEN", token)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null());
            },
        ),
    )
    .unwrap()
}

async fn call(client: &RunningService<RoleClient, ()>, tool: &'static str, args: Value) -> Value {
    let arguments: JsonObject = serde_json::from_value(args).unwrap();
    let result = client
        .call_tool(CallToolRequestParams::new(tool).with_arguments(arguments))
        .await
        .unwrap_or_else(|_| panic!("{tool}: transport failure"));
    let structured = result
        .structured_content
        .unwrap_or_else(|| panic!("{tool}: no structured content"));
    assert_eq!(
        structured["status"], "success",
        "{tool} failed with error code {}",
        structured["error"]["code"]
    );
    eprintln!("ok  {tool}");
    structured["data"].clone()
}

#[tokio::test]
#[ignore = "needs META_ACCESS_TOKEN and network access"]
async fn live_read_only_smoke() {
    let token = std::env::var("META_ACCESS_TOKEN").expect("set META_ACCESS_TOKEN");
    let home = std::env::temp_dir().join(format!("armavita-live-smoke-{}", std::process::id()));
    fs::create_dir_all(&home).unwrap();
    let client = ().serve(transport(&home, &token)).await.unwrap();

    let accounts = call(&client, "list_ad_accounts", json!({ "page_size": 5 })).await;
    let account_id = match std::env::var("META_SMOKE_AD_ACCOUNT_ID") {
        Ok(id) => id,
        Err(_) => accounts["accounts"][0]["id"]
            .as_str()
            .expect("the token sees no ad account")
            .to_owned(),
    };

    call(
        &client,
        "read_ad_account",
        json!({ "ad_account_id": account_id }),
    )
    .await;
    call(
        &client,
        "list_campaigns",
        json!({ "ad_account_id": account_id, "page_size": 5 }),
    )
    .await;
    call(
        &client,
        "list_insights",
        json!({ "object_id": account_id, "level": "account", "date_preset": "last_7d" }),
    )
    .await;

    client.cancel().await.unwrap();
    fs::remove_dir_all(home).unwrap();
}
