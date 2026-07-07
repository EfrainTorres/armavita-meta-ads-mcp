# Meta, Instagram, Facebook Ads MCP

<p align="center">
  <img src="docs/assets/armavita-meta-ads-mcp-hero-1080.jpg" alt="ArmaVita Meta Ads MCP hero image" width="100%" />
</p>

<p align="center"><strong>Brought to you by <a href="https://armavita.com">ArmaVita.com</a></strong></p>
<p align="center">Need a custom implementation? <a href="https://armavita.com">Contact us</a>.</p>

`armavita-meta-ads-mcp` is a local Model Context Protocol server for Meta Ads.
It is built for local MCP clients (Claude Code, Cursor, Codex) and supports:

- Meta access token auth (`META_ACCESS_TOKEN`)
- Local OAuth flow (`META_APP_ID` + `META_APP_SECRET`)
- stdio MCP transport only
- Python `3.11+`
- `mcp[cli]==1.27.2`
- Meta Marketing API `v25.0` by default (`META_GRAPH_API_VERSION` override supported)

Current contract version: `1.2.0` (**78 tools**).

The default OAuth scope (`META_AUTH_SCOPE`) requests `ads_management`, `ads_read`, `business_management`, `public_profile`, `pages_show_list`, `pages_read_engagement`, `instagram_basic`, and `threads_business_basic`. The catalog tools additionally require `catalog_management` — add it to `META_AUTH_SCOPE` if you use them.

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

## Quick MCP Client Config

Minimal MCP server registration (JSON format used by many clients):

```json
{
  "mcpServers": {
    "meta-ads-armavita": {
      "command": "armavita-meta-ads-mcp",
      "env": {
        "META_ACCESS_TOKEN": "EA...",
        "META_GRAPH_API_VERSION": "v25.0"
      }
    }
  }
}
```

OAuth mode (no direct token in config):

```json
{
  "mcpServers": {
    "meta-ads-armavita": {
      "command": "armavita-meta-ads-mcp",
      "env": {
        "META_APP_ID": "YOUR_APP_ID",
        "META_APP_SECRET": "YOUR_APP_SECRET"
      }
    }
  }
}
```

Then run once to complete login:

```bash
armavita-meta-ads-mcp --login
```

## Advanced Environment Variables

Beyond the core auth variables above, the server reads four optional knobs:

| Variable | Default | Effect |
| --- | --- | --- |
| `META_LOGIN_CONFIG_ID` | unset | Facebook Login for Business configuration ID appended to the OAuth authorization URL. |
| `META_ADS_DISABLE_CALLBACK_SERVER` | unset | Set to any value to disable the local OAuth callback server (token/OAuth login flows will error instead of opening a port). |
| `META_MCP_DR_CAMPAIGN_SCAN_LIMIT` | `10` | Number of ad accounts whose campaigns are scanned by `search_web_content` research queries. |
| `META_MCP_DISABLE_DELIVERY_FALLBACK` | `1` | Set to `0` to let `estimate_audience_size` fall back to the `delivery_estimate` edge when `reachestimate` returns an error. |

## Tool Coverage (78 tools)

### Accounts & controls
- `list_ad_accounts`, `read_ad_account`, `get_account_controls`, `update_account_controls`, `list_account_pages`, `search_pages`

### Campaigns
- `list_campaigns`, `read_campaign`, `create_campaign`, `update_campaign`, `create_campaign_budget_schedule`

### Ad sets
- `list_ad_sets`, `read_ad_set`, `create_ad_set`, `update_ad_set`

### Ads, creatives & media
- `list_ads`, `read_ad`, `create_ad`, `update_ad`, `list_ad_previews`
- `list_ad_creatives`, `read_ad_creative`, `create_ad_creative`, `update_ad_creative`
- `upload_ad_image_asset`, `upload_ad_video_asset`, `read_ad_image`, `export_ad_image_file`
- `list_ad_images`, `list_ad_videos`

### Insights & reporting
- `list_insights`, `create_report`
- Async insights: `create_insights_job`, `read_insights_job`, `read_insights_job_results`
- `list_ad_custom_derived_metrics`

### Custom conversions
- `list_custom_conversions`, `read_custom_conversion`, `create_custom_conversion`, `update_custom_conversion`, `delete_custom_conversion`

### Targeting research
- `search_interests`, `suggest_interests`, `estimate_audience_size`, `search_behaviors`, `search_demographics`, `search_geo_locations`

### Audiences
- `list_custom_audiences`, `read_custom_audience`, `create_custom_audience`, `update_custom_audience`, `delete_custom_audience`
- `create_lookalike_audience`, `manage_custom_audience_users`

### Catalogs & products
- `list_product_catalogs`, `list_product_sets`, `list_products`, `upsert_product`, `batch_products`

### Duplication
- `clone_campaign`, `clone_ad_set`, `clone_ad`, `clone_ad_creative`

### Ads Library & research helpers
- `search_ads_archive`
- `search_web_content`, `read_web_content`

### Conversions API (CAPI)
- `send_capi_events`, `list_business_datasets`, `read_dataset_quality`

### Partnership & branded content
- `list_branded_content_ad_permissions`, `grant_branded_content_ad_permission`

### Reach & frequency
- `list_reach_frequency_predictions`, `create_reach_frequency_prediction`, `read_reach_frequency_prediction`

### Recommendations
- `list_recommendations`, `apply_recommendation`

### Threads
- `create_threads_account`, `get_threads_account`

## Pagination

Cursor-based pagination is supported on list/read streams that expose `page_cursor`:

- Accounts: `list_ad_accounts`
- Campaigns: `list_campaigns`
- Ad sets: `list_ad_sets`
- Ads: `list_ads`, `list_ad_creatives`
- Media library: `list_ad_images`, `list_ad_videos`
- Insights: `list_insights`, `read_insights_job_results`
- Targeting: `search_interests`, `suggest_interests`, `search_behaviors`, `search_demographics`, `search_geo_locations`
- Audiences: `list_custom_audiences`
- Custom conversions: `list_custom_conversions`
- Catalogs: `list_product_catalogs`, `list_product_sets`, `list_products`
- Derived metrics: `list_ad_custom_derived_metrics`
- Partnership: `list_branded_content_ad_permissions`
- Reach & frequency: `list_reach_frequency_predictions`
- CAPI datasets: `list_business_datasets`
- Ads Library: `search_ads_archive`

Use `page_cursor` with the `paging.cursors.after` value from the previous response.
Responses preserve Meta's native `paging` object.

## Insights Query Notes

- `list_insights` accepts an optional `fields` parameter (defaults to a standard KPI set). Custom derived metric names from `list_ad_custom_derived_metrics` can be passed in `fields`.
- `list_insights` and `create_report` support either:
  - `date_range` as `{ "since": "YYYY-MM-DD", "until": "YYYY-MM-DD" }`, or
  - `date_range` as a preset (for example `last_30d`, `maximum`).
- `create_report.comparison_period` uses the same format and validation as `date_range`.
- `list_insights`, `create_report`, and async insights accept `action_attribution_windows`. Deprecated windows (`7d_view`, `28d_view`) return warnings and may yield empty data under v25.
- `previous_30d` is normalized to `last_30d`.
- For action metrics, use `action_breakdowns` (and optional `summary_action_breakdowns`) instead of mixing action keys into `breakdowns`.

### Async insights workflow (including MMM CSV)

1. `create_insights_job(object_id, breakdowns=["mmm"], export_format="csv", ...)` → `report_run_id`
2. Poll `read_insights_job(report_run_id)` until `async_status` is `Job Completed` (also returns `async_percent_completion` and `async_report_url` when available)
3. Fetch rows via `read_insights_job_results(report_run_id)` or download `async_report_url`

## v25 behavior notes

- **API version:** default `v25.0` — the current latest Marketing/Graph API version (released Feb 18, 2026). Set `META_GRAPH_API_VERSION` to override. Watch the Meta changelog before adopting a future version as the default.
- **Campaign budgets:** `create_campaign` no longer silently applies `daily_budget=1000`. Provide `daily_budget`/`lifetime_budget`, set `use_ad_set_level_budgets=True`, or pass `apply_default_budget=True` to opt in to the MCP default.
- **Special ad categories:** when `special_ad_categories` includes HOUSING, EMPLOYMENT, or FINANCIAL_PRODUCTS_SERVICES, `special_ad_category_country` is required on create/update.
- **Ad set placement opt-out:** `placement_soft_opt_out` must be an object keyed by placement group, for example:
  ```json
  {
    "facebook_positions": ["marketplace"],
    "instagram_positions": ["stream"]
  }
  ```
  Allowed keys: `facebook_positions`, `instagram_positions`, `audience_network_positions`, `messenger_positions`, `threads_positions`.
- **Advantage+ duplication/create guard:** deprecated Advantage+ Shopping/App campaign signatures are blocked on `create_campaign` (when `smart_promotion_type` is set) and on `clone_campaign` / `clone_ad_set` / `clone_ad` preflight. Use `migrate_to_advantage_plus` on `update_campaign` / `clone_campaign` where supported.
- **Carousel & catalog creatives:** `create_ad_creative` supports `carousel_cards`, `product_set_id` (see `list_product_sets`), and `url_tags`. `create_ad` supports `conversion_domain`.
- **Video upload:** `upload_ad_video_asset` accepts `video_source_url` or local `video_file_path` (multipart).
- **Ads Library:** `search_ads_archive` exposes v25 filters including `ad_active_status`, delivery date bounds, `search_page_ids`, `search_type`, `languages`, `media_type`, and `publisher_platforms`.

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
