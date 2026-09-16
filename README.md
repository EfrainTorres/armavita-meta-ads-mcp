# ArmaVita Meta Ads MCP

Manage Facebook, Instagram, and Threads ads from Claude Code, Codex, or another MCP-compatible assistant.

We built this for fast startup, low memory use, and broad advertising coverage in one local server.
**125 tools** cover campaigns, reporting, lead forms, catalogs, audiences, and more, with compact
responses and safeguards for sensitive changes. Related actions share tools to keep the list manageable.

It runs as a single executable with no Python environment to maintain and uses Meta Marketing API v26.

**Rust 2.0 release candidate:** smoke tested and heavily used internally.
Please [report any issues](https://github.com/EfrainTorres/armavita-meta-ads-mcp/issues) you run into.

## What you can do

| Area | Capabilities |
| --- | --- |
| Ads | Campaigns, ad sets, creatives, previews, budgets, schedules, and delivery checks |
| Reporting | Insights, filters, attribution settings, breakdowns, async reports, and change history |
| Leads | Create and archive Instant Forms, retrieve submissions, test leads, and manage webhook subscriptions |
| Audiences | Custom and lookalike audiences, saved-audience lookup, targeting, uploads, and sharing |
| Automation | Rules, A/B studies, value rules, and recommendations |
| Catalogs | Products, sets, feeds, batch updates, diagnostics, and catalog advertising settings |
| Creative assets | Account and Page media, playables, labels, and Instant Experiences |
| Engagement | Facebook/Instagram ad comments and supported Threads ad replies |
| Measurement and access | Pixels, datasets, CAPI, custom conversions, business assets, permissions, and publisher block lists |

Meta permissions and account eligibility still apply. [Setup and limitations](docs/SETUP.md).

## Why we rebuilt it in Rust

We rebuilt our [Python server](https://github.com/EfrainTorres/armavita-meta-ads-mcp/tree/python-legacy) for faster
startup, lower memory use, and a single executable with no Python environment to maintain.

| Metric | Python | Rust | Improvement |
| --- | ---: | ---: | ---: |
| Startup | 512 ms | 8.17 ms | **~63× faster** |
| Idle memory | 77.84 MiB | 16.30 MiB | **~79% less** |
| Runtime disk space | 45.7 MiB | 7.97 MiB | **~83% smaller** |

Original rewrite baseline, measured on macOS arm64 on August 20, 2026. Startup uses warm-run medians; loading tool definitions
was slower in this benchmark. [Full results and methodology](docs/BENCHMARKS.md).

The rebuild also keeps credentials out of tool calls and adds write plans you can review before
applying. [See what changed from Python](docs/MIGRATION.md).

## Get started

### 1. Install

Source builds require Rust 1.98.0, a C/C++ compiler, and CMake. From a checkout of the `rust` branch
or an extracted [source release](https://github.com/EfrainTorres/armavita-meta-ads-mcp/releases/tag/v2.0.0-rc.2):

```bash
cargo install --locked --path .
```

### 2. Authenticate

Set `META_ACCESS_TOKEN` in the environment used to launch your MCP client. Your token needs the
appropriate Meta permissions and access to the ad accounts you want to manage.

For browser login instead, set `META_APP_ID` and `META_APP_SECRET`, then run:

```bash
armavita-meta-ads-mcp --login
```

Browser login saves a private local token cache on macOS/Linux. On Windows, use `META_ACCESS_TOKEN`.
Never put credentials in prompts or committed configuration files.

### 3. Connect your assistant

For clients that use JSON MCP configuration, add:

```json
{
  "mcpServers": {
    "meta-ads-armavita": {
      "command": "armavita-meta-ads-mcp",
      "args": ["--transport=stdio"]
    }
  }
}
```

Claude Code can use the included [`.mcp.json`](.mcp.json). For Codex, use the
[TOML configuration example](docs/SETUP.md#client-configuration).

[Full setup guide](docs/SETUP.md): permissions, authentication, client settings, and media uploads.

## Before changing live ads

Keep write approvals enabled in your MCP client. Review mutation plans before applying them;
deletes require an explicit acknowledgement. The server never automatically retries a write.

Local uploads are off by default. Set `META_MEDIA_ROOT` to a folder you control to enable them.
See [security guidance](SECURITY.md) for details.

Coming from the Python version? Some tool names, inputs, and results have changed.
Read the [migration guide](docs/MIGRATION.md) before switching.

## License

Copyright (c) ArmaVita LLC. [GNU AGPLv3 only](LICENSE) (`AGPL-3.0-only`). Distributed without warranty.
