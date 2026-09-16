# Security

## Report a vulnerability

Report vulnerabilities [privately on GitHub](https://github.com/EfrainTorres/armavita-meta-ads-mcp/security/advisories/new).
Include the version, steps to
reproduce the issue, and its impact. Never post tokens, app secrets, or customer data in public issues.

## Use it safely

- Run this server locally with a trusted MCP client. It has no public HTTP endpoint. The local
  connection does not limit incoming MCP message sizes or simultaneous requests.
- Keep credentials in your environment or the private OAuth cache. Never include them in prompts,
  tool arguments, or committed configuration. Revoke any credential that has been exposed.
- Give the Meta token only the permissions and account access you need. Keep your MCP client's
  write approvals enabled.
- Upload only customer data you are authorized to use. Hashing does not make that data anonymous.
- Enable local media uploads only from a folder whose parent directories you also control. Keep
  other processes from changing the folder or its files during uploads; path checks cannot prevent those races.

## Approving changes

Review a mutation plan before applying it. Plans expire after 15 minutes and disappear when the
server restarts. Each plan contains one request; a group of plans is not an all-or-nothing operation.

Applying a plan requires `APPLY_LIVE_META_ADS_CHANGES`. Deletions and other removals also require
`CONFIRM_META_ADS_REMOVALS`. These phrases confirm intent; they do not replace client approvals
or Meta permissions.

Writes are never retried automatically. If a request times out or returns `outcome_unknown`, check
Meta before trying again—the change may already have happened.

See [setup](docs/SETUP.md) for authentication and [migration status](docs/MIGRATION.md#status)
for workflows that still need live testing.
