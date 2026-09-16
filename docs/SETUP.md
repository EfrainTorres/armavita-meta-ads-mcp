# Setup

The [README](../README.md) covers the quick start. This page covers the extra settings.

## Build and run

You need Rust 1.98.0, a C/C++ compiler, and CMake. From an extracted source package:

```bash
cargo install --locked --path .
armavita-meta-ads-mcp --version
```

The installed command must be on your MCP client's PATH. `run.sh` can also start an already-built
or installed binary.

## Authentication

Set `META_ACCESS_TOKEN` in the environment that launches your MCP client, or use browser login:

1. Set `META_APP_ID` and `META_APP_SECRET` in your environment.
2. Run `armavita-meta-ads-mcp --login` and finish the browser prompts.
3. Restart your MCP client.

An environment token takes priority over a saved login. Browser login uses a local callback on
ports 8080–8089. On Windows, use an environment token; private OAuth caching is supported on
macOS/Linux.

Saved login locations:

- macOS: `~/Library/Application Support/armavita-meta-ads-mcp/token_cache.json`
- Linux: `${XDG_CONFIG_HOME:-$HOME/.config}/armavita-meta-ads-mcp/token_cache.json`

To start over, delete the cache file and log in again. Keep these files private.

### Permissions

The default login requests `ads_management`, `ads_read`, `business_management`, `pages_show_list`,
`pages_read_engagement`, `instagram_basic`, and `threads_business_basic`.

Catalogs also need `catalog_management`. Partnership permission writes need
`instagram_branded_content_ads_brand` and the appropriate ADVERTISER asset role.
To add scopes, set `META_AUTH_SCOPE` to the **full** desired list—it replaces the defaults.
Your token must also have access to the accounts and assets you request.

## Client configuration

### Codex

Add this to `~/.codex/config.toml` or your trusted project's `.codex/config.toml`:

```toml
[mcp_servers.meta-ads-armavita]
command = "armavita-meta-ads-mcp"
args = ["--transport=stdio"]
env_vars = ["META_ACCESS_TOKEN", "META_MEDIA_ROOT"]
startup_timeout_sec = 10
tool_timeout_sec = 330
default_tools_approval_mode = "writes"
```

Run `codex mcp list` to check the configuration.

### Claude Code

Use the included [`.mcp.json`](../.mcp.json). Launch Claude with `MCP_TOOL_TIMEOUT=330000` to allow
long-running calls. Pass credentials through the environment, not as values saved in JSON.

## Media uploads

Set `META_MEDIA_ROOT` to an existing absolute directory you control. Pass tool paths relative to
that folder. Uploads stay disabled if this setting is absent.

- JPEG/PNG images: up to 15 MiB.
- MP4/MOV videos: up to 512 MiB.
- Larger videos: use a public HTTPS `file_url` without credentials, which Meta downloads.

Keep write approvals enabled and read the [security guidance](../SECURITY.md) before changing live ads.
