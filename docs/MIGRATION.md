# Moving from Python

The Rust version keeps the main advertising workflows and expands to 125 tools, compared with 78 in
[Python 1.2.0](https://github.com/EfrainTorres/armavita-meta-ads-mcp/tree/102f07fefa35dc94af5a00661ac51cfc16b63155).
Both versions use local stdio and AGPLv3-only licensing. Some inputs and results have changed,
including for tools whose names stayed the same.

## Update your setup

- Install the Rust executable and keep the server name `meta-ads-armavita` in your client config.
- Remove `meta_access_token` from tool calls. Use `META_ACCESS_TOKEN` or browser login.
- The API version is fixed at v26.0; `META_GRAPH_API_VERSION` no longer changes it.
- Use `--app-id` only with `--login`. On Windows, use an environment token.
- Put local uploads under `META_MEDIA_ROOT` and use paths relative to that folder.

See the [setup guide](SETUP.md) for configuration and permissions.

## Update your workflows

Results are structured objects rather than JSON strings. Read the `status` and `data` fields,
and handle the shared `error` object. Native image results remain image content.

Use the returned `next_cursor` as the next call's `page_cursor`; do not follow provider paging URLs.
Derived metrics and dataset quality use a Business ID; recommendations use an ad-account ID.
Existing-object writes may require the owning ad-account ID as well as the object ID.

| Python tool | Rust replacement |
| --- | --- |
| `search_pages`, `list_account_pages` | `list_pages` |
| `export_ad_image_file` | `read_ad_image`; save the returned image in your client |
| `clone_ad_creative` | Read the creative, then create a new one explicitly |
| `apply_recommendation` | Get its signature with `read_ad_inventory` (recommendations), then apply it with `manage_ad_extensions`; the old Python helper did not apply changes |
| `search_web_content`, `read_web_content` | Use the account, campaign, ad, and other domain tools directly |

The Rust version adds partnership revocation with `revoke_branded_content_ad_permission` and
four tools for reviewing changes: `build_mutation_plan`, `get_mutation_plan`, `apply_mutation_plan`,
and `discard_mutation_plan`. Their [approval requirements](../SECURITY.md#approving-changes) also
cover deletions. After an uncertain write result, check Meta before retrying.

The original 77 Rust tool names remain available. Reporting options and preview formats have grown;
new tools add lead forms, automation, catalog administration, comments, and asset management.
Use selected fields in `inspect_ad_object` when you need the full creative or targeting details.

## Status

The Rust server is smoke tested and heavily used internally.
Please [report any issues](https://github.com/EfrainTorres/armavita-meta-ads-mcp/issues)
with the tool name and steps to reproduce.

For customer-audience uploads using mobile/app identifiers (`MADID`/`APPUID`), check Meta's hashing
requirements; its SDK and examples differ. Advanced features also depend on Meta permissions and
asset eligibility. WhatsApp-specific workflows are not included.
