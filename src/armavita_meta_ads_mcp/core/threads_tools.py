# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""Threads Ads account discovery and creation.

Threads ads require a Threads account ID (one of three types):
  * Instagram-associated: matching-username Threads account in the Business Portfolio.
  * Instagram-backed: a mock Threads account created via API for an Instagram account
    that has no Threads profile.
  * Page-backed: a mock Threads account backed by a Facebook Page.

These tools wrap the discovery/creation endpoints; the resulting `threads_user_id`
should then be passed to `create_ad_creative(threads_user_id=...)`.
"""


import json
from typing import Any, Dict, Optional

from .graph_client import make_api_request, meta_api_tool
from .mcp_runtime import mcp_server
from mcp.types import ToolAnnotations


_VALID_MODES = {"associated", "ig_backed", "page_backed"}


def _json(payload: Dict[str, Any]) -> str:
    return json.dumps(payload, indent=2)


def _normalize_mode(mode: Optional[str]) -> str:
    return str(mode or "associated").strip().lower()


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=True))
@meta_api_tool
async def get_threads_account(
    instagram_user_id: Optional[str] = None,
    facebook_page_id: Optional[str] = None,
    mode: str = "associated",
    meta_access_token: Optional[str] = None,
) -> str:
    """Discover an existing Threads account ID for running Threads ads.

    `mode`:
      * "associated" — Instagram-associated Threads account (default; needs instagram_user_id).
        Use this when the IG account already has a matching-username Threads profile
        in the same Business Portfolio.
      * "ig_backed" — Instagram-backed (mock) Threads account (needs instagram_user_id).
      * "page_backed" — Page-backed (mock) Threads account (needs facebook_page_id).

    Returns `{threads_user_id: ...}` (or `{page_backed_threads_account_id: ...}`).
    Pass the returned ID into `create_ad_creative(threads_user_id=...)` to run
    Threads ads. If no account exists, use `create_threads_account` first.
    """
    normalized_mode = _normalize_mode(mode)
    if normalized_mode not in _VALID_MODES:
        return _json({"error": f"Invalid mode '{mode}'. Use one of: {sorted(_VALID_MODES)}"})

    if normalized_mode == "page_backed":
        if not facebook_page_id:
            return _json({"error": "facebook_page_id is required for mode='page_backed'"})
        payload = await make_api_request(
            str(facebook_page_id).strip(),
            meta_access_token,
            {"fields": "page_backed_threads_account_id"},
        )
        return _json(payload)

    if not instagram_user_id:
        return _json({"error": f"instagram_user_id is required for mode='{normalized_mode}'"})

    edge = "connected_threads_user" if normalized_mode == "associated" else "instagram_backed_threads_user"
    payload = await make_api_request(
        f"{str(instagram_user_id).strip()}/{edge}",
        meta_access_token,
        {"fields": "threads_user_id"},
    )
    return _json(payload)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=False, destructiveHint=False, idempotentHint=False, openWorldHint=True))
@meta_api_tool
async def create_threads_account(
    instagram_user_id: Optional[str] = None,
    facebook_page_id: Optional[str] = None,
    mode: str = "ig_backed",
    meta_access_token: Optional[str] = None,
) -> str:
    """Create a mock Threads account so ads can run when no real Threads profile exists.

    Use this only after `get_threads_account` returns nothing for the desired mode.

    `mode`:
      * "ig_backed" — create from an Instagram account (needs instagram_user_id).
      * "page_backed" — create from a Facebook Page (needs facebook_page_id).

    Returns the Threads account ID. If one already exists for this IG/Page,
    returns the existing ID (idempotent). Use the returned `threads_user_id`
    in `create_ad_creative(threads_user_id=...)`.
    """
    normalized_mode = _normalize_mode(mode)
    if normalized_mode not in {"ig_backed", "page_backed"}:
        return _json(
            {"error": f"Invalid mode '{mode}'. Use 'ig_backed' or 'page_backed' for creation."}
        )

    if normalized_mode == "page_backed":
        if not facebook_page_id:
            return _json({"error": "facebook_page_id is required for mode='page_backed'"})
        payload = await make_api_request(
            f"{str(facebook_page_id).strip()}/page_backed_threads_accounts",
            meta_access_token,
            {},
            method="POST",
        )
        return _json(payload)

    if not instagram_user_id:
        return _json({"error": "instagram_user_id is required for mode='ig_backed'"})

    payload = await make_api_request(
        f"{str(instagram_user_id).strip()}/instagram_backed_threads_user",
        meta_access_token,
        {},
        method="POST",
    )
    return _json(payload)
