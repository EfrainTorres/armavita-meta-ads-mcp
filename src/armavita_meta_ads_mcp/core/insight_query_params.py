"""Normalization helpers for Meta insights query parameters."""

from __future__ import annotations

from typing import Any, Dict, List, Optional, Sequence, Tuple, Union

_SUPPORTED_DATE_PRESETS = {
    "today",
    "yesterday",
    "this_month",
    "last_month",
    "this_quarter",
    "last_quarter",
    "this_year",
    "last_year",
    "last_3d",
    "last_7d",
    "last_14d",
    "last_28d",
    "last_30d",
    "last_90d",
    "maximum",
    "data_maximum",
    "this_week_mon_today",
    "this_week_sun_today",
    "last_week_mon_sun",
    "last_week_sun_sat",
}

_DATE_PRESET_ALIASES = {
    "previous_3d": "last_3d",
    "previous_7d": "last_7d",
    "previous_14d": "last_14d",
    "previous_28d": "last_28d",
    "previous_30d": "last_30d",
    "previous_90d": "last_90d",
}

_ACTION_BREAKDOWN_KEYS = {
    "action_type",
    "action_target_id",
    "action_destination",
    "action_device",
    "action_canvas_component_name",
    "action_carousel_card_id",
    "action_carousel_card_name",
    "action_reaction",
    "action_video_sound",
}


def _normalize_list_tokens(values: Optional[Sequence[Any]]) -> List[str]:
    normalized: List[str] = []
    seen = set()
    for raw in values or []:
        token = str(raw).strip()
        if not token:
            continue
        if token in seen:
            continue
        seen.add(token)
        normalized.append(token)
    return normalized


def normalize_time_input(
    date_range: Union[str, Dict[str, str]],
    *,
    default_preset: str,
) -> Tuple[Optional[Dict[str, Any]], Optional[Dict[str, Any]], List[Dict[str, Any]]]:
    """Normalize date range/preset input to Graph API request params."""
    warnings: List[Dict[str, Any]] = []

    if isinstance(date_range, dict):
        since = str(date_range.get("since", "")).strip()
        until = str(date_range.get("until", "")).strip()
        if not (since and until):
            return None, {
                "error": "invalid_date_range",
                "message": "Custom date_range must contain both 'since' and 'until' in YYYY-MM-DD format.",
            }, warnings
        return {"date_range": {"since": since, "until": until}}, None, warnings

    raw_preset = str(date_range or "").strip().lower() or default_preset
    canonical_preset = _DATE_PRESET_ALIASES.get(raw_preset, raw_preset)

    if canonical_preset != raw_preset:
        warnings.append(
            {
                "code": "date_preset_alias_applied",
                "message": f"Mapped unsupported date preset alias '{raw_preset}' to '{canonical_preset}'.",
                "provided": raw_preset,
                "applied": canonical_preset,
            }
        )

    if canonical_preset not in _SUPPORTED_DATE_PRESETS:
        return None, {
            "error": "invalid_date_preset",
            "message": f"Unsupported date_preset '{raw_preset}'.",
            "supported_presets": sorted(_SUPPORTED_DATE_PRESETS),
            "known_aliases": _DATE_PRESET_ALIASES,
        }, warnings

    return {"date_preset": canonical_preset}, None, warnings


def normalize_breakdown_inputs(
    *,
    breakdown: str = "",
    breakdowns: Optional[Sequence[Any]] = None,
    action_breakdowns: Optional[Sequence[Any]] = None,
    summary_action_breakdowns: Optional[Sequence[Any]] = None,
) -> Tuple[Dict[str, str], List[Dict[str, Any]]]:
    """Normalize/route breakdown inputs into Graph API parameters."""
    warnings: List[Dict[str, Any]] = []

    inferred_tokens = _normalize_list_tokens((str(breakdown).split(",") if breakdown else []) + list(breakdowns or []))
    explicit_action_tokens = _normalize_list_tokens(action_breakdowns)
    summary_action_tokens = _normalize_list_tokens(summary_action_breakdowns)

    routed_breakdowns: List[str] = []
    inferred_action_tokens: List[str] = []
    for token in inferred_tokens:
        normalized = token.strip()
        if not normalized:
            continue
        token_lower = normalized.lower()
        if token_lower in _ACTION_BREAKDOWN_KEYS or token_lower.startswith("action_"):
            inferred_action_tokens.append(normalized)
        else:
            routed_breakdowns.append(normalized)

    combined_action_tokens = _normalize_list_tokens(inferred_action_tokens + explicit_action_tokens)

    if inferred_action_tokens:
        warnings.append(
            {
                "code": "breakdown_autorouted",
                "message": (
                    "Moved action breakdown keys from breakdowns to action_breakdowns to avoid invalid Meta combinations."
                ),
                "moved_keys": inferred_action_tokens,
            }
        )

    params: Dict[str, str] = {}
    if routed_breakdowns:
        params["breakdowns"] = ",".join(routed_breakdowns)
    if combined_action_tokens:
        params["action_breakdowns"] = ",".join(combined_action_tokens)
    if summary_action_tokens:
        params["summary_action_breakdowns"] = ",".join(summary_action_tokens)

    return params, warnings
