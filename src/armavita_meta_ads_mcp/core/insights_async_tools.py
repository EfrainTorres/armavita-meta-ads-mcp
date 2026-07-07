# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""Asynchronous Insights jobs.

Use when an insights query times out, exceeds row limits, or needs MMM/long-range
breakdowns. Workflow:
  1. `create_insights_job(object_id, ...)` returns a `report_run_id`.
  2. Poll `read_insights_job(report_run_id)` until `async_status` is `Job Completed`.
  3. Fetch results with `read_insights_job_results(report_run_id)`.

v25.0+ surfaces richer error fields on failure (`error_code`, `error_subcode`,
`error_user_title`, `error_user_msg`); they're extracted from the job payload.
"""


import json
from typing import Any, Dict, List, Optional, Union

from .graph_client import make_api_request, meta_api_tool
from .insight_query_params import normalize_breakdown_inputs, normalize_time_input
from .insight_tools import _DEFAULT_FIELDS, _VALID_LEVELS, _append_warnings
from .meta_v25_guards import append_warning, attribution_window_warning, deprecated_attribution_windows
from .mcp_runtime import mcp_server
from mcp.types import ToolAnnotations


_ERROR_FIELDS = (
    "error_code",
    "error_message",
    "error_subcode",
    "error_user_title",
    "error_user_msg",
)

_ASYNC_JOB_FIELDS = "async_status,async_percent_completion,async_report_url"


def _json(payload: Dict[str, Any]) -> str:
    return json.dumps(payload, indent=2)


def _extract_error_fields(payload: Dict[str, Any]) -> Optional[Dict[str, Any]]:
    if not isinstance(payload, dict):
        return None
    extracted = {key: payload.get(key) for key in _ERROR_FIELDS if payload.get(key) is not None}
    return extracted or None


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=False, destructiveHint=False, idempotentHint=False, openWorldHint=True))
@meta_api_tool
async def create_insights_job(
    object_id: str,
    meta_access_token: Optional[str] = None,
    date_range: Union[str, Dict[str, str]] = "maximum",
    breakdown: str = "",
    breakdowns: Optional[List[str]] = None,
    action_breakdowns: Optional[List[str]] = None,
    summary_action_breakdowns: Optional[List[str]] = None,
    level: str = "ad",
    fields: Optional[str] = None,
    action_attribution_windows: Optional[List[str]] = None,
    export_format: Optional[str] = None,
) -> str:
    """Start an asynchronous Insights job. Returns `{report_run_id: "..."}`.

    Use when `list_insights` would time out or hit row limits — large date
    ranges, MMM breakdowns, or many breakdowns combined. For MMM exports pass
    `breakdowns=["mmm"]` and `export_format="csv"`. Workflow:
      1. Call this — get `report_run_id`.
      2. Poll `read_insights_job(report_run_id)` until `async_status` is
         "Job Completed" (other states: "Job Started", "Job Running",
         "Job Failed", "Job Skipped").
      3. Call `read_insights_job_results(report_run_id)` or download
         `async_report_url` from the job status when present.
    """
    if not str(object_id or "").strip():
        return _json({"error": "object_id is required"})

    normalized_level = str(level or "").strip().lower()
    if normalized_level not in _VALID_LEVELS:
        return _json({
            "error": "invalid_level",
            "message": f"level must be one of {sorted(_VALID_LEVELS)}.",
            "provided": level,
        })

    time_params, time_error, time_warnings = normalize_time_input(date_range, default_preset="maximum")
    if time_error:
        return _json(time_error)

    breakdown_params, breakdown_warnings = normalize_breakdown_inputs(
        breakdown=breakdown,
        breakdowns=breakdowns,
        action_breakdowns=action_breakdowns,
        summary_action_breakdowns=summary_action_breakdowns,
    )

    params: Dict[str, Any] = {"level": normalized_level, "fields": fields or _DEFAULT_FIELDS}
    params.update(time_params)
    params.update(breakdown_params)

    if action_attribution_windows:
        params["action_attribution_windows"] = list(action_attribution_windows)
    if export_format:
        normalized_export = str(export_format).strip().lower()
        if normalized_export not in {"csv"}:
            return _json(
                {
                    "error": "invalid_export_format",
                    "message": "export_format must be 'csv' for async insights jobs",
                    "export_format": export_format,
                }
            )
        params["export_format"] = normalized_export

    payload = await make_api_request(
        f"{str(object_id).strip()}/insights",
        meta_access_token,
        params,
        method="POST",
    )
    if isinstance(payload, dict):
        normalization_warnings = list(time_warnings) + list(breakdown_warnings)
        _append_warnings(payload, normalization_warnings)
        deprecated = deprecated_attribution_windows(action_attribution_windows)
        append_warning(payload, attribution_window_warning(deprecated))
    return _json(payload)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=True))
@meta_api_tool
async def read_insights_job(
    report_run_id: str,
    meta_access_token: Optional[str] = None,
) -> str:
    """Read async-job status. Surfaces v25.0 richer error fields on failure."""
    if not str(report_run_id or "").strip():
        return _json({"error": "report_run_id is required"})

    payload = await make_api_request(
        str(report_run_id).strip(),
        meta_access_token,
        {"fields": _ASYNC_JOB_FIELDS},
    )

    if isinstance(payload, dict):
        error_details = _extract_error_fields(payload)
        async_status = str(payload.get("async_status") or "")
        if error_details and async_status and async_status not in {"Job Completed"}:
            payload["_error_details"] = error_details
    return _json(payload)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=True))
@meta_api_tool
async def read_insights_job_results(
    report_run_id: str,
    meta_access_token: Optional[str] = None,
    page_size: int = 200,
    page_cursor: str = "",
) -> str:
    """Fetch results for a completed async Insights job."""
    if not str(report_run_id or "").strip():
        return _json({"error": "report_run_id is required"})

    params: Dict[str, Any] = {"page_size": int(page_size)}
    if page_cursor:
        params["page_cursor"] = page_cursor

    payload = await make_api_request(
        f"{str(report_run_id).strip()}/insights",
        meta_access_token,
        params,
    )
    return _json(payload)
