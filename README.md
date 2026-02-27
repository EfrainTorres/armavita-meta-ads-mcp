# armavita-meta-ads-mcp

<p align="center">
  <img src="docs/assets/armavita-meta-ads-mcp-hero-1080.jpg" alt="ArmaVita Meta Ads MCP hero image" width="100%" />
</p>

<p align="center"><strong>Brought to you by <a href="https://armavita.com">ArmaVita.com</a></strong></p>

`armavita-meta-ads-mcp` is a local Model Context Protocol server for Meta Ads.
It is built for local MCP clients (Claude Code, Cursor, Codex) and supports:

- Meta access token auth (`META_ACCESS_TOKEN`)
- Local OAuth flow (`META_APP_ID` + `META_APP_SECRET`)
- stdio MCP transport only
- Meta Marketing API `v25.0` by default (`META_GRAPH_API_VERSION` override supported)

Current contract version: `1.0.0`.

## Install

From PyPI (once published):

```bash
pip install armavita-meta-ads-mcp
```

From source (recommended during development):

```bash
uv sync
```

## Run

```bash
armavita-meta-ads-mcp
```

Module entrypoint:

```bash
python -m armavita_meta_ads_mcp
```

Login flow:

```bash
armavita-meta-ads-mcp --login
```

## Tool Coverage

- Accounts: `list_ad_accounts`, `read_ad_account`
- Campaigns: `list_campaigns`, `read_campaign`, `create_campaign`, `update_campaign`
- Budget schedules: `create_campaign_budget_schedule`
- Ad sets: `list_ad_sets`, `read_ad_set`, `create_ad_set`, `update_ad_set`
- Ads/creatives/media: `list_ads`, `read_ad`, `list_ad_previews`, `create_ad`, `update_ad`, `list_ad_creatives`, `read_ad_creative`, `create_ad_creative`, `update_ad_creative`, `upload_ad_image_asset`, `read_ad_image`, `export_ad_image_file`, `search_pages`, `list_account_pages`
- Insights/reporting: `list_insights`, `create_report`
- Targeting: `search_interests`, `suggest_interests`, `estimate_audience_size`, `search_behaviors`, `search_demographics`, `search_geo_locations`
- Duplication: `clone_campaign`, `clone_ad_set`, `clone_ad`, `clone_ad_creative`
- Ads Library: `search_ads_archive`
- Research helpers: `search_web_content`, `read_web_content`

## Pagination

Cursor-based pagination is supported on core list/read streams:

- `list_ad_accounts`, `list_campaigns`, `list_ad_sets`, `list_ads`, `list_insights`
- `list_ad_creatives`, `search_interests`, `suggest_interests`, `search_behaviors`, `search_demographics`, `search_geo_locations`, `search_ads_archive`
- Use `page_cursor` with the `paging.cursors.after` value from the previous response.
- Responses preserve Meta's native `paging` object.

## Insights Query Notes

- `list_insights` and `create_report` support either:
  - `date_range` as `{ "since": "YYYY-MM-DD", "until": "YYYY-MM-DD" }`, or
  - `date_range` as a preset (for example `last_30d`, `maximum`).
- `create_report.comparison_period` uses the same format and validation as `date_range`.
- `previous_30d` is normalized to `last_30d`.
- For action metrics, use `action_breakdowns` (and optional `summary_action_breakdowns`) instead of mixing action keys into `breakdowns`.

## Security

- Access tokens are redacted from URL fields returned by the server (including nested `paging.next` URLs).

## Docs

- [Authentication Setup Guide](docs/auth-setup.md)
- [Local Client Guide](docs/local-client-guide.md)
- [Security Policy](SECURITY.md)

## Scope

- This repository is an OSS local MCP server.
- Transport mode is local `stdio` only.
- Tool aliases are intentionally not exposed.

## License

GNU Affero General Public License v3.0 (AGPLv3). See `LICENSE`.
