# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""Account-level controls (`AdAccountBusinessConstraints`).

The `/act_{id}/account_controls` edge holds account-wide controls applied to every
campaign on the account. Per Meta's Marketing API it accepts two object params on
write — `audience_controls` and `placement_controls` — and the constraint object
reads back the fields: `audience_controls`, `placement_controls`,
`is_age_restriction_enabled`, `status`, `campaigns_with_error`. The edge returns a
LIST (`{"data": [...]}`), empty (`{"data": []}`) when no controls are configured.
"""


import json
from typing import Any, Dict, Optional

from .graph_client import make_api_request, meta_api_tool
from .mcp_runtime import mcp_server
from mcp.types import ToolAnnotations

# Read fields on the AdAccountBusinessConstraints object.
_CONTROL_FIELDS = "audience_controls,placement_controls,is_age_restriction_enabled,status,campaigns_with_error"


def _json(payload: Dict[str, Any]) -> str:
    return json.dumps(payload, indent=2)


def _normalize_account(ad_account_id: str) -> str:
    value = str(ad_account_id or "").strip()
    if not value:
        return ""
    return value if value.startswith("act_") else f"act_{value}"


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=True))
@meta_api_tool
async def get_account_controls(
    ad_account_id: str,
    meta_access_token: Optional[str] = None,
) -> str:
    """Read the ad account's account-level controls (audience_controls, placement_controls, age restriction, status).

    Returns a list (`{"data": [...]}`), empty when no controls are configured.
    """
    normalized = _normalize_account(ad_account_id)
    if not normalized:
        return _json({"error": "ad_account_id is required"})

    payload = await make_api_request(
        f"{normalized}/account_controls",
        meta_access_token,
        {"fields": _CONTROL_FIELDS},
    )
    return _json(payload)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=False, destructiveHint=False, idempotentHint=True, openWorldHint=True))
@meta_api_tool
async def update_account_controls(
    ad_account_id: str,
    audience_controls: Optional[Dict[str, Any]] = None,
    placement_controls: Optional[Dict[str, Any]] = None,
    meta_access_token: Optional[str] = None,
) -> str:
    """Set account-level controls (applied to all campaigns on the account).

    Per Meta's Marketing API the edge accepts two object params, at least one
    required: `audience_controls` and `placement_controls`. Each is sent as a single
    JSON object (not flattened). Read the current state with `get_account_controls`.
    These account-wide controls complement, and do not replace, per-ad-set targeting.
    """
    normalized = _normalize_account(ad_account_id)
    if not normalized:
        return _json({"error": "ad_account_id is required"})
    if not audience_controls and not placement_controls:
        return _json({"error": "Provide audience_controls and/or placement_controls"})

    # Each control set is a single object param (the request pipeline JSON-encodes it).
    payload: Dict[str, Any] = {}
    if audience_controls:
        payload["audience_controls"] = audience_controls
    if placement_controls:
        payload["placement_controls"] = placement_controls

    result = await make_api_request(
        f"{normalized}/account_controls",
        meta_access_token,
        payload,
        method="POST",
    )
    return _json(result)
