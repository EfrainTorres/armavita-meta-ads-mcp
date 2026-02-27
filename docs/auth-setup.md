# Authentication Setup Guide

This server supports two authentication modes:

1. direct token (`META_ACCESS_TOKEN`)
2. local OAuth (`META_APP_ID` + `META_APP_SECRET`)

## Option 1: Direct Access Token

Set a valid Meta access token in the server environment.

```bash
export META_ACCESS_TOKEN="EA..."
armavita-meta-ads-mcp
```

Notes:

- token scope must include ad account access for the tools you call
- token expiry and revoked permissions will cause auth errors at runtime

## Option 2: Local OAuth Login

Prerequisites:

1. a Meta app with valid Ads permissions
2. `META_APP_ID` and `META_APP_SECRET`
3. localhost callback availability

Run:

```bash
export META_APP_ID="..."
export META_APP_SECRET="..."
armavita-meta-ads-mcp --login
```

The login command opens a browser and stores token cache locally after successful auth.

## Token Cache Location

Cached token file is stored per OS:

- Linux: `~/.config/armavita-meta-ads-mcp/token_cache.json`
- macOS: `~/Library/Application Support/armavita-meta-ads-mcp/token_cache.json`
- Windows: `%APPDATA%\\armavita-meta-ads-mcp\\token_cache.json`

## Common Errors

`Authentication Required`:

- set `META_ACCESS_TOKEN`, or
- run `--login` with valid `META_APP_ID` and `META_APP_SECRET`

`No valid META_APP_ID configured`:

- set `META_APP_ID` in the MCP server environment used by your client launcher

`Authorization code exchange failed`:

- verify app secret, redirect URI, and permission approval status

`Invalid OAuth access token` / Meta error `code: 190`:

- token expired, revoked, or app secret changed
- if using direct token, generate a fresh token and restart the MCP server
- if using OAuth cache, rerun `armavita-meta-ads-mcp --login` to refresh the cached token

`Insufficient permission` / Meta error `code: 200` or `code: 10`:

- the token lacks required Meta Ads permissions/scopes
- confirm the app and user have approved the needed Ads scopes in Meta App Dashboard
- re-authenticate after updating scopes to issue a token with the new grants

`No ad account access` or empty account list:

- the authenticated user/token can be valid but not have access to the target ad account
- verify ad account assignment in Meta Business Manager and user role permissions
- check that you are querying the correct account ID format (`act_<ID>`)
