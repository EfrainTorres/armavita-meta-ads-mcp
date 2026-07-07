# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""Reach & Frequency predictions for reservation media buying."""


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


# Without an explicit `fields` request the node returns only `id`, so status
# polling and prediction outputs come back empty. Field names follow the
# Marketing API reachfrequencyprediction node schema.
_RF_READ_FIELDS = (
    "id,name,status,prediction_progress,reservation_status,campaign_group_id,"
    "objective,buying_type,target_spec,frequency_cap,start_time,end_time,"
    "time_created,expiration_time,curve_budget_reach,external_reach,external_budget,"
    "external_impression,external_maximum_reach,external_minimum_reach,"
    "destination_id,destination_ids,instagram_destination_id,prediction_mode,budget"
)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=False, destructiveHint=False, idempotentHint=False, openWorldHint=True))
@meta_api_tool
async def create_reach_frequency_prediction(
    ad_account_id: str,
    prediction: Dict[str, Any],
    meta_access_token: Optional[str] = None,
) -> str:
    """Create a reach & frequency prediction for reservation media buying.

    R&F is reservation-style buying with guaranteed reach — distinct from auction
    campaigns. Use `read_reach_frequency_prediction(rf_prediction_id)` to fetch
    results and `list_reach_frequency_predictions` to list past predictions.

    `prediction` is the dict POSTed verbatim. Common required keys:
    `objective`, `target_spec`, `start_time`, `end_time`, `budget`,
    `prediction_mode` (1 = standard reach), and `destination_id` (a Page id,
    Facebook-only) or `destination_ids` (`[page_id, instagram_account_id]`) when
    Instagram placements are used. Optional: `frequency_cap`, `instream_packages`.
    """
    normalized = _normalize_account(ad_account_id)
    if not normalized:
        return _json({"error": "ad_account_id is required"})
    if not prediction:
        return _json({"error": "prediction payload is required"})

    payload = await make_api_request(
        f"{normalized}/reachfrequencypredictions",
        meta_access_token,
        {k: json.dumps(v) if isinstance(v, (dict, list)) else v for k, v in prediction.items()},
        method="POST",
    )
    return _json(payload)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=True))
@meta_api_tool
async def read_reach_frequency_prediction(
    rf_prediction_id: str,
    meta_access_token: Optional[str] = None,
    fields: Optional[str] = None,
) -> str:
    """Fetch a reach & frequency prediction by ID (status + prediction outputs).

    Defaults to a full field set covering status polling (`status`,
    `prediction_progress`, `reservation_status`) and the reach/budget curve.
    Pass `fields` to request a custom subset.
    """
    if not str(rf_prediction_id or "").strip():
        return _json({"error": "rf_prediction_id is required"})

    payload = await make_api_request(
        str(rf_prediction_id).strip(),
        meta_access_token,
        {"fields": fields or _RF_READ_FIELDS},
    )
    return _json(payload)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=True))
@meta_api_tool
async def list_reach_frequency_predictions(
    ad_account_id: str,
    meta_access_token: Optional[str] = None,
    page_size: int = 25,
    page_cursor: str = "",
    fields: Optional[str] = None,
) -> str:
    """List reach & frequency predictions for an ad account."""
    normalized = _normalize_account(ad_account_id)
    if not normalized:
        return _json({"error": "ad_account_id is required"})

    params: Dict[str, Any] = {"fields": fields or _RF_READ_FIELDS, "page_size": int(page_size)}
    if page_cursor:
        params["page_cursor"] = page_cursor

    payload = await make_api_request(
        f"{normalized}/reachfrequencypredictions",
        meta_access_token,
        params,
    )
    return _json(payload)
