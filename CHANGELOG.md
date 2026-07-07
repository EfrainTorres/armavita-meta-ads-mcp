# Changelog

## 1.2.0 — 2026-07-07

### Added

- New tool groups, bringing the surface to **78 tools**: account controls, custom audiences (incl. lookalikes and user management), catalogs & products, Conversions API (`send_capi_events`, datasets, quality), custom conversions, async insights jobs (incl. MMM CSV export), custom derived metrics, reach & frequency predictions, partnership / branded-content ad permissions, recommendations, and Threads accounts.
- MCP tool annotations (`readOnlyHint` / `destructiveHint` / `idempotentHint` / `openWorldHint`) on all 78 tools.
- Cursor pagination (`page_cursor`) across all list streams; `create_report` follows insights cursor pagination end to end.
- Retry with exponential backoff for rate-limited Graph calls (rate limits retry any method; transient 5xx/transport errors retry idempotent GET/DELETE only).
- `default_conversion_value` support on custom conversion create/update/read.
- `level` validation on `create_insights_job`, matching `list_insights`.

### Changed

- v25 payload guards: `create_campaign`/`update_campaign` and `update_ad_set` drop non-writable fields (`campaign_budget_optimization`, `bid_cap`, `is_dynamic_creative` on update) and surface warnings instead of sending invalid params.
- `search_ads_archive`: `search_terms` is optional; requires at least one of `search_terms`/`search_page_ids`; v25 plural field names by default.
- `update_account_controls` sends `audience_controls`/`placement_controls` as whole objects per the v25 schema.
- Branded-content permissions moved to the Instagram User node; `grant_branded_content_ad_permission` replaces the former campaign-code tool.
- PII for CAPI events and Custom Audience uploads is normalized per Meta's field-specific rules (phone/DOB digits-only, city/state stripped of punctuation, zip without spaces/dashes) before SHA-256 hashing.

### Security

- OAuth login flow hardened: CSRF `state` nonce with fail-closed, constant-time validation; callback output HTML-escaped; token-leaking local `/token` route removed.
- Access tokens are redacted from returned URLs (including nested `paging.next`) and from logs; token cache written with `0600` permissions.
- Windows token-cache path falls back to the user home directory when `APPDATA` is unset.

## 1.1.0

- Meta Marketing API v25.0 baseline (`META_GRAPH_API_VERSION` override supported).
- Python 3.11+, `mcp[cli]==1.27.2`, stdio-only MCP transport.
