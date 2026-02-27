"""Insights query tools."""


import json
from typing import Any, Dict, List, Optional, Union

from .graph_client import make_api_request, meta_api_tool
from .insight_query_params import normalize_breakdown_inputs, normalize_time_input
from .mcp_runtime import mcp_server

_DEFAULT_FIELDS = (
    "account_id,account_name,campaign_id,campaign_name,adset_id,adset_name,"
    "ad_id,ad_name,impressions,clicks,spend,cpc,cpm,ctr,reach,frequency,"
    "actions,action_values,conversions,unique_clicks,cost_per_action_type"
)

_REDUNDANT_ACTION_PREFIXES = (
    "omni_",
    "onsite_web_app_",
    "onsite_web_",
    "onsite_app_",
    "web_app_in_store_",
    "offsite_conversion.fb_pixel_",
)

_DEPRECATED_ATTRIBUTION_WINDOWS = {"7d_view", "28d_view"}


def _deprecated_windows(input_windows: Optional[List[str]]) -> List[str]:
    if not input_windows:
        return []
    flags = {
        str(window).strip().lower()
        for window in input_windows
        if str(window).strip().lower() in _DEPRECATED_ATTRIBUTION_WINDOWS
    }
    return sorted(flags)


def _strip_redundant_actions(row: Dict[str, Any]) -> Dict[str, Any]:
    for container_key in ("actions", "action_values", "cost_per_action_type"):
        values = row.get(container_key)
        if not isinstance(values, list):
            continue

        filtered = []
        for entry in values:
            action_type = ""
            if isinstance(entry, dict):
                action_type = str(entry.get("action_type", ""))
            if any(action_type.startswith(prefix) for prefix in _REDUNDANT_ACTION_PREFIXES):
                continue
            filtered.append(entry)
        row[container_key] = filtered
    return row


def _with_warning(payload: Dict[str, Any], deprecated: List[str]) -> Dict[str, Any]:
    if not deprecated:
        return payload

    warning = {
        "code": "deprecated_attribution_windows",
        "message": "One or more requested attribution windows are deprecated and may return empty data.",
        "deprecated_windows": deprecated,
        "recommended_windows": ["1d_click", "7d_click", "1d_view"],
    }

    existing = payload.get("warnings")
    if isinstance(existing, list):
        existing.append(warning)
    else:
        payload["warnings"] = [warning]
    return payload


def _append_warnings(payload: Dict[str, Any], warnings: List[Dict[str, Any]]) -> Dict[str, Any]:
    if not warnings:
        return payload
    existing = payload.get("warnings")
    if isinstance(existing, list):
        existing.extend(warnings)
    else:
        payload["warnings"] = list(warnings)
    return payload


@mcp_server.tool()
@meta_api_tool
async def list_insights(
    object_id: str,
    meta_access_token: Optional[str] = None,
    date_range: Union[str, Dict[str, str]] = "maximum",
    breakdown: str = "",
    breakdowns: Optional[List[str]] = None,
    action_breakdowns: Optional[List[str]] = None,
    summary_action_breakdowns: Optional[List[str]] = None,
    level: str = "ad",
    page_size: int = 25,
    page_cursor: str = "",
    action_attribution_windows: Optional[List[str]] = None,
    compact: bool = False,
) -> str:
    """Fetch insights for an account, campaign, ad set, or ad."""
    if not str(object_id or "").strip():
        return json.dumps({"error": "No object ID provided"}, indent=2)

    params: Dict[str, Any] = {
        "fields": _DEFAULT_FIELDS,
        "level": level,
        "page_size": int(page_size),
    }

    time_params, time_error, normalization_warnings = normalize_time_input(date_range, default_preset="maximum")
    if time_error:
        return json.dumps(time_error, indent=2)
    params.update(time_params or {})

    breakdown_params, breakdown_warnings = normalize_breakdown_inputs(
        breakdown=breakdown,
        breakdowns=breakdowns,
        action_breakdowns=action_breakdowns,
        summary_action_breakdowns=summary_action_breakdowns,
    )
    params.update(breakdown_params)
    normalization_warnings.extend(breakdown_warnings)

    if page_cursor:
        params["page_cursor"] = page_cursor

    deprecated_windows = _deprecated_windows(action_attribution_windows)
    if action_attribution_windows:
        params["action_attribution_windows"] = list(action_attribution_windows)

    payload = await make_api_request(f"{object_id}/insights", meta_access_token, params)

    if isinstance(payload, dict):
        _append_warnings(payload, normalization_warnings)
        _with_warning(payload, deprecated_windows)

        if compact and isinstance(payload.get("data"), list):
            payload["data"] = [
                _strip_redundant_actions(row) if isinstance(row, dict) else row
                for row in payload["data"]
            ]

    return json.dumps(payload, indent=2)
