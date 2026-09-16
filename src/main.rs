use std::process::ExitCode;

use armavita_meta_ads_mcp::{MetaAdsServer, MetaConfig, login};
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::fmt::writer::MakeWriterExt;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_target(false)
        .with_writer(std::io::stderr.with_max_level(tracing::Level::INFO))
        .init();

    match run().await {
        Ok(code) => code,
        Err(error) => {
            tracing::error!(%error, "server stopped");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut login_requested = false;
    let mut app_id = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--version" | "-V" => {
                println!("armavita-meta-ads-mcp {}", env!("CARGO_PKG_VERSION"));
                return Ok(ExitCode::SUCCESS);
            }
            "--help" | "-h" => {
                print_help();
                return Ok(ExitCode::SUCCESS);
            }
            "--login" => login_requested = true,
            "--app-id" => {
                app_id = Some(args.next().ok_or("--app-id requires a value")?);
            }
            value if value.starts_with("--app-id=") => {
                app_id = Some(value.trim_start_matches("--app-id=").to_owned());
            }
            "--transport" => {
                let transport = args.next().ok_or("--transport requires a value")?;
                if transport != "stdio" {
                    return Err("this OSS build supports local stdio only".into());
                }
            }
            value if value.starts_with("--transport=") => {
                if value.trim_start_matches("--transport=") != "stdio" {
                    return Err("this OSS build supports local stdio only".into());
                }
            }
            _ => return Err("unknown argument; use --help for supported options".into()),
        }
    }

    if login_requested {
        let outcome = login(app_id.as_deref()).await?;
        println!(
            "Meta authentication complete (long-lived token: {}).",
            outcome.used_long_lived_token
        );
        return Ok(ExitCode::SUCCESS);
    }
    if app_id.is_some() {
        return Err("--app-id is only valid with --login".into());
    }

    let config = MetaConfig::from_env()?;
    let server = MetaAdsServer::new(config)?;
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(ExitCode::SUCCESS)
}

fn print_help() {
    println!(
        "armavita-meta-ads-mcp {}\n\nUSAGE:\n    armavita-meta-ads-mcp [--transport=stdio]\n    armavita-meta-ads-mcp --login [--app-id ID]\n\nOPTIONS:\n    --transport stdio   Run the local MCP server (default)\n    --login             Complete Meta OAuth and cache the token locally\n    --app-id ID         Override META_APP_ID during login\n    -h, --help          Print help\n    -V, --version       Print version",
        env!("CARGO_PKG_VERSION")
    );
}
