# Local Client Guide

This OSS build is designed for local MCP clients only.

## Supported Use

- Claude Code
- Cursor
- Codex
- Any MCP client that launches stdio commands locally

## Start Command

```bash
armavita-meta-ads-mcp
```

## Authentication

Use one of:

- `META_ACCESS_TOKEN`
- `META_APP_ID` + `META_APP_SECRET`, then run:

```bash
armavita-meta-ads-mcp --login
```

Full auth details:

- see [Authentication Setup Guide](auth-setup.md)

## Notes

- This build runs with local `stdio` transport.
