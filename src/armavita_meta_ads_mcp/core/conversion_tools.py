# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""Custom conversion CRUD tools."""

import json
from typing import Any, Dict, Optional

from .graph_client import make_api_request, meta_api_tool
from .mcp_runtime import mcp_server
from mcp.types import ToolAnnotations

_DEFAULT_FIELDS = (
    "id,name,pixel,rule,custom_event_type,is_unavailable,creation_time,last_fired_time,"
    "description,event_source_id,action_source_type,advanced_rule,default_conversion_value"
)


def _json(payload: Dict[str, Any]) -> str:
    return json.dumps(payload, indent=2)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=True))
@meta_api_tool
async def list_custom_conversions(
    ad_account_id: str,
    meta_access_token: Optional[str] = None,
    page_size: int = 25,
    page_cursor: str = "",
    fields: Optional[str] = None,
) -> str:
    """List custom conversions for an ad account."""
    if not ad_account_id:
        return _json({"error": "ad_account_id is required"})

    params: Dict[str, Any] = {
        "fields": fields or _DEFAULT_FIELDS,
        "page_size": int(page_size),
    }
    if page_cursor:
        params["page_cursor"] = page_cursor

    payload = await make_api_request(f"{ad_account_id}/customconversions", meta_access_token, params)
    return _json(payload)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=True))
@meta_api_tool
async def read_custom_conversion(
    custom_conversion_id: str,
    meta_access_token: Optional[str] = None,
    fields: Optional[str] = None,
) -> str:
    """Read a custom conversion by ID."""
    if not custom_conversion_id:
        return _json({"error": "custom_conversion_id is required"})

    payload = await make_api_request(
        custom_conversion_id,
        meta_access_token,
        {"fields": fields or _DEFAULT_FIELDS},
    )
    return _json(payload)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=False, destructiveHint=False, idempotentHint=False, openWorldHint=True))
@meta_api_tool
async def create_custom_conversion(
    ad_account_id: str,
    name: str,
    event_source_id: str,
    rule: Dict[str, Any],
    custom_event_type: str,
    meta_access_token: Optional[str] = None,
    description: Optional[str] = None,
    advanced_rule: Optional[Dict[str, Any]] = None,
    action_source_type: Optional[str] = None,
    default_conversion_value: Optional[float] = None,
) -> str:
    """Create a custom conversion on an ad account.

    `default_conversion_value` assigns a fixed value to conversions that arrive
    without one (enables value-based optimization).
    """
    if not ad_account_id:
        return _json({"error": "ad_account_id is required"})
    if not name:
        return _json({"error": "name is required"})
    if not event_source_id:
        return _json({"error": "event_source_id is required"})
    if not rule:
        return _json({"error": "rule is required"})
    if not custom_event_type:
        return _json({"error": "custom_event_type is required"})

    payload: Dict[str, Any] = {
        "name": name,
        "event_source_id": event_source_id,
        "rule": json.dumps(rule),
        "custom_event_type": custom_event_type,
    }
    if description:
        payload["description"] = description
    if advanced_rule is not None:
        payload["advanced_rule"] = json.dumps(advanced_rule)
    if action_source_type:
        payload["action_source_type"] = action_source_type
    if default_conversion_value is not None:
        payload["default_conversion_value"] = float(default_conversion_value)

    result = await make_api_request(f"{ad_account_id}/customconversions", meta_access_token, payload, method="POST")
    return _json(result)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=False, destructiveHint=False, idempotentHint=True, openWorldHint=True))
@meta_api_tool
async def update_custom_conversion(
    custom_conversion_id: str,
    meta_access_token: Optional[str] = None,
    name: Optional[str] = None,
    description: Optional[str] = None,
    rule: Optional[Dict[str, Any]] = None,
    advanced_rule: Optional[Dict[str, Any]] = None,
    custom_event_type: Optional[str] = None,
    default_conversion_value: Optional[float] = None,
) -> str:
    """Update a custom conversion."""
    if not custom_conversion_id:
        return _json({"error": "custom_conversion_id is required"})

    payload: Dict[str, Any] = {}
    if name is not None:
        payload["name"] = name
    if description is not None:
        payload["description"] = description
    if rule is not None:
        payload["rule"] = json.dumps(rule)
    if advanced_rule is not None:
        payload["advanced_rule"] = json.dumps(advanced_rule)
    if custom_event_type is not None:
        payload["custom_event_type"] = custom_event_type
    if default_conversion_value is not None:
        payload["default_conversion_value"] = float(default_conversion_value)

    if not payload:
        return _json({"error": "No update parameters provided"})

    result = await make_api_request(custom_conversion_id, meta_access_token, payload, method="POST")
    return _json(result)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=False, destructiveHint=True, idempotentHint=False, openWorldHint=True))
@meta_api_tool
async def delete_custom_conversion(
    custom_conversion_id: str,
    meta_access_token: Optional[str] = None,
) -> str:
    """Archive/delete a custom conversion."""
    if not custom_conversion_id:
        return _json({"error": "custom_conversion_id is required"})

    result = await make_api_request(custom_conversion_id, meta_access_token, {}, method="DELETE")
    return _json(result)
