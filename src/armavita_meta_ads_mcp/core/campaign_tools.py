"""Campaign CRUD helpers."""


import json
from typing import Any, Dict, List, Optional, Tuple, Union

from .graph_client import make_api_request, meta_api_tool
from .mcp_runtime import mcp_server

_DEPRECATED_SPECIAL_AD_CATEGORIES = {
    "CREDIT": "FINANCIAL_PRODUCTS_SERVICES",
}


def _json(payload: Dict[str, Any]) -> str:
    return json.dumps(payload, indent=2)


def _normalize_special_ad_categories(values: Optional[List[str]]) -> Tuple[Optional[List[str]], Optional[str]]:
    if values is None:
        return [], None
    if not isinstance(values, list):
        return None, "special_ad_categories must be a list of strings"

    normalized = [str(item).strip().upper() for item in values if str(item).strip()]
    if not normalized:
        return [], None

    for value in normalized:
        if value in _DEPRECATED_SPECIAL_AD_CATEGORIES:
            replacement = _DEPRECATED_SPECIAL_AD_CATEGORIES[value]
            return None, f"special_ad_categories value '{value}' is deprecated and rejected. Use '{replacement}' instead."

    deduped = list(dict.fromkeys(normalized))
    if "NONE" in deduped:
        if len(deduped) > 1:
            return None, "special_ad_categories cannot mix 'NONE' with other categories"
        return [], None

    return deduped, None


def _validate_bid_strategy(bid_strategy: Optional[str]) -> Optional[Dict[str, Any]]:
    if str(bid_strategy or "").strip().upper() == "TARGET_COST":
        return {
            "error": "bid_strategy 'TARGET_COST' is deprecated and not supported",
            "details": "Use one of: LOWEST_COST_WITHOUT_CAP, LOWEST_COST_WITH_BID_CAP, COST_CAP, LOWEST_COST_WITH_MIN_ROAS",
            "replacement_examples": [
                {"bid_strategy": "LOWEST_COST_WITHOUT_CAP"},
                {"bid_strategy": "LOWEST_COST_WITH_BID_CAP", "bid_cap": 500},
                {"bid_strategy": "COST_CAP", "bid_cap": 500},
                {
                    "bid_strategy": "LOWEST_COST_WITH_MIN_ROAS",
                    "bid_constraints": {"roas_average_floor": 20000},
                },
            ],
        }
    return None


def _normalize_objectives(objective_filter: Union[str, List[str]]) -> List[str]:
    if isinstance(objective_filter, list):
        values = objective_filter
    else:
        values = [objective_filter]

    return [str(value).strip() for value in values if str(value).strip()]


@mcp_server.tool()
@meta_api_tool
async def list_campaigns(
    ad_account_id: str,
    meta_access_token: Optional[str] = None,
    page_size: int = 10,
    status_filter: str = "",
    objective_filter: Union[str, List[str]] = "",
    page_cursor: str = "",
) -> str:
    """List campaigns for an ad account with optional filters."""
    if not ad_account_id:
        return _json({"error": "No account ID specified"})

    params: Dict[str, Any] = {
        "fields": (
            "id,name,objective,status,effective_status,daily_budget,lifetime_budget,buying_type,"
            "start_time,stop_time,created_time,updated_time,bid_strategy,advantage_state_info"
        ),
        "page_size": int(page_size),
    }

    if status_filter:
        params["effective_status"] = [status_filter]

    objectives = _normalize_objectives(objective_filter)
    if objectives:
        params["filtering"] = [{"field": "objective", "operator": "IN", "value": objectives}]

    if page_cursor:
        params["page_cursor"] = page_cursor

    payload = await make_api_request(f"{ad_account_id}/campaigns", meta_access_token, params)
    return _json(payload)


@mcp_server.tool()
@meta_api_tool
async def read_campaign(campaign_id: str, meta_access_token: Optional[str] = None) -> str:
    """Fetch detailed campaign metadata."""
    if not campaign_id:
        return _json({"error": "No campaign ID provided"})

    payload = await make_api_request(
        campaign_id,
        meta_access_token,
        {
            "fields": (
                "id,name,objective,status,effective_status,daily_budget,lifetime_budget,buying_type,"
                "start_time,stop_time,created_time,updated_time,bid_strategy,special_ad_categories,"
                "special_ad_category_country,budget_remaining,configured_status,advantage_state_info"
            )
        },
    )
    return _json(payload)


@mcp_server.tool()
@meta_api_tool
async def create_campaign(
    ad_account_id: str,
    name: str,
    objective: str,
    meta_access_token: Optional[str] = None,
    status: str = "PAUSED",
    special_ad_categories: Optional[List[str]] = None,
    daily_budget: Optional[int] = None,
    lifetime_budget: Optional[int] = None,
    buying_type: Optional[str] = None,
    bid_strategy: Optional[str] = None,
    bid_cap: Optional[int] = None,
    spend_cap: Optional[int] = None,
    campaign_budget_optimization: Optional[bool] = None,
    ab_test_control_setups: Optional[List[Dict[str, Any]]] = None,
    use_ad_set_level_budgets: bool = False,
) -> str:
    """Create a campaign with optional budgeting and bid controls."""
    if not ad_account_id:
        return _json({"error": "No account ID provided"})
    if not name:
        return _json({"error": "No campaign name provided"})
    if not objective:
        return _json({"error": "No campaign objective provided"})

    categories, category_error = _normalize_special_ad_categories(special_ad_categories)
    if category_error:
        return _json({"error": category_error})

    bid_error = _validate_bid_strategy(bid_strategy)
    if bid_error:
        return _json(bid_error)

    auto_budget_applied = False
    if not use_ad_set_level_budgets and daily_budget is None and lifetime_budget is None:
        daily_budget = 1000
        auto_budget_applied = True

    payload: Dict[str, Any] = {
        "name": name,
        "objective": objective,
        "status": status,
        "special_ad_categories": json.dumps(categories),
    }

    if use_ad_set_level_budgets:
        payload["is_adset_budget_sharing_enabled"] = "false"
    else:
        if daily_budget is not None:
            payload["daily_budget"] = str(daily_budget)
        if lifetime_budget is not None:
            payload["lifetime_budget"] = str(lifetime_budget)
        if campaign_budget_optimization is not None:
            payload["campaign_budget_optimization"] = "true" if campaign_budget_optimization else "false"

    if buying_type is not None:
        payload["buying_type"] = buying_type
    if bid_strategy is not None:
        payload["bid_strategy"] = str(bid_strategy).strip().upper()
    if bid_cap is not None:
        payload["bid_cap"] = str(bid_cap)
    if spend_cap is not None:
        payload["spend_cap"] = str(spend_cap)
    if ab_test_control_setups:
        payload["ab_test_control_setups"] = ab_test_control_setups

    result = await make_api_request(f"{ad_account_id}/campaigns", meta_access_token, payload, method="POST")

    if isinstance(result, dict):
        if use_ad_set_level_budgets:
            result["budget_strategy"] = "ad_set_level"
            result["note"] = "Campaign created with ad set level budgets. Set budgets on child ad sets."
        elif auto_budget_applied:
            result["budget_default_applied"] = "daily_budget=1000"
            result["note"] = "No campaign budget provided, so MCP applied daily_budget=1000."

    return _json(result)


@mcp_server.tool()
@meta_api_tool
async def update_campaign(
    campaign_id: str,
    meta_access_token: Optional[str] = None,
    name: Optional[str] = None,
    status: Optional[str] = None,
    special_ad_categories: Optional[List[str]] = None,
    daily_budget: Optional[int] = None,
    lifetime_budget: Optional[int] = None,
    bid_strategy: Optional[str] = None,
    bid_cap: Optional[int] = None,
    spend_cap: Optional[int] = None,
    campaign_budget_optimization: Optional[bool] = None,
    objective: Optional[str] = None,
    use_ad_set_level_budgets: Optional[bool] = None,
) -> str:
    """Update campaign settings for an existing campaign."""
    if not campaign_id:
        return _json({"error": "No campaign ID provided"})

    bid_error = _validate_bid_strategy(bid_strategy)
    if bid_error:
        return _json(bid_error)

    payload: Dict[str, Any] = {}

    if name is not None:
        payload["name"] = name
    if status is not None:
        payload["status"] = status

    if special_ad_categories is not None:
        categories, category_error = _normalize_special_ad_categories(special_ad_categories)
        if category_error:
            return _json({"error": category_error})
        payload["special_ad_categories"] = json.dumps(categories)

    if use_ad_set_level_budgets is True:
        payload["daily_budget"] = ""
        payload["lifetime_budget"] = ""
        if campaign_budget_optimization is not None:
            payload["campaign_budget_optimization"] = "false"
    else:
        if daily_budget is not None:
            payload["daily_budget"] = "" if daily_budget == "" else str(daily_budget)
        if lifetime_budget is not None:
            payload["lifetime_budget"] = "" if lifetime_budget == "" else str(lifetime_budget)
        if campaign_budget_optimization is not None:
            payload["campaign_budget_optimization"] = "true" if campaign_budget_optimization else "false"

    if bid_strategy is not None:
        payload["bid_strategy"] = str(bid_strategy).strip().upper()
    if bid_cap is not None:
        payload["bid_cap"] = str(bid_cap)
    if spend_cap is not None:
        payload["spend_cap"] = str(spend_cap)
    if objective is not None:
        payload["objective"] = objective

    if not payload:
        return _json({"error": "No update parameters provided"})

    result = await make_api_request(campaign_id, meta_access_token, payload, method="POST")

    if isinstance(result, dict) and use_ad_set_level_budgets is True:
        result["budget_strategy"] = "ad_set_level"
        result["note"] = "Campaign updated to use ad set level budgets."

    return _json(result)
