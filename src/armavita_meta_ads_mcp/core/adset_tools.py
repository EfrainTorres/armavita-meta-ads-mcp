"""Ad set CRUD tools."""


import json
from typing import Any, Dict, List, Optional

from .graph_client import make_api_request, meta_api_tool
from .graph_constants import META_GRAPH_API_VERSION
from .mcp_runtime import mcp_server

_REQUIRED_STORE_HOSTS = ("apps.apple.com", "itunes.apple.com", "play.google.com")
_BID_STRATEGIES_REQUIRING_BID_AMOUNT = {"LOWEST_COST_WITH_BID_CAP", "COST_CAP"}
_BID_STRATEGY_MIN_ROAS = "LOWEST_COST_WITH_MIN_ROAS"


def _json(payload: Dict[str, Any]) -> str:
    return json.dumps(payload, indent=2)


def _normalize_bid_strategy(bid_strategy: Optional[str]) -> Optional[str]:
    if bid_strategy is None:
        return None
    return str(bid_strategy).strip().upper()


def _validate_promoted_object_for_app_installs(
    optimization_goal: str,
    promoted_object: Optional[Dict[str, Any]],
) -> Optional[Dict[str, Any]]:
    if optimization_goal != "APP_INSTALLS":
        return None

    if not promoted_object:
        return {
            "error": "promoted_object is required for APP_INSTALLS optimization goal",
            "details": "Mobile app campaigns must specify which app is being promoted",
            "required_fields": ["application_id", "object_store_url"],
        }

    if not isinstance(promoted_object, dict):
        return {
            "error": "promoted_object must be a dictionary",
            "example": {
                "application_id": "123456789012345",
                "object_store_url": "https://apps.apple.com/app/id123456789",
            },
        }

    if not promoted_object.get("application_id"):
        return {
            "error": "promoted_object missing required field: application_id",
            "details": "application_id is the Facebook app ID for your mobile app",
        }

    store_url = str(promoted_object.get("object_store_url", ""))
    if not store_url:
        return {
            "error": "promoted_object missing required field: object_store_url",
            "details": "object_store_url should be the App Store or Google Play URL for your app",
        }

    if not any(host in store_url for host in _REQUIRED_STORE_HOSTS):
        return {
            "error": "Invalid object_store_url format",
            "details": "URL must be from App Store (apps.apple.com) or Google Play (play.google.com)",
            "provided_url": store_url,
        }

    return None


def _validate_bid_controls(
    bid_strategy: Optional[str],
    bid_amount: Optional[int],
    bid_constraints: Optional[Dict[str, Any]],
) -> Optional[Dict[str, Any]]:
    normalized = _normalize_bid_strategy(bid_strategy)
    if normalized is None:
        return None

    if normalized == "TARGET_COST":
        return {
            "error": "bid_strategy 'TARGET_COST' is deprecated and not supported",
            "details": "Use one of: LOWEST_COST_WITHOUT_CAP, LOWEST_COST_WITH_BID_CAP, COST_CAP, LOWEST_COST_WITH_MIN_ROAS",
        }

    if normalized == "LOWEST_COST":
        return {
            "error": "'LOWEST_COST' is not a valid bid_strategy value",
            "details": f"The 'LOWEST_COST' bid strategy is not valid in Meta Ads API {META_GRAPH_API_VERSION}",
            "workaround": "Use 'LOWEST_COST_WITHOUT_CAP' instead (no bid_amount required)",
            "valid_values": [
                "LOWEST_COST_WITHOUT_CAP",
                "LOWEST_COST_WITH_BID_CAP",
                "COST_CAP",
                "LOWEST_COST_WITH_MIN_ROAS",
            ],
        }

    if normalized in _BID_STRATEGIES_REQUIRING_BID_AMOUNT and bid_amount is None:
        return {
            "error": f"bid_amount is required when using bid_strategy '{normalized}'",
            "details": f"The '{normalized}' bid strategy requires you to specify a bid amount in cents",
            "workaround": "Either provide bid_amount or use LOWEST_COST_WITHOUT_CAP",
            "example_with_bid_amount": f'{{\"bid_strategy\": \"{normalized}\", \"bid_amount\": 500}}',
            "example_without_bid_amount": '{"bid_strategy": "LOWEST_COST_WITHOUT_CAP"}',
        }

    if normalized == _BID_STRATEGY_MIN_ROAS and not bid_constraints:
        return {
            "error": "bid_constraints is required when using bid_strategy 'LOWEST_COST_WITH_MIN_ROAS'",
            "details": "Provide bid_constraints with roas_average_floor (target ROAS * 10000)",
            "example": {
                "bid_strategy": "LOWEST_COST_WITH_MIN_ROAS",
                "bid_constraints": {"roas_average_floor": 20000},
                "optimization_goal": "VALUE",
            },
        }

    return None


def _default_targeting() -> Dict[str, Any]:
    return {
        "age_min": 18,
        "age_max": 65,
        "geo_locations": {"countries": ["US"]},
        "targeting_automation": {"advantage_audience": 1},
    }


def _normalize_targeting(targeting: Optional[Dict[str, Any]]) -> Dict[str, Any]:
    if not targeting:
        return _default_targeting()

    normalized = dict(targeting)
    if "targeting_automation" not in normalized:
        normalized["targeting_automation"] = {"advantage_audience": 0}
    return normalized


async def _parent_campaign_bid_strategy(campaign_id: str, meta_access_token: str) -> Optional[str]:
    payload = await make_api_request(campaign_id, meta_access_token, {"fields": "bid_strategy"})
    if isinstance(payload, dict):
        strategy = payload.get("bid_strategy")
        if isinstance(strategy, str) and strategy:
            return strategy
    return None


@mcp_server.tool()
@meta_api_tool
async def list_ad_sets(
    ad_account_id: str,
    meta_access_token: Optional[str] = None,
    page_size: int = 10,
    campaign_id: str = "",
    page_cursor: str = "",
) -> str:
    """List ad sets under an account or campaign."""
    if not ad_account_id:
        return _json({"error": "No account ID specified"})

    target = campaign_id or ad_account_id
    endpoint = f"{target}/adsets"
    params: Dict[str, Any] = {
        "fields": (
            "id,name,campaign_id,status,daily_budget,lifetime_budget,targeting,bid_amount,bid_strategy,"
            "bid_constraints,optimization_goal,billing_event,start_time,end_time,created_time,updated_time,"
            "is_dynamic_creative,frequency_control_specs{event,interval_days,max_frequency}"
        ),
        "page_size": int(page_size),
    }
    if page_cursor:
        params["page_cursor"] = page_cursor

    payload = await make_api_request(endpoint, meta_access_token, params)
    return _json(payload)


@mcp_server.tool()
@meta_api_tool
async def read_ad_set(ad_set_id: str, meta_access_token: Optional[str] = None) -> str:
    """Fetch full details for one ad set."""
    if not ad_set_id:
        return _json({"error": "No ad set ID provided"})

    payload = await make_api_request(
        ad_set_id,
        meta_access_token,
        {
            "fields": (
                "id,name,campaign_id,status,frequency_control_specs{event,interval_days,max_frequency},"
                "daily_budget,lifetime_budget,targeting,bid_amount,bid_strategy,bid_constraints,"
                "optimization_goal,billing_event,start_time,end_time,created_time,updated_time,"
                "attribution_spec,destination_type,promoted_object,pacing_type,budget_remaining,"
                "dsa_beneficiary,is_dynamic_creative"
            )
        },
    )

    if isinstance(payload, dict) and "frequency_control_specs" not in payload:
        payload["_meta"] = {
            "note": "No frequency_control_specs were returned. Either none are set or the API omitted the field."
        }

    return _json(payload)


@mcp_server.tool()
@meta_api_tool
async def create_ad_set(
    ad_account_id: str,
    campaign_id: str,
    name: str,
    optimization_goal: str,
    billing_event: str,
    status: str = "PAUSED",
    daily_budget: Optional[int] = None,
    lifetime_budget: Optional[int] = None,
    targeting: Optional[Dict[str, Any]] = None,
    bid_amount: Optional[int] = None,
    bid_strategy: Optional[str] = None,
    bid_constraints: Optional[Dict[str, Any]] = None,
    start_time: Optional[str] = None,
    end_time: Optional[str] = None,
    dsa_beneficiary: Optional[str] = None,
    promoted_object: Optional[Dict[str, Any]] = None,
    destination_type: Optional[str] = None,
    is_dynamic_creative: Optional[bool] = None,
    meta_access_token: Optional[str] = None,
) -> str:
    """Create an ad set under a campaign."""
    if not ad_account_id:
        return _json({"error": "No account ID provided"})
    if not campaign_id:
        return _json({"error": "No campaign ID provided"})
    if not name:
        return _json({"error": "No ad set name provided"})
    if not optimization_goal:
        return _json({"error": "No optimization goal provided"})
    if not billing_event:
        return _json({"error": "No billing event provided"})

    app_error = _validate_promoted_object_for_app_installs(optimization_goal, promoted_object)
    if app_error:
        return _json(app_error)

    bid_error = _validate_bid_controls(bid_strategy, bid_amount, bid_constraints)
    if bid_error:
        return _json(bid_error)

    normalized_bid_strategy = _normalize_bid_strategy(bid_strategy)

    if bid_amount is None:
        try:
            parent_strategy = await _parent_campaign_bid_strategy(campaign_id, meta_access_token)
        except Exception:  # noqa: BLE001
            parent_strategy = None

        if parent_strategy in (_BID_STRATEGIES_REQUIRING_BID_AMOUNT | {"TARGET_COST"}):
            return _json(
                {
                    "error": (
                        f"bid_amount is required because the parent campaign uses bid_strategy "
                        f"'{parent_strategy}'"
                    ),
                    "details": "Provide bid_amount in cents or update parent campaign strategy.",
                    "example_with_bid_amount": {"bid_amount": 500},
                }
            )

    payload: Dict[str, Any] = {
        "name": name,
        "campaign_id": campaign_id,
        "status": status,
        "optimization_goal": optimization_goal,
        "billing_event": billing_event,
        "targeting": json.dumps(_normalize_targeting(targeting)),
    }

    if daily_budget is not None:
        payload["daily_budget"] = str(daily_budget)
    if lifetime_budget is not None:
        payload["lifetime_budget"] = str(lifetime_budget)
    if bid_amount is not None:
        payload["bid_amount"] = str(bid_amount)
    if normalized_bid_strategy is not None:
        payload["bid_strategy"] = normalized_bid_strategy
    if bid_constraints is not None:
        payload["bid_constraints"] = json.dumps(bid_constraints)
    if start_time:
        payload["start_time"] = start_time
    if end_time:
        payload["end_time"] = end_time
    if dsa_beneficiary:
        payload["dsa_beneficiary"] = dsa_beneficiary
    if promoted_object is not None:
        payload["promoted_object"] = json.dumps(promoted_object)
    if destination_type is not None:
        payload["destination_type"] = destination_type
    if is_dynamic_creative is not None:
        payload["is_dynamic_creative"] = "true" if bool(is_dynamic_creative) else "false"

    result = await make_api_request(f"{ad_account_id}/adsets", meta_access_token, payload, method="POST")

    if isinstance(result, dict) and result.get("error"):
        rendered_error = json.dumps(result.get("error", {}), default=str).lower()
        if "permission" in rendered_error or "insufficient" in rendered_error:
            return _json(
                {
                    "error": "Insufficient permissions to set DSA beneficiary. Please ensure business_management permissions.",
                    "details": result,
                    "permission_required": True,
                }
            )
        if "dsa_beneficiary" in rendered_error and ("not supported" in rendered_error or "parameter" in rendered_error):
            return _json(
                {
                    "error": "DSA beneficiary parameter not supported in this API version.",
                    "details": result,
                    "manual_setup_required": True,
                }
            )
        if "benefits from ads" in rendered_error or "dsa beneficiary" in rendered_error:
            return _json(
                {
                    "error": "DSA beneficiary required for European compliance.",
                    "details": result,
                    "dsa_required": True,
                }
            )

    return _json(result)


@mcp_server.tool()
@meta_api_tool
async def update_ad_set(
    ad_set_id: str,
    frequency_control_specs: Optional[List[Dict[str, Any]]] = None,
    bid_strategy: Optional[str] = None,
    bid_amount: Optional[int] = None,
    bid_constraints: Optional[Dict[str, Any]] = None,
    status: Optional[str] = None,
    targeting: Optional[Dict[str, Any]] = None,
    optimization_goal: Optional[str] = None,
    daily_budget: Optional[int] = None,
    lifetime_budget: Optional[int] = None,
    is_dynamic_creative: Optional[bool] = None,
    meta_access_token: Optional[str] = None,
) -> str:
    """Update an ad set's delivery, budgeting, and targeting configuration."""
    if not ad_set_id:
        return _json({"error": "No ad set ID provided"})

    bid_error = _validate_bid_controls(bid_strategy, bid_amount, bid_constraints)
    if bid_error:
        return _json(bid_error)

    payload: Dict[str, Any] = {}

    if frequency_control_specs is not None:
        payload["frequency_control_specs"] = frequency_control_specs

    normalized_bid_strategy = _normalize_bid_strategy(bid_strategy)
    if normalized_bid_strategy is not None:
        payload["bid_strategy"] = normalized_bid_strategy

    if bid_amount is not None:
        payload["bid_amount"] = str(bid_amount)
    if bid_constraints is not None:
        payload["bid_constraints"] = json.dumps(bid_constraints)
    if status is not None:
        payload["status"] = status
    if targeting is not None:
        payload["targeting"] = json.dumps(targeting)
    if optimization_goal is not None:
        payload["optimization_goal"] = optimization_goal
    if daily_budget is not None:
        payload["daily_budget"] = str(daily_budget)
    if lifetime_budget is not None:
        payload["lifetime_budget"] = str(lifetime_budget)
    if is_dynamic_creative is not None:
        payload["is_dynamic_creative"] = "true" if bool(is_dynamic_creative) else "false"

    if not payload:
        return _json({"error": "No update parameters provided"})

    try:
        result = await make_api_request(ad_set_id, meta_access_token, payload, method="POST")
        return _json(result)
    except Exception as exc:  # noqa: BLE001
        return _json(
            {
                "error": f"Failed to update ad set {ad_set_id}",
                "details": str(exc),
                "params_sent": payload,
            }
        )
