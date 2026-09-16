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
Other workflows may need these additional permissions:

| Workflow | Permissions |
| --- | --- |
| Instant Forms and submissions | `pages_manage_ads`, `leads_retrieval`, and Page lead access |
| Lead webhook subscriptions | `pages_manage_metadata` |
| Page media uploads and form cover photos | `pages_manage_posts` |
| Facebook ad comments | `pages_read_user_content`, `pages_manage_engagement` |
| Instagram ad comments | `instagram_manage_comments` |

To add scopes, set `META_AUTH_SCOPE` to the **full** desired list—it replaces the defaults.
Your token must also have access to the accounts and assets you request.
Page tokens are resolved internally and never returned to your assistant.

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
- Lead form documents: PDF/JPEG/PNG up to 20 MiB.
- Catalog feeds: UTF-8 CSV/TSV/XML/JSON up to 64 MiB.
- Playable archives: ZIP up to 5 MiB.

Use Page media uploads when an Instant Experience needs a photo or video ID. Account image hashes
are different assets. Page uploads remain unpublished.

## A few Meta limits

- Instant Form content cannot be edited after creation. Create a replacement form, or archive the old one.
- Lead webhooks need an existing app callback; this local server does not host a public webhook.
- Threads placements require Instagram Feed alongside them. Keep captions within 1,000 characters
  and source images at least 500 pixels wide.
- Threads reply tools support direct text replies and hide/unhide on eligible ads. They require the
  Threads media ID; nested replies and reply deletion are not available through this ad API.
- Edit Instant Experiences before publishing. To retire a published experience, hide it.
- Saved audiences are available for lookup; creation and editing are not exposed by the current API.

Keep write approvals enabled and read the [security guidance](../SECURITY.md) before changing live ads.
