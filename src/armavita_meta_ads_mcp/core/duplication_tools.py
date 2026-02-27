"""OSS-local duplication tools for Meta Ads API."""

import json
import logging
from typing import Any, Dict, List, Optional, Tuple

from . import auth_state as auth
from .graph_client import McpToolError, make_api_request, meta_api_tool
from .mcp_runtime import mcp_server

logger = logging.getLogger(__name__)


# Preserve exception class identity across importlib.reload() in tests.
if "RateLimitError" not in globals():
    class RateLimitError(McpToolError):
        """Raised on rate-page_size errors so FastMCP sets isError: true."""


if "DuplicationError" not in globals():
    class DuplicationError(McpToolError):
        """Raised on non-success duplication responses."""


_DEPRECATED_ADVANTAGE_PLUS_PROMOTION_TYPES = {
    "ADVANTAGE_PLUS_APP_CAMPAIGN",
    "ADVANTAGE_PLUS_SHOPPING",
    "ADVANTAGE_PLUS_SHOPPING_CAMPAIGN",
    "AUTOMATED_SHOPPING_ADS",
    "SMART_APP_PROMOTION",
}

_DEPRECATED_ADVANTAGE_PLUS_STATE_MARKERS = {
    "ADVANTAGE_PLUS_APP_CAMPAIGN",
    "ADVANTAGE_PLUS_SHOPPING_CAMPAIGN",
}

def _detect_deprecated_advantage_plus_block(campaign_data: Dict[str, Any]) -> Optional[str]:
    """Return block reason when campaign metadata matches deprecated Advantage+ signatures."""
    if not isinstance(campaign_data, dict):
        return None

    smart_promotion_type = str(campaign_data.get("smart_promotion_type", "")).strip().upper()
    objective = str(campaign_data.get("objective", "")).strip().upper()

    if smart_promotion_type in _DEPRECATED_ADVANTAGE_PLUS_PROMOTION_TYPES:
        return (
            f"Campaign uses smart_promotion_type '{smart_promotion_type}', which matches "
            "deprecated Advantage+ Shopping/App campaign flows blocked in v25 duplication."
        )

    if smart_promotion_type and "ADVANTAGE" in smart_promotion_type:
        if "SHOPPING" in smart_promotion_type or "APP" in smart_promotion_type:
            return (
                f"Campaign uses smart_promotion_type '{smart_promotion_type}', which appears "
                "to be a deprecated Advantage+ Shopping/App flow blocked in v25 duplication."
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
                "that cannot be duplicated under v25 constraints."
            )

    if objective == "OUTCOME_APP_PROMOTION" and smart_promotion_type and "APP" in smart_promotion_type:
        return (
            f"Campaign objective '{objective}' with smart_promotion_type '{smart_promotion_type}' "
            "matches deprecated Advantage+ App duplication restrictions in v25."
        )

    if objective == "OUTCOME_SALES" and smart_promotion_type and "SHOPPING" in smart_promotion_type:
        return (
            f"Campaign objective '{objective}' with smart_promotion_type '{smart_promotion_type}' "
            "matches deprecated Advantage+ Shopping duplication restrictions in v25."
        )

    return None


async def _resolve_campaign_id_for_resource(
    resource_type: str,
    resource_id: str,
    meta_access_token: str,
) -> Optional[str]:
    """Resolve parent campaign ID for campaign/adset/ad duplication resources."""
    if resource_type == "campaign":
        return resource_id

    if resource_type == "adset":
        adset_data = await make_api_request(resource_id, meta_access_token, {"fields": "campaign_id"})
        campaign_id = adset_data.get("campaign_id") if isinstance(adset_data, dict) else None
        return str(campaign_id) if campaign_id else None

    if resource_type == "ad":
        ad_data = await make_api_request(resource_id, meta_access_token, {"fields": "campaign_id,adset{campaign_id}"})
        if not isinstance(ad_data, dict):
            return None
        campaign_id = ad_data.get("campaign_id")
        if campaign_id:
            return str(campaign_id)
        adset_obj = ad_data.get("adset")
        if isinstance(adset_obj, dict) and adset_obj.get("campaign_id"):
            return str(adset_obj.get("campaign_id"))

    return None


async def _run_v25_duplication_preflight(
    resource_type: str,
    resource_id: str,
    meta_access_token: str,
) -> Optional[Dict[str, Any]]:
    """Return block payload when duplication targets deprecated Advantage+ campaign types."""
    if not isinstance(meta_access_token, str) or not meta_access_token.startswith("EA"):
        return None

    if resource_type not in {"campaign", "adset", "ad"}:
        return None

    try:
        campaign_id = await _resolve_campaign_id_for_resource(resource_type, resource_id, meta_access_token)
        if not campaign_id:
            return None

        campaign_data = await make_api_request(
            campaign_id,
            meta_access_token,
            {"fields": "id,name,objective,smart_promotion_type,advantage_state_info"},
        )
        if not isinstance(campaign_data, dict) or "error" in campaign_data:
            return None

        reason = _detect_deprecated_advantage_plus_block(campaign_data)
        if not reason:
            return None

        return {
            "campaign_id": campaign_id,
            "campaign_name": campaign_data.get("name"),
            "campaign_objective": campaign_data.get("objective"),
            "smart_promotion_type": campaign_data.get("smart_promotion_type"),
            "advantage_state_info": campaign_data.get("advantage_state_info"),
            "reason": reason,
        }
    except Exception as exc:
        logger.warning("v25 duplication preflight skipped due to lookup failure: %s", exc)
        return None


def _append_warning(warnings: List[Dict[str, Any]], option: str, reason: str) -> None:
    warnings.append({"option": option, "reason": reason})


def _build_copy_params(resource_type: str, options: Dict[str, Any]) -> Tuple[Dict[str, Any], List[Dict[str, Any]]]:
    """Map tool options to Meta copy edge params and warnings for unsupported overrides."""
    params: Dict[str, Any] = {}
    warnings: List[Dict[str, Any]] = []

    name_suffix = options.get("name_suffix")
    if name_suffix:
        params["rename_options"] = {
            "rename_strategy": "DEEP_RENAME",
            "rename_suffix": str(name_suffix),
        }

    new_status = options.get("new_status")
    if new_status:
        params["status_option"] = str(new_status).upper()

    if resource_type == "campaign":
        params["deep_copy"] = True

        if options.get("include_ad_sets") is False:
            _append_warning(
                warnings,
                "include_ad_sets",
                "Meta's campaign copy edge deep-copies hierarchy; selective exclusion is not supported.",
            )
        if options.get("include_ads") is False:
            _append_warning(
                warnings,
                "include_ads",
                "Meta's campaign copy edge deep-copies hierarchy; selective exclusion is not supported.",
            )
        if options.get("include_creatives") is False:
            _append_warning(
                warnings,
                "include_creatives",
                "Meta's campaign copy edge deep-copies hierarchy; selective exclusion is not supported.",
            )
        if options.get("copy_schedule") is not None:
            _append_warning(
                warnings,
                "copy_schedule",
                "Schedule-copy override is not exposed by local copy edge; source behavior is used.",
            )
        if options.get("new_daily_budget") is not None:
            _append_warning(
                warnings,
                "new_daily_budget",
                "Budget override is not applied during copy; update the duplicated object after creation.",
            )

    elif resource_type == "adset":
        params["deep_copy"] = bool(options.get("include_ads", True) or options.get("include_creatives", True))

        target_campaign_id = options.get("target_campaign_id")
        if target_campaign_id:
            params["campaign_id"] = target_campaign_id

        if options.get("include_ads") is False:
            _append_warning(
                warnings,
                "include_ads",
                "Ad-set copy edge does not support omitting child ads during copy.",
            )
        if options.get("include_creatives") is False:
            _append_warning(
                warnings,
                "include_creatives",
                "Ad-set copy edge does not support selective creative exclusion.",
            )
        if options.get("new_daily_budget") is not None:
            _append_warning(
                warnings,
                "new_daily_budget",
                "Budget override is not applied during copy; update the duplicated object after creation.",
            )
        if options.get("new_targeting") is not None:
            _append_warning(
                warnings,
                "new_targeting",
                "Targeting override is not applied during copy; update the duplicated object after creation.",
            )

    elif resource_type == "ad":
        params["deep_copy"] = bool(options.get("clone_ad_creative", True))

        target_ad_set_id = options.get("target_ad_set_id")
        if target_ad_set_id:
            params["adset_id"] = target_ad_set_id

        if options.get("new_creative_name") is not None:
            _append_warning(
                warnings,
                "new_creative_name",
                "Creative rename override is not applied during copy; update duplicated creative after creation.",
            )

    elif resource_type == "creative":
        params["deep_copy"] = True

        for option_name in (
            "new_primary_text",
            "new_headline",
            "new_description",
            "new_cta_type",
            "new_destination_url",
        ):
            if options.get(option_name) is not None:
                _append_warning(
                    warnings,
                    option_name,
                    "Content overrides are not applied during creative copy; update duplicated creative after creation.",
                )

    return params, warnings


def _extract_new_id(resource_type: str, response: Dict[str, Any]) -> Optional[str]:
    """Extract duplicated object ID from diverse Graph copy response shapes."""
    preferred_keys = {
        "campaign": ("copied_campaign_id", "campaign_id", "id"),
        "adset": ("copied_adset_id", "adset_id", "id"),
        "ad": ("copied_ad_id", "ad_id", "id"),
        "creative": ("copied_adcreative_id", "creative_id", "id"),
    }

    for key in preferred_keys.get(resource_type, ()):
        value = response.get(key)
        if value:
            return str(value)

    if isinstance(response.get("data"), dict):
        nested = response["data"]
        for key in preferred_keys.get(resource_type, ()):
            value = nested.get(key)
            if value:
                return str(value)

    if isinstance(response.get("data"), list) and response["data"]:
        first = response["data"][0]
        if isinstance(first, dict):
            for key in preferred_keys.get(resource_type, ()):
                value = first.get(key)
                if value:
                    return str(value)

    return None


def _build_graph_error_payload(
    resource_type: str,
    resource_id: str,
    graph_error: Dict[str, Any],
) -> Dict[str, Any]:
    """Normalize Graph API error details into a deterministic duplication error envelope."""
    payload: Dict[str, Any] = {
        "success": False,
        "error": "duplication_failed",
        "message": graph_error.get("message") or graph_error.get("primary_text") or f"Failed to duplicate {resource_type}",
        "details": {
            "resource_type": resource_type,
            "resource_id": resource_id,
            "code": graph_error.get("code"),
        },
    }

    for field in ("error_subcode", "error_user_title", "error_user_msg", "type", "fbtrace_id"):
        if graph_error.get(field) is not None:
            payload[field] = graph_error.get(field)

    return payload


@mcp_server.tool()
@meta_api_tool
async def clone_campaign(
    campaign_id: str,
    meta_access_token: Optional[str] = None,
    name_suffix: Optional[str] = " - Copy",
    include_ad_sets: bool = True,
    include_ads: bool = True,
    include_creatives: bool = True,
    copy_schedule: bool = False,
    new_daily_budget: Optional[float] = None,
    new_status: Optional[str] = "PAUSED",
) -> str:
    """Duplicate a campaign using Meta's local Graph copy edge."""
    return await _forward_duplication_request(
        "campaign",
        campaign_id,
        meta_access_token,
        {
            "name_suffix": name_suffix,
            "include_ad_sets": include_ad_sets,
            "include_ads": include_ads,
            "include_creatives": include_creatives,
            "copy_schedule": copy_schedule,
            "new_daily_budget": new_daily_budget,
            "new_status": new_status,
        },
    )


@mcp_server.tool()
@meta_api_tool
async def clone_ad_set(
    ad_set_id: str,
    meta_access_token: Optional[str] = None,
    target_campaign_id: Optional[str] = None,
    name_suffix: Optional[str] = " - Copy",
    include_ads: bool = True,
    include_creatives: bool = True,
    new_daily_budget: Optional[float] = None,
    new_targeting: Optional[Dict[str, Any]] = None,
    new_status: Optional[str] = "PAUSED",
) -> str:
    """Duplicate an ad set using Meta's local Graph copy edge."""
    return await _forward_duplication_request(
        "adset",
        ad_set_id,
        meta_access_token,
        {
            "target_campaign_id": target_campaign_id,
            "name_suffix": name_suffix,
            "include_ads": include_ads,
            "include_creatives": include_creatives,
            "new_daily_budget": new_daily_budget,
            "new_targeting": new_targeting,
            "new_status": new_status,
        },
    )


@mcp_server.tool()
@meta_api_tool
async def clone_ad(
    ad_id: str,
    meta_access_token: Optional[str] = None,
    target_ad_set_id: Optional[str] = None,
    name_suffix: Optional[str] = " - Copy",
    clone_ad_creative: bool = True,
    new_creative_name: Optional[str] = None,
    new_status: Optional[str] = "PAUSED",
) -> str:
    """Duplicate an ad using Meta's local Graph copy edge."""
    return await _forward_duplication_request(
        "ad",
        ad_id,
        meta_access_token,
        {
            "target_ad_set_id": target_ad_set_id,
            "name_suffix": name_suffix,
            "clone_ad_creative": clone_ad_creative,
            "new_creative_name": new_creative_name,
            "new_status": new_status,
        },
    )


@mcp_server.tool()
@meta_api_tool
async def clone_ad_creative(
    ad_creative_id: str,
    meta_access_token: Optional[str] = None,
    name_suffix: Optional[str] = " - Copy",
    new_primary_text: Optional[str] = None,
    new_headline: Optional[str] = None,
    new_description: Optional[str] = None,
    new_cta_type: Optional[str] = None,
    new_destination_url: Optional[str] = None,
) -> str:
    """Duplicate a creative using Meta's local Graph copy edge."""
    return await _forward_duplication_request(
        "creative",
        ad_creative_id,
        meta_access_token,
        {
            "name_suffix": name_suffix,
            "new_primary_text": new_primary_text,
            "new_headline": new_headline,
            "new_description": new_description,
            "new_cta_type": new_cta_type,
            "new_destination_url": new_destination_url,
        },
    )


async def _forward_duplication_request(
    resource_type: str,
    resource_id: str,
    meta_access_token: Optional[str],
    options: Dict[str, Any],
) -> str:
    """Execute OSS-local duplication against Graph API copy edges."""
    try:
        facebook_token = meta_access_token if meta_access_token else await auth.get_current_access_token()
        if not facebook_token:
            raise DuplicationError(
                json.dumps(
                    {
                        "success": False,
                        "error": "authentication_required",
                        "message": "Meta Ads access token not found",
                        "details": {
                            "required": "Valid Meta access token",
                            "check": "Authenticate and retry duplication request.",
                        },
                    },
                    indent=2,
                )
            )

        preflight_block = await _run_v25_duplication_preflight(resource_type, resource_id, facebook_token)
        if preflight_block:
            raise DuplicationError(
                json.dumps(
                    {
                        "success": False,
                        "error": "v25_blocked_operation",
                        "message": "Duplication is blocked for deprecated Advantage+ Shopping/App campaign flows in v25.",
                        "details": {
                            "resource_type": resource_type,
                            "resource_id": resource_id,
                            "campaign_id": preflight_block.get("campaign_id"),
                            "campaign_name": preflight_block.get("campaign_name"),
                            "campaign_objective": preflight_block.get("campaign_objective"),
                            "smart_promotion_type": preflight_block.get("smart_promotion_type"),
                            "reason": preflight_block.get("reason"),
                        },
                        "suggestion": (
                            "Use supported Advantage+ migration or rebuild the campaign with v25-compatible "
                            "flows before attempting duplication."
                        ),
                    },
                    indent=2,
                )
            )

        copy_params, warnings = _build_copy_params(resource_type, options)
        endpoint = f"{resource_id}/copies"

        data = await make_api_request(endpoint, facebook_token, copy_params, method="POST")
        if not isinstance(data, dict):
            raise DuplicationError(
                json.dumps(
                    {
                        "success": False,
                        "error": "duplication_failed",
                        "message": "Unexpected response from Graph API copy edge",
                        "details": {
                            "resource_type": resource_type,
                            "resource_id": resource_id,
                            "response_type": type(data).__name__,
                        },
                    },
                    indent=2,
                )
            )

        graph_error = data.get("error") if isinstance(data.get("error"), dict) else None
        if graph_error:
            code = graph_error.get("code")
            if code == 4:
                raise RateLimitError(
                    json.dumps(
                        {
                            "error": "rate_limit_exceeded",
                            "message": graph_error.get("message") or graph_error.get("primary_text", "Meta API rate limit exceeded"),
                            "details": {
                                "code": code,
                                "error_subcode": graph_error.get("error_subcode"),
                                "retry_hint": "Retry with backoff.",
                            },
                        },
                        indent=2,
                    )
                )

            raise DuplicationError(
                json.dumps(
                    _build_graph_error_payload(resource_type, resource_id, graph_error),
                    indent=2,
                )
            )

        new_id = _extract_new_id(resource_type, data)
        success = bool(new_id) or bool(data.get("success") is True)

        return json.dumps(
            {
                "success": success,
                "source_id": resource_id,
                "resource_type": resource_type,
                "new_id": new_id,
                "warnings": warnings,
                "meta_response": data,
            },
            indent=2,
        )

    except (RateLimitError, DuplicationError):
        raise
    except Exception as exc:
        raise DuplicationError(
            json.dumps(
                {
                    "success": False,
                    "error": "unexpected_error",
                    "message": f"Unexpected error during {resource_type} duplication",
                    "details": {
                        "resource_type": resource_type,
                        "resource_id": resource_id,
                        "error": str(exc),
                    },
                },
                indent=2,
            )
        )
