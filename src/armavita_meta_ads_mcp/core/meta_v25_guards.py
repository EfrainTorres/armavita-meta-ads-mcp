# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""Shared v25 validation helpers for campaigns and insights."""

from typing import Any, Dict, List, Optional

_SAC_REQUIRING_COUNTRY = frozenset(
    {
        "HOUSING",
        "EMPLOYMENT",
        "FINANCIAL_PRODUCTS_SERVICES",
        "FINANCIAL_PRODUCTS_AND_SERVICES",
        "CREDIT",
    }
)

_DEPRECATED_ATTRIBUTION_WINDOWS = frozenset({"7d_view", "28d_view"})

_DEPRECATED_ADVANTAGE_PLUS_PROMOTION_TYPES = frozenset(
    {
        "ADVANTAGE_PLUS_APP_CAMPAIGN",
        "ADVANTAGE_PLUS_SHOPPING",
        "ADVANTAGE_PLUS_SHOPPING_CAMPAIGN",
        "AUTOMATED_SHOPPING_ADS",
        "SMART_APP_PROMOTION",
    }
)

_DEPRECATED_ADVANTAGE_PLUS_STATE_MARKERS = frozenset(
    {
        "ADVANTAGE_PLUS_APP_CAMPAIGN",
        "ADVANTAGE_PLUS_SHOPPING_CAMPAIGN",
    }
)


def detect_deprecated_advantage_plus_block(campaign_data: Dict[str, Any]) -> Optional[str]:
    """Return block reason when campaign metadata matches deprecated Advantage+ signatures."""
    if not isinstance(campaign_data, dict):
        return None

    smart_promotion_type = str(campaign_data.get("smart_promotion_type", "")).strip().upper()
    objective = str(campaign_data.get("objective", "")).strip().upper()

    if smart_promotion_type in _DEPRECATED_ADVANTAGE_PLUS_PROMOTION_TYPES:
        return (
            f"Campaign uses smart_promotion_type '{smart_promotion_type}', which matches "
            "deprecated Advantage+ Shopping/App campaign flows blocked in v25."
        )

    if smart_promotion_type and "ADVANTAGE" in smart_promotion_type:
        if "SHOPPING" in smart_promotion_type or "APP" in smart_promotion_type:
            return (
                f"Campaign uses smart_promotion_type '{smart_promotion_type}', which appears "
                "to be a deprecated Advantage+ Shopping/App flow blocked in v25."
            )

    advantage_state_info = campaign_data.get("advantage_state_info")
    if isinstance(advantage_state_info, dict):
        state_tokens = set()
        for key in ("advantage_state", "campaign_type", "type", "name"):
            value = advantage_state_info.get(key)
            if isinstance(value, str):
                state_tokens.add(value.strip().upper())
        if any(token in _DEPRECATED_ADVANTAGE_PLUS_STATE_MARKERS for token in state_tokens):
            return (
                "Campaign advantage_state_info indicates a deprecated Advantage+ Shopping/App campaign "
                "that cannot be created or duplicated under v25 constraints."
            )

    if objective == "OUTCOME_APP_PROMOTION" and smart_promotion_type and "APP" in smart_promotion_type:
        return (
            f"Campaign objective '{objective}' with smart_promotion_type '{smart_promotion_type}' "
            "matches deprecated Advantage+ App restrictions in v25."
        )

    if objective == "OUTCOME_SALES" and smart_promotion_type and "SHOPPING" in smart_promotion_type:
        return (
            f"Campaign objective '{objective}' with smart_promotion_type '{smart_promotion_type}' "
            "matches deprecated Advantage+ Shopping restrictions in v25."
        )

    return None


def validate_special_ad_category_country(
    special_ad_categories: Optional[List[str]],
    special_ad_category_country: Optional[List[str]],
) -> Optional[str]:
    if not special_ad_categories:
        return None

    normalized = [str(item).strip().upper() for item in special_ad_categories if str(item).strip()]
    if not normalized:
        return None

    requires_country = any(category in _SAC_REQUIRING_COUNTRY for category in normalized)
    if not requires_country:
        return None

    countries = [str(code).strip().upper() for code in (special_ad_category_country or []) if str(code).strip()]
    if countries:
        return None

    return (
        "special_ad_category_country is required when special_ad_categories includes "
        "HOUSING, EMPLOYMENT, or FINANCIAL_PRODUCTS_SERVICES"
    )


def normalize_country_codes(values: Optional[List[str]]) -> List[str]:
    if not values:
        return []
    return list(dict.fromkeys(str(code).strip().upper() for code in values if str(code).strip()))


def deprecated_attribution_windows(input_windows: Optional[List[str]]) -> List[str]:
    if not input_windows:
        return []
    return sorted(
        {
            str(window).strip().lower()
            for window in input_windows
            if str(window).strip().lower() in _DEPRECATED_ATTRIBUTION_WINDOWS
        }
    )


def attribution_window_warning(deprecated: List[str]) -> Optional[Dict[str, Any]]:
    if not deprecated:
        return None
    return {
        "code": "deprecated_attribution_windows",
        "message": "One or more requested attribution windows are deprecated and may return empty data.",
        "deprecated_windows": deprecated,
        "recommended_windows": ["1d_click", "7d_click", "1d_view"],
    }


def append_warning(payload: Dict[str, Any], warning: Optional[Dict[str, Any]]) -> Dict[str, Any]:
    if not warning:
        return payload
    existing = payload.get("warnings")
    if isinstance(existing, list):
        existing.append(warning)
    else:
        payload["warnings"] = [warning]
    return payload
