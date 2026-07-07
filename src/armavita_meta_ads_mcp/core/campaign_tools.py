# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""Campaign CRUD helpers."""


import json
from typing import Any, Dict, List, Optional, Tuple, Union

from .graph_client import make_api_request, meta_api_tool
from .meta_v25_guards import (
    detect_deprecated_advantage_plus_block,
    normalize_country_codes,
    validate_special_ad_category_country,
)
from .mcp_runtime import mcp_server
from mcp.types import ToolAnnotations

_DEPRECATED_SPECIAL_AD_CATEGORIES = {
    "CREDIT": "FINANCIAL_PRODUCTS_SERVICES",
}

_CAMPAIGN_LIST_FIELDS = (
    "id,name,objective,status,effective_status,daily_budget,lifetime_budget,buying_type,"
    "start_time,stop_time,created_time,updated_time,bid_strategy,advantage_state_info,"
    "special_ad_categories,special_ad_category_country,is_adset_budget_sharing_enabled,spend_cap"
)

_CAMPAIGN_READ_FIELDS = (
    "id,name,objective,status,effective_status,daily_budget,lifetime_budget,buying_type,"
    "start_time,stop_time,created_time,updated_time,bid_strategy,special_ad_categories,"
    "special_ad_category_country,budget_remaining,configured_status,advantage_state_info,"
    "is_adset_budget_sharing_enabled,spend_cap,smart_promotion_type"
)


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


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=True))
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
        "fields": _CAMPAIGN_LIST_FIELDS,
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


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=True))
@meta_api_tool
async def read_campaign(campaign_id: str, meta_access_token: Optional[str] = None) -> str:
    """Fetch detailed campaign metadata."""
    if not campaign_id:
        return _json({"error": "No campaign ID provided"})

    payload = await make_api_request(
        campaign_id,
        meta_access_token,
        {"fields": _CAMPAIGN_READ_FIELDS},
    )
    return _json(payload)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=False, destructiveHint=False, idempotentHint=False, openWorldHint=True))
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
    is_adset_budget_sharing_enabled: Optional[bool] = None,
    special_ad_category_country: Optional[List[str]] = None,
    apply_default_budget: bool = False,
    bid_constraints: Optional[Dict[str, Any]] = None,
    smart_promotion_type: Optional[str] = None,
) -> str:
    """Create a campaign with optional budgeting and bid controls.

    Budgeting: pass `daily_budget`/`lifetime_budget` for campaign-level budget, or set
    `use_ad_set_level_budgets=True` for ad-set budgets. If neither is provided, the
    request fails unless `apply_default_budget=True` (applies `daily_budget=1000`).

    For ad-set-level budgets, `is_adset_budget_sharing_enabled` defaults to `True`
    (Meta's recommendation in v24.0+). Override via `is_adset_budget_sharing_enabled=False`.
    """
    if not ad_account_id:
        return _json({"error": "No account ID provided"})
    if not name:
        return _json({"error": "No campaign name provided"})
    if not objective:
        return _json({"error": "No campaign objective provided"})

    categories, category_error = _normalize_special_ad_categories(special_ad_categories)
    if category_error:
        return _json({"error": category_error})

    country_error = validate_special_ad_category_country(categories, special_ad_category_country)
    if country_error:
        return _json({"error": country_error})

    bid_error = _validate_bid_strategy(bid_strategy)
    if bid_error:
        return _json(bid_error)

    advantage_plus_block = detect_deprecated_advantage_plus_block(
        {
            "objective": objective,
            "smart_promotion_type": smart_promotion_type or "",
        }
    )
    if advantage_plus_block:
        return _json(
            {
                "error": "Deprecated Advantage+ campaign type",
                "details": advantage_plus_block,
                "hint": "Use current Advantage+ campaign creation flows documented for Graph API v25.",
            }
        )

    if (
        not use_ad_set_level_budgets
        and daily_budget is None
        and lifetime_budget is None
        and not apply_default_budget
    ):
        return _json(
            {
                "error": "Campaign budget required",
                "details": (
                    "Provide daily_budget or lifetime_budget for campaign-level budgeting, "
                    "set use_ad_set_level_budgets=True for ad-set budgets, or pass "
                    "apply_default_budget=True to apply daily_budget=1000."
                ),
            }
        )

    auto_budget_applied = False
    if (
        not use_ad_set_level_budgets
        and daily_budget is None
        and lifetime_budget is None
        and apply_default_budget
    ):
        daily_budget = 1000
        auto_budget_applied = True

    payload: Dict[str, Any] = {
        "name": name,
        "objective": objective,
        "status": status,
        "special_ad_categories": json.dumps(categories),
    }

    countries = normalize_country_codes(special_ad_category_country)
    if countries:
        payload["special_ad_category_country"] = json.dumps(countries)

    if use_ad_set_level_budgets:
        # Required field for ad-set-level budgets (v24.0+); Meta recommends `true`
        # to enable cross-adset budget sharing optimization.
        share_default = True if is_adset_budget_sharing_enabled is None else is_adset_budget_sharing_enabled
        payload["is_adset_budget_sharing_enabled"] = "true" if share_default else "false"
    else:
        if daily_budget is not None:
            payload["daily_budget"] = str(daily_budget)
        if lifetime_budget is not None:
            payload["lifetime_budget"] = str(lifetime_budget)

    # v25: `campaign_budget_optimization` and `bid_cap` are NOT writable Campaign
    # (ad-campaign-group) fields. CBO is implied by the presence of a campaign-level
    # daily/lifetime budget, and the bid-cap amount is the ad-set `bid_amount`.
    # Sending them is silently ignored by Graph, so we drop them and warn instead.
    ignored_warnings: List[Dict[str, Any]] = []
    if campaign_budget_optimization is not None:
        ignored_warnings.append({
            "code": "campaign_budget_optimization_not_a_field",
            "message": (
                "campaign_budget_optimization is not a writable Campaign field. CBO is enabled by "
                "setting a campaign-level daily_budget/lifetime_budget; using ad-set budgets disables it."
            ),
        })
    if bid_cap is not None:
        ignored_warnings.append({
            "code": "bid_cap_not_a_campaign_field",
            "message": (
                "bid_cap is not a Campaign field. Set the cap amount as bid_amount on the ad set "
                "(create_ad_set/update_ad_set) when bid_strategy is LOWEST_COST_WITH_BID_CAP or COST_CAP."
            ),
        })

    if buying_type is not None:
        payload["buying_type"] = buying_type
    if bid_strategy is not None:
        payload["bid_strategy"] = str(bid_strategy).strip().upper()
    if spend_cap is not None:
        payload["spend_cap"] = str(spend_cap)
    if bid_constraints is not None:
        payload["bid_constraints"] = json.dumps(bid_constraints)
    if ab_test_control_setups:
        payload["ab_test_control_setups"] = ab_test_control_setups
    if smart_promotion_type:
        payload["smart_promotion_type"] = str(smart_promotion_type).strip()

    result = await make_api_request(f"{ad_account_id}/campaigns", meta_access_token, payload, method="POST")

    if isinstance(result, dict):
        if use_ad_set_level_budgets:
            result["budget_strategy"] = "ad_set_level"
            result["note"] = "Campaign created with ad set level budgets. Set budgets on child ad sets."
        elif auto_budget_applied:
            result["budget_default_applied"] = "daily_budget=1000"
            result["note"] = "No campaign budget provided, so MCP applied daily_budget=1000."
        if ignored_warnings:
            existing = result.get("warnings")
            result["warnings"] = (existing + ignored_warnings) if isinstance(existing, list) else ignored_warnings

    return _json(result)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=False, destructiveHint=False, idempotentHint=True, openWorldHint=True))
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
    is_adset_budget_sharing_enabled: Optional[bool] = None,
    special_ad_category_country: Optional[List[str]] = None,
    migrate_to_advantage_plus: Optional[bool] = None,
    bid_constraints: Optional[Dict[str, Any]] = None,
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
        country_error = validate_special_ad_category_country(categories, special_ad_category_country)
        if country_error:
            return _json({"error": country_error})
        payload["special_ad_categories"] = json.dumps(categories)

    if special_ad_category_country is not None:
        payload["special_ad_category_country"] = json.dumps(normalize_country_codes(special_ad_category_country))

    if use_ad_set_level_budgets is True:
        # Clearing the campaign budget is what actually disables CBO; there is no
        # writable `campaign_budget_optimization` field on the Campaign node.
        payload["daily_budget"] = ""
        payload["lifetime_budget"] = ""
        share_default = True if is_adset_budget_sharing_enabled is None else is_adset_budget_sharing_enabled
        payload["is_adset_budget_sharing_enabled"] = "true" if share_default else "false"
    elif is_adset_budget_sharing_enabled is not None:
        payload["is_adset_budget_sharing_enabled"] = "true" if is_adset_budget_sharing_enabled else "false"

    if use_ad_set_level_budgets is not True:
        if daily_budget is not None:
            payload["daily_budget"] = "" if daily_budget == "" else str(daily_budget)
        if lifetime_budget is not None:
            payload["lifetime_budget"] = "" if lifetime_budget == "" else str(lifetime_budget)

    # v25: these are not writable Campaign fields (see create_campaign); drop + warn.
    ignored_warnings: List[Dict[str, Any]] = []
    if campaign_budget_optimization is not None:
        ignored_warnings.append({
            "code": "campaign_budget_optimization_not_a_field",
            "message": (
                "campaign_budget_optimization is not a writable Campaign field. CBO follows the "
                "presence of a campaign-level budget; set use_ad_set_level_budgets=True to disable it."
            ),
        })
    if bid_cap is not None:
        ignored_warnings.append({
            "code": "bid_cap_not_a_campaign_field",
            "message": "bid_cap is not a Campaign field. Set bid_amount on the ad set instead.",
        })

    if bid_strategy is not None:
        payload["bid_strategy"] = str(bid_strategy).strip().upper()
    if spend_cap is not None:
        payload["spend_cap"] = str(spend_cap)
    if objective is not None:
        payload["objective"] = objective
    if migrate_to_advantage_plus is not None:
        payload["migrate_to_advantage_plus"] = "true" if migrate_to_advantage_plus else "false"
    if bid_constraints is not None:
        payload["bid_constraints"] = json.dumps(bid_constraints)

    if not payload:
        # Nothing writable to send. If the caller only passed non-writable fields
        # (campaign_budget_optimization/bid_cap), surface the guidance warnings
        # instead of a bare generic error.
        if ignored_warnings:
            # Dict error (not a bare string) so the tool envelope keeps `warnings`
            # at the top level instead of burying it under {"data": ...}.
            return _json({
                "error": {"message": "No writable Campaign fields were provided"},
                "warnings": ignored_warnings,
            })
        return _json({"error": "No update parameters provided"})

    result = await make_api_request(campaign_id, meta_access_token, payload, method="POST")

    if isinstance(result, dict):
        if use_ad_set_level_budgets is True:
            result["budget_strategy"] = "ad_set_level"
            result["note"] = "Campaign updated to use ad set level budgets."
        if ignored_warnings:
            existing = result.get("warnings")
            result["warnings"] = (existing + ignored_warnings) if isinstance(existing, list) else ignored_warnings

    return _json(result)
