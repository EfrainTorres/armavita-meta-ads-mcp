# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""Ad Custom Derived Metrics — read user-defined formulas computed from base metrics."""


import json
from typing import Any, Dict, Optional

from .graph_client import make_api_request, meta_api_tool
from .mcp_runtime import mcp_server
from mcp.types import ToolAnnotations


def _json(payload: Dict[str, Any]) -> str:
    return json.dumps(payload, indent=2)


def _normalize_account(ad_account_id: str) -> str:
    value = str(ad_account_id or "").strip()
    if not value:
        return ""
    return value if value.startswith("act_") else f"act_{value}"


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=True))
@meta_api_tool
async def list_ad_custom_derived_metrics(
    ad_account_id: str,
    meta_access_token: Optional[str] = None,
    page_size: int = 50,
    page_cursor: str = "",
) -> str:
    """List custom derived metrics defined on an ad account.

    Custom derived metrics are user-defined formulas (e.g., custom ROAS variants)
    that can then be requested as fields on `list_insights`.
    """
    normalized = _normalize_account(ad_account_id)
    if not normalized:
        return _json({"error": "ad_account_id is required"})

    params: Dict[str, Any] = {"page_size": int(page_size)}
    if page_cursor:
        params["page_cursor"] = page_cursor

    payload = await make_api_request(
        f"{normalized}/ad_custom_derived_metrics",
        meta_access_token,
        params,
    )
    return _json(payload)
