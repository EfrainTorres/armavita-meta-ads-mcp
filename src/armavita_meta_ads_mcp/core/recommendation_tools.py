# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""Meta Marketing API recommendations / opportunity-score tools.

Reads the advisory `recommendations` field on any ad object (account, campaign,
ad set, ad). Recommendations are advisory: each entry carries a human-readable
`title`/`message`, a `code`, `importance`/`confidence`, and a `blame_field`
identifying the setting to change. To act on one, update that `blame_field` via
the relevant tool (`update_campaign`, `update_ad_set`, `update_ad`).
"""


import json
from typing import Any, Dict, Optional

from .graph_client import make_api_request, meta_api_tool
from .mcp_runtime import mcp_server
from mcp.types import ToolAnnotations


def _json(payload: Dict[str, Any]) -> str:
    return json.dumps(payload, indent=2)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=True))
@meta_api_tool
async def list_recommendations(
    object_id: str,
    meta_access_token: Optional[str] = None,
) -> str:
    """Fetch Meta's optimization recommendations for an ad account, campaign, ad set, or ad.

    Recommendations are advisory. Each entry typically includes a human-readable
    `title`/`message`, a `code`, `importance`/`confidence`, and a `blame_field`
    naming the setting Meta suggests changing. To act on one, update that field
    on the object via `update_campaign` / `update_ad_set` / `update_ad`.
    """
    if not str(object_id or "").strip():
        return _json({"error": "No object ID provided"})

    payload = await make_api_request(
        str(object_id).strip(),
        meta_access_token,
        {"fields": "recommendations"},
    )
    return _json(payload)


# No @meta_api_tool / token required: this tool makes NO Graph API call — it only
# inspects the recommendation locally and returns guidance. readOnly + closed-world.
@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, idempotentHint=True, openWorldHint=False))
async def apply_recommendation(
    object_id: str,
    recommendation_data: Dict[str, Any],
    meta_access_token: Optional[str] = None,
) -> str:
    """Explain how to act on a recommendation (recommendations are advisory).

    Meta's recommendations are advisory: there is NO documented generic
    "apply recommendation" endpoint, and POSTing a recommendation blob to the
    object node is silently ignored. Rather than issue a no-op call that falsely
    reports success, this tool inspects the recommendation's `blame_field` and
    returns the exact field + update tool to use. `object_id` is the object the
    recommendation was fetched on; `recommendation_data` is the recommendation
    entry from `list_recommendations`.
    """
    if not str(object_id or "").strip():
        return _json({"error": "No object ID provided"})
    if not recommendation_data:
        return _json({"error": "recommendation_data is required"})

    blame_field = None
    if isinstance(recommendation_data, dict):
        blame_field = recommendation_data.get("blame_field") or recommendation_data.get("code")

    return _json(
        {
            "status": "not_applied",
            "reason": (
                "Meta recommendations are advisory; there is no API endpoint that applies them "
                "directly. Apply the change yourself, then re-read the object to confirm."
            ),
            "object_id": str(object_id).strip(),
            "blame_field": blame_field,
            "how_to_apply": (
                "Update the setting named in `blame_field` on this object using "
                "update_campaign / update_ad_set / update_ad."
            ),
        }
    )
