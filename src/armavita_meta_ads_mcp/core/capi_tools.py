# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""Conversions API (CAPI) — server-side event ingestion.

Covers the universal `/{dataset_id}/events` endpoint (web, app, and offline
events all flow through it) plus the Dataset Quality API for matching health.

PII auto-hashing: any field listed in `_HASHABLE_USER_DATA_KEYS` (em, ph, fn,
ln, etc.) is SHA256-hashed if not already a 64-char hex digest. Phone (`ph`) is
first normalized to digits only. Values that look hashed are forwarded as-is.
The `client_ip_address`, `client_user_agent`, `madid`, `subscription_id`, `fbc`,
`fbp`, `fb_login_id`, and `lead_id` fields are forwarded verbatim — Meta does
not want these hashed.
"""


import hashlib
import json
import re
import time
from typing import Any, Dict, List, Optional

from .graph_client import make_api_request, meta_api_tool
from .mcp_runtime import mcp_server
from mcp.types import ToolAnnotations


# Per Conversions API spec: hash these keys before sending. Note that
# subscription_id, fbc, fbp, fb_login_id, lead_id, madid, client_ip_address,
# and client_user_agent are technical identifiers sent in PLAINTEXT (NOT hashed).
_HASHABLE_USER_DATA_KEYS = {
    "em", "ph", "fn", "ln", "fi", "ge", "db", "ct", "st", "zp", "country",
    "external_id",
}


def _json(payload: Dict[str, Any]) -> str:
    return json.dumps(payload, indent=2)


def _looks_hashed(value: Any) -> bool:
    if not isinstance(value, str) or len(value) != 64:
        return False
    return all(c in "0123456789abcdef" for c in value.lower())


def _normalize_pii_text(key: str, value: Any) -> str:
    """Field-specific normalization required before SHA256 (per Meta's CAPI spec).

    Getting this wrong means the digest never matches Meta's, silently dropping
    the identifier. Phone/DOB -> digits only; gender/first-initial -> one char;
    city/state -> strip spaces+punctuation; zip -> strip spaces/dashes; the rest
    (email/name/country/external_id) -> trim + lowercase.
    """
    raw = str(value)
    if key in ("ph", "db"):
        return re.sub(r"\D", "", raw)
    if key in ("ge", "fi"):
        return raw.strip().lower()[:1]
    if key in ("ct", "st"):
        return re.sub(r"[^a-z0-9]", "", raw.lower())
    if key == "zp":
        return re.sub(r"[\s\-]", "", raw.strip().lower())
    return raw.strip().lower()


def _hash_pii(key: str, value: Any) -> str:
    return hashlib.sha256(_normalize_pii_text(key, value).encode("utf-8")).hexdigest()


def _normalize_one(key: str, value: Any) -> Any:
    if isinstance(value, str) and _looks_hashed(value):
        return value.lower()
    return _hash_pii(key, value)


def _normalize_user_data_value(key: str, value: Any) -> Any:
    if key not in _HASHABLE_USER_DATA_KEYS:
        return value
    if isinstance(value, list):
        return [_normalize_one(key, v) for v in value]
    return _normalize_one(key, value)


def _normalize_user_data(user_data: Dict[str, Any]) -> Dict[str, Any]:
    return {key: _normalize_user_data_value(key, val) for key, val in user_data.items()}


def _prepare_event(event: Dict[str, Any]) -> Dict[str, Any]:
    prepared = dict(event)
    if "event_time" not in prepared:
        prepared["event_time"] = int(time.time())
    user_data = prepared.get("user_data")
    if isinstance(user_data, dict):
        prepared["user_data"] = _normalize_user_data(user_data)
    return prepared


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=False, destructiveHint=False, idempotentHint=False, openWorldHint=True))
@meta_api_tool
async def send_capi_events(
    dataset_id: str,
    events: List[Dict[str, Any]],
    meta_access_token: Optional[str] = None,
    test_event_code: Optional[str] = None,
    auto_hash_user_data: bool = True,
    partner_agent: Optional[str] = None,
) -> str:
    """Send server-side conversion events through the Conversions API (web/app/offline).

    `dataset_id` is the Pixel/Dataset ID. Each event minimally needs:
      * `event_name` (e.g., "Purchase", "Lead", "AddToCart")
      * `action_source` ("website", "app", "system_generated", "physical_store", ...)
      * `event_source_url` for web events, or app/offline equivalents
      * `user_data` with at least one matchable identifier (em/ph/external_id/etc.)
    Set `event_id` to enable Pixel ↔ CAPI deduplication.

    Example event:
      {"event_name":"Purchase","event_time":1714000000,"action_source":"website",
       "event_source_url":"https://shop.example.com/order/123","event_id":"order-123",
       "user_data":{"em":"alice@example.com","ph":"15551234567"},
       "custom_data":{"value":29.99,"currency":"USD"}}

    With `auto_hash_user_data=True` (default) PII fields inside `user_data`
    (em/ph/fn/ln/ge/db/ct/st/zp/country/external_id) are SHA256-hashed if not
    already hashed (email/name lowercased+trimmed; phone reduced to digits only).
    `client_ip_address`, `client_user_agent`, `madid`, `subscription_id`, `fbc`,
    `fbp`, `fb_login_id`, and `lead_id` are forwarded verbatim per Meta spec.

    Use `test_event_code` (from Events Manager → Test Events) to route to test
    events for verification before going live.
    """
    if not str(dataset_id or "").strip():
        return _json({"error": "dataset_id is required"})
    if not events:
        return _json({"error": "events list is required"})

    if auto_hash_user_data:
        normalized_events = [_prepare_event(event) for event in events]
    else:
        normalized_events = [
            {**event, "event_time": event.get("event_time") or int(time.time())}
            for event in events
        ]

    body: Dict[str, Any] = {"data": json.dumps(normalized_events)}
    if test_event_code:
        body["test_event_code"] = test_event_code
    if partner_agent:
        body["partner_agent"] = partner_agent

    result = await make_api_request(
        f"{str(dataset_id).strip()}/events",
        meta_access_token,
        body,
        method="POST",
    )
    return _json(result)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=True))
@meta_api_tool
async def read_dataset_quality(
    dataset_id: str,
    meta_access_token: Optional[str] = None,
) -> str:
    """Read CAPI dataset quality metrics — match rates, deduplication health, EMQ."""
    if not str(dataset_id or "").strip():
        return _json({"error": "dataset_id is required"})

    payload = await make_api_request(
        f"{str(dataset_id).strip()}/dataset_quality",
        meta_access_token,
        {},
    )
    return _json(payload)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=True))
@meta_api_tool
async def list_business_datasets(
    business_id: str,
    meta_access_token: Optional[str] = None,
    page_size: int = 25,
    page_cursor: str = "",
) -> str:
    """List CAPI datasets (Pixels) owned by a business."""
    if not str(business_id or "").strip():
        return _json({"error": "business_id is required"})

    params: Dict[str, Any] = {
        "fields": "id,name,creation_time,last_fired_time,is_unavailable",
        "page_size": int(page_size),
    }
    if page_cursor:
        params["page_cursor"] = page_cursor

    payload = await make_api_request(
        f"{str(business_id).strip()}/owned_pixels",
        meta_access_token,
        params,
    )
    return _json(payload)
