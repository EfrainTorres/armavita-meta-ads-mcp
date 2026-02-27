# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""Targeting discovery and audience estimation tools."""


import json
import os
from typing import Any, Dict, List, Optional

from .graph_client import make_api_request, meta_api_tool
from .mcp_runtime import mcp_server


def _as_json(payload: Dict[str, Any]) -> str:
    return json.dumps(payload, indent=2)


def _has_location_or_custom_audience(targeting: Dict[str, Any]) -> bool:
    if not isinstance(targeting, dict):
        return False

    geo = targeting.get("geo_locations")
    if isinstance(geo, dict):
        for key in ("countries", "regions", "cities", "zips", "geo_markets", "country_groups"):
            candidate = geo.get(key)
            if isinstance(candidate, list) and candidate:
                return True

    custom = targeting.get("custom_audiences")
    if isinstance(custom, list) and custom:
        return True

    flexible = targeting.get("flexible_spec")
    if isinstance(flexible, list):
        for entry in flexible:
            if not isinstance(entry, dict):
                continue
            nested_custom = entry.get("custom_audiences")
            if isinstance(nested_custom, list) and nested_custom:
                return True

    return False


def _deprecated_interest_validation_mode(
    ad_account_id: Optional[str],
    targeting: Optional[Dict[str, Any]],
    interest_list: Optional[List[str]],
    interest_fbid_list: Optional[List[str]],
) -> bool:
    return bool(interest_list or interest_fbid_list) or (not ad_account_id and not targeting)


def _extract_graph_error(payload: Dict[str, Any]) -> Dict[str, Any]:
    if not isinstance(payload, dict):
        return {}

    error_container = payload.get("error")
    if isinstance(error_container, dict):
        details = error_container.get("details")
        if isinstance(details, dict) and isinstance(details.get("error"), dict):
            return details["error"]
        if isinstance(error_container.get("error"), dict):
            return error_container["error"]
        return error_container

    return {}


def _missing_location_error_payload(error_obj: Dict[str, Any], ad_account_id: str) -> Optional[Dict[str, Any]]:
    if not isinstance(error_obj, dict):
        return None

    subcode = error_obj.get("error_subcode")
    title = error_obj.get("error_user_title")
    if subcode != 1885364 and title != "Missing Target Audience Location":
        return None

    error_data = error_obj.get("error_data") if isinstance(error_obj.get("error_data"), dict) else {}
    return {
        "error": "Missing target audience location",
        "details": error_obj.get("error_user_msg")
        or "Select at least one location, or choose a custom audience.",
        "endpoint_used": f"{ad_account_id}/reachestimate",
        "action_required": "Add geo_locations or include custom_audiences in targeting_spec.",
        "blame_field_specs": error_data.get("blame_field_specs"),
    }


def _normalize_reach_result(
    payload: Dict[str, Any],
    ad_account_id: str,
    targeting: Dict[str, Any],
    optimization_goal: str,
    fallback_endpoint_used: Optional[str] = None,
) -> Dict[str, Any]:
    data = payload.get("data") if isinstance(payload, dict) else None

    if isinstance(data, list):
        if not data:
            return {
                "error": "No estimation data returned from Meta API",
                "raw_response": payload,
                "debug_info": {
                    "response_type": str(type(payload)),
                    "endpoint_used": f"{ad_account_id}/reachestimate",
                },
            }

        first = data[0] if isinstance(data[0], dict) else {}
        result = {
            "success": True,
            "ad_account_id": ad_account_id,
            "targeting": targeting,
            "optimization_goal": optimization_goal,
            "estimated_audience_size": first.get("estimate_mau", 0),
            "estimate_details": {
                "monthly_active_users": first.get("estimate_mau", 0),
                "daily_outcomes_curve": first.get("estimate_dau", []),
                "bid_estimate": first.get("bid_estimates", {}),
                "unsupported_targeting": first.get("unsupported_targeting", []),
            },
            "raw_response": payload,
        }
        if fallback_endpoint_used:
            result["fallback_endpoint_used"] = fallback_endpoint_used
        return result

    if isinstance(data, dict):
        lower = data.get("users_lower_bound", data.get("estimate_mau_lower_bound"))
        upper = data.get("users_upper_bound", data.get("estimate_mau_upper_bound"))
        midpoint = 0
        try:
            if isinstance(lower, (int, float)) and isinstance(upper, (int, float)):
                midpoint = int((lower + upper) / 2)
        except Exception:  # noqa: BLE001
            midpoint = 0

        result = {
            "success": True,
            "ad_account_id": ad_account_id,
            "targeting": targeting,
            "optimization_goal": optimization_goal,
            "estimated_audience_size": midpoint,
            "estimate_details": {
                "users_lower_bound": lower,
                "users_upper_bound": upper,
                "estimate_ready": data.get("estimate_ready"),
            },
            "raw_response": payload,
        }
        if fallback_endpoint_used:
            result["fallback_endpoint_used"] = fallback_endpoint_used
        return result

    return {
        "error": "No estimation data returned from Meta API",
        "raw_response": payload,
        "debug_info": {
            "response_type": str(type(payload)),
            "endpoint_used": f"{ad_account_id}/reachestimate",
        },
    }


async def _run_delivery_estimate_fallback(
    ad_account_id: str,
    meta_access_token: str,
    targeting: Dict[str, Any],
    optimization_goal: str,
) -> Dict[str, Any]:
    payload = await make_api_request(
        f"{ad_account_id}/delivery_estimate",
        meta_access_token,
        {
            "targeting_spec": targeting,
            "optimization_goal": optimization_goal,
        },
        method="GET",
    )
    return _normalize_reach_result(
        payload,
        ad_account_id=ad_account_id,
        targeting=targeting,
        optimization_goal=optimization_goal,
        fallback_endpoint_used="delivery_estimate",
    )


@mcp_server.tool()
@meta_api_tool
async def search_interests(
    query: str,
    meta_access_token: Optional[str] = None,
    page_size: int = 25,
    page_cursor: str = "",
) -> str:
    """Find audience interests by keyword."""
    if not str(query or "").strip():
        return _as_json({"error": "No query provided"})

    params: Dict[str, Any] = {
        "type": "adinterest",
        "q": query,
        "page_size": int(page_size),
    }
    if page_cursor:
        params["page_cursor"] = page_cursor

    payload = await make_api_request("search", meta_access_token, params)
    return _as_json(payload)


@mcp_server.tool()
@meta_api_tool
async def suggest_interests(
    interest_list: List[str],
    meta_access_token: Optional[str] = None,
    page_size: int = 25,
    page_cursor: str = "",
) -> str:
    """Fetch related interests from a base list of interests."""
    if not interest_list:
        return _as_json({"error": "No interest list provided"})

    params: Dict[str, Any] = {
        "type": "adinterestsuggestion",
        "interest_list": json.dumps(list(interest_list)),
        "page_size": int(page_size),
    }
    if page_cursor:
        params["page_cursor"] = page_cursor

    payload = await make_api_request("search", meta_access_token, params)
    return _as_json(payload)


@mcp_server.tool()
@meta_api_tool
async def estimate_audience_size(
    meta_access_token: Optional[str] = None,
    ad_account_id: Optional[str] = None,
    targeting: Optional[Dict[str, Any]] = None,
    optimization_goal: str = "REACH",
    interest_list: Optional[List[str]] = None,
    interest_fbid_list: Optional[List[str]] = None,
) -> str:
    """Estimate audience size for targeting specs or validate deprecated interest-list mode."""
    deprecated_interest_mode = _deprecated_interest_validation_mode(
        ad_account_id=ad_account_id,
        targeting=targeting,
        interest_list=interest_list,
        interest_fbid_list=interest_fbid_list,
    )

    if deprecated_interest_mode and not targeting:
        if not (interest_list or interest_fbid_list):
            return _as_json({"error": "No interest list or FBID list provided"})

        deprecated_interest_payload: Dict[str, Any] = {"type": "adinterestvalid"}
        if interest_list:
            deprecated_interest_payload["interest_list"] = json.dumps(list(interest_list))
        if interest_fbid_list:
            deprecated_interest_payload["interest_fbid_list"] = json.dumps(list(interest_fbid_list))
        payload = await make_api_request(
            "search",
            meta_access_token,
            deprecated_interest_payload,
        )
        return _as_json(payload)

    if not ad_account_id:
        return _as_json(
            {
                "error": "ad_account_id is required for comprehensive audience estimation",
                "details": "For simple interest validation, use interest_list or interest_fbid_list parameters",
            }
        )

    if not targeting:
        return _as_json(
            {
                "error": "targeting specification is required for comprehensive audience estimation",
                "example": {
                    "age_min": 25,
                    "age_max": 65,
                    "geo_locations": {"countries": ["US"]},
                    "flexible_spec": [{"interests": [{"id": "6003371567474"}]}],
                },
            }
        )

    if not _has_location_or_custom_audience(targeting):
        return _as_json(
            {
                "error": "Missing target audience location",
                "details": "Select at least one location in targeting.geo_locations or include a custom audience.",
                "action_required": "Add geo_locations with countries/regions/cities/zips or include custom_audiences.",
                "example": {
                    "geo_locations": {"countries": ["US"]},
                    "age_min": 25,
                    "age_max": 65,
                },
            }
        )

    fallback_disabled = os.environ.get("META_MCP_DISABLE_DELIVERY_FALLBACK", "1") == "1"

    try:
        reach_payload = await make_api_request(
            f"{ad_account_id}/reachestimate",
            meta_access_token,
            {"targeting_spec": targeting},
            method="GET",
        )

        if isinstance(reach_payload, dict) and reach_payload.get("error"):
            graph_error = _extract_graph_error(reach_payload)
            missing_location = _missing_location_error_payload(graph_error, ad_account_id)
            if missing_location:
                return _as_json(missing_location)

            if fallback_disabled:
                return _as_json(
                    {
                        "error": "Graph API returned an error for reachestimate",
                        "details": reach_payload.get("error"),
                        "endpoint_used": f"{ad_account_id}/reachestimate",
                        "request_params": {"has_targeting_spec": bool(targeting)},
                        "note": "delivery_estimate fallback disabled via META_MCP_DISABLE_DELIVERY_FALLBACK",
                    }
                )

            fallback_result = await _run_delivery_estimate_fallback(
                ad_account_id=ad_account_id,
                meta_access_token=meta_access_token,
                targeting=targeting,
                optimization_goal=optimization_goal,
            )
            if fallback_result.get("success"):
                return _as_json(fallback_result)

            return _as_json(
                {
                    "error": "Graph API returned an error for reachestimate; delivery_estimate fallback did not return usable data",
                    "reachestimate_error": reach_payload.get("error"),
                    "fallback_endpoint_used": "delivery_estimate",
                    "fallback_raw_response": fallback_result,
                    "endpoint_used": f"{ad_account_id}/reachestimate",
                }
            )

        normalized = _normalize_reach_result(
            reach_payload,
            ad_account_id=ad_account_id,
            targeting=targeting,
            optimization_goal=optimization_goal,
        )
        return _as_json(normalized)

    except Exception as exc:  # noqa: BLE001
        if not fallback_disabled:
            try:
                fallback_result = await _run_delivery_estimate_fallback(
                    ad_account_id=ad_account_id,
                    meta_access_token=meta_access_token,
                    targeting=targeting,
                    optimization_goal=optimization_goal,
                )
                if fallback_result.get("success"):
                    return _as_json(fallback_result)
            except Exception:  # noqa: BLE001
                pass

        return _as_json(
            {
                "error": f"Failed to get audience estimation from reachestimate endpoint: {exc}",
                "details": "Check targeting parameters and account permissions",
                "error_type": "general_api_error",
                "endpoint_used": f"{ad_account_id}/reachestimate",
            }
        )


@mcp_server.tool()
@meta_api_tool
async def search_behaviors(
    meta_access_token: Optional[str] = None,
    page_size: int = 50,
    page_cursor: str = "",
) -> str:
    """Search behavior-based targeting categories."""
    params: Dict[str, Any] = {
        "type": "adTargetingCategory",
        "class": "behaviors",
        "page_size": int(page_size),
    }
    if page_cursor:
        params["page_cursor"] = page_cursor
    payload = await make_api_request("search", meta_access_token, params)
    return _as_json(payload)


@mcp_server.tool()
@meta_api_tool
async def search_demographics(
    meta_access_token: Optional[str] = None,
    demographic_class: str = "demographics",
    page_size: int = 50,
    page_cursor: str = "",
) -> str:
    """Search demographic targeting classes."""
    params: Dict[str, Any] = {
        "type": "adTargetingCategory",
        "class": demographic_class,
        "page_size": int(page_size),
    }
    if page_cursor:
        params["page_cursor"] = page_cursor
    payload = await make_api_request("search", meta_access_token, params)
    return _as_json(payload)


@mcp_server.tool()
@meta_api_tool
async def search_geo_locations(
    query: str,
    meta_access_token: Optional[str] = None,
    location_types: Optional[List[str]] = None,
    page_size: int = 25,
    page_cursor: str = "",
) -> str:
    """Search geo-location entities for targeting."""
    if not str(query or "").strip():
        return _as_json({"error": "No query provided"})

    params: Dict[str, Any] = {
        "type": "adgeolocation",
        "q": query,
        "page_size": int(page_size),
    }
    if page_cursor:
        params["page_cursor"] = page_cursor
    if location_types:
        params["location_types"] = json.dumps(list(location_types))
    payload = await make_api_request(
        "search",
        meta_access_token,
        params,
    )
    return _as_json(payload)