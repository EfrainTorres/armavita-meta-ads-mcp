# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""Custom Audiences and Lookalikes.

Covers CRUD on `/customaudiences` plus user add/replace/remove and lookalike
creation. `lookalike_spec` is required when creating a lookalike audience.

PII hashing: `manage_custom_audience_users` auto-hashes PII columns by default
(email/name lowercased+trimmed, PHONE reduced to digits, then SHA256), while
MADID and EXTERN_ID columns are sent in plaintext per Meta's spec. Pass
already-hashed 64-char hex strings and they'll pass through; set
`auto_hash=False` to opt out entirely.
"""


import hashlib
import json
import re
from typing import Any, Dict, List, Optional

from .graph_client import make_api_request, meta_api_tool
from .mcp_runtime import mcp_server
from mcp.types import ToolAnnotations


_AUDIENCE_FIELDS = (
    "id,name,description,subtype,approximate_count_lower_bound,"
    "approximate_count_upper_bound,delivery_status,operation_status,"
    "customer_file_source,is_value_based,lookalike_spec,retention_days,"
    "rule,rule_aggregation,time_created,time_updated"
)

_USER_OPERATIONS = {"add", "replace", "remove"}

# Per Meta's customer-list spec these identifiers are sent in PLAINTEXT and
# must never be SHA256-hashed (hashing them silently breaks all matching).
_NO_HASH_SCHEMA_KEYS = {"MADID", "EXTERN_ID"}


def _json(payload: Dict[str, Any]) -> str:
    return json.dumps(payload, indent=2)


def _normalize_account(ad_account_id: str) -> str:
    value = str(ad_account_id or "").strip()
    if not value:
        return ""
    return value if value.startswith("act_") else f"act_{value}"


def _looks_hashed(value: Any) -> bool:
    """SHA256 hex digests are 64 hex chars (case-insensitive)."""
    if not isinstance(value, str) or len(value) != 64:
        return False
    return all(c in "0123456789abcdef" for c in value.lower())


def _normalize_audience_value(schema_key: str, value: Any) -> Any:
    """Normalize+hash one cell per Meta's customer-list rules.

    MADID and EXTERN_ID are forwarded in plaintext. PHONE is reduced to digits
    only before hashing; other PII fields are lowercased+trimmed. Already-hashed
    64-char hex values pass through (lowercased).
    """
    key = str(schema_key or "").strip().upper()
    if key in _NO_HASH_SCHEMA_KEYS:
        return value
    if isinstance(value, str) and _looks_hashed(value):
        return value.lower()
    return hashlib.sha256(_normalize_audience_text(key, value).encode("utf-8")).hexdigest()


def _normalize_audience_text(key: str, value: Any) -> str:
    """Field-specific customer-list normalization before SHA256 (per Meta's spec).

    PHONE / DOB(Y/M/D) -> digits only; GEN / FI -> one lowercase char;
    CT(city) / ST(state) -> strip spaces+punctuation; ZIP -> strip spaces/dashes;
    the rest (EMAIL/FN/LN/COUNTRY) -> trim + lowercase.
    """
    raw = str(value)
    if key in ("PHONE", "DOBY", "DOBM", "DOBD"):
        return re.sub(r"\D", "", raw)
    if key in ("GEN", "FI"):
        return raw.strip().lower()[:1]
    if key in ("CT", "ST"):
        return re.sub(r"[^a-z0-9]", "", raw.lower())
    if key == "ZIP":
        return re.sub(r"[\s\-]", "", raw.strip().lower())
    return raw.strip().lower()


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=True))
@meta_api_tool
async def list_custom_audiences(
    ad_account_id: str,
    meta_access_token: Optional[str] = None,
    page_size: int = 25,
    page_cursor: str = "",
) -> str:
    """List custom audiences on an ad account."""
    normalized = _normalize_account(ad_account_id)
    if not normalized:
        return _json({"error": "ad_account_id is required"})

    params: Dict[str, Any] = {"fields": _AUDIENCE_FIELDS, "page_size": int(page_size)}
    if page_cursor:
        params["page_cursor"] = page_cursor

    payload = await make_api_request(
        f"{normalized}/customaudiences",
        meta_access_token,
        params,
    )
    return _json(payload)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=True))
@meta_api_tool
async def read_custom_audience(
    custom_audience_id: str,
    meta_access_token: Optional[str] = None,
) -> str:
    """Read full details for a custom audience, including delivery/flagged status."""
    if not str(custom_audience_id or "").strip():
        return _json({"error": "custom_audience_id is required"})

    payload = await make_api_request(
        str(custom_audience_id).strip(),
        meta_access_token,
        {"fields": _AUDIENCE_FIELDS + ",is_eligible_for_sac_campaigns"},
    )
    return _json(payload)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=False, destructiveHint=False, idempotentHint=False, openWorldHint=True))
@meta_api_tool
async def create_custom_audience(
    ad_account_id: str,
    name: str,
    subtype: str,
    meta_access_token: Optional[str] = None,
    description: Optional[str] = None,
    customer_file_source: Optional[str] = None,
    rule: Optional[Dict[str, Any]] = None,
    rule_aggregation: Optional[Dict[str, Any]] = None,
    retention_days: Optional[int] = None,
) -> str:
    """Create a custom audience.

    `subtype` examples: CUSTOM, WEBSITE, APP, ENGAGEMENT, VIDEO, LOOKALIKE,
    CLAIM, OFFLINE_CONVERSION. For customer-list audiences, also set
    `customer_file_source` (e.g., USER_PROVIDED_ONLY).
    """
    normalized = _normalize_account(ad_account_id)
    if not normalized:
        return _json({"error": "ad_account_id is required"})
    if not name:
        return _json({"error": "name is required"})
    if not subtype:
        return _json({"error": "subtype is required"})

    payload: Dict[str, Any] = {"name": name, "subtype": subtype}
    if description:
        payload["description"] = description
    if customer_file_source:
        payload["customer_file_source"] = customer_file_source
    if rule is not None:
        payload["rule"] = json.dumps(rule)
    if rule_aggregation is not None:
        payload["rule_aggregation"] = json.dumps(rule_aggregation)
    if retention_days is not None:
        payload["retention_days"] = int(retention_days)

    result = await make_api_request(
        f"{normalized}/customaudiences",
        meta_access_token,
        payload,
        method="POST",
    )
    return _json(result)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=False, destructiveHint=False, idempotentHint=True, openWorldHint=True))
@meta_api_tool
async def update_custom_audience(
    custom_audience_id: str,
    meta_access_token: Optional[str] = None,
    name: Optional[str] = None,
    description: Optional[str] = None,
    retention_days: Optional[int] = None,
    rule: Optional[Dict[str, Any]] = None,
    rule_aggregation: Optional[Dict[str, Any]] = None,
) -> str:
    """Update mutable fields on a custom audience."""
    if not str(custom_audience_id or "").strip():
        return _json({"error": "custom_audience_id is required"})

    payload: Dict[str, Any] = {}
    if name is not None:
        payload["name"] = name
    if description is not None:
        payload["description"] = description
    if retention_days is not None:
        payload["retention_days"] = int(retention_days)
    if rule is not None:
        payload["rule"] = json.dumps(rule)
    if rule_aggregation is not None:
        payload["rule_aggregation"] = json.dumps(rule_aggregation)

    if not payload:
        return _json({"error": "No update parameters provided"})

    result = await make_api_request(
        str(custom_audience_id).strip(),
        meta_access_token,
        payload,
        method="POST",
    )
    return _json(result)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=False, destructiveHint=True, idempotentHint=False, openWorldHint=True))
@meta_api_tool
async def delete_custom_audience(
    custom_audience_id: str,
    meta_access_token: Optional[str] = None,
) -> str:
    """Delete a custom audience."""
    if not str(custom_audience_id or "").strip():
        return _json({"error": "custom_audience_id is required"})

    result = await make_api_request(
        str(custom_audience_id).strip(),
        meta_access_token,
        {},
        method="DELETE",
    )
    return _json(result)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=False, destructiveHint=True, idempotentHint=False, openWorldHint=True))
@meta_api_tool
async def manage_custom_audience_users(
    custom_audience_id: str,
    operation: str,
    schema: List[str],
    data: List[List[Any]],
    meta_access_token: Optional[str] = None,
    auto_hash: bool = True,
    app_ids: Optional[List[str]] = None,
    page_ids: Optional[List[str]] = None,
) -> str:
    """Add, replace, or remove users on a customer-list custom audience.

    `operation`:
      * "add" — append users to the audience.
      * "replace" — atomically swap the audience contents (uses /usersreplace).
      * "remove" — remove the listed users from the audience.

    `schema` is a list per Meta spec, e.g. `["EMAIL"]` or `["EMAIL","PHONE"]`.
    `data` is a 2D array — outer rows are users, inner values match `schema` order.
    With `auto_hash=True` (default) each PII column is normalized then SHA256-hashed
    per Meta's rules (email/name lowercased+trimmed; PHONE reduced to digits only),
    while MADID and EXTERN_ID columns are forwarded in plaintext. Already-hashed
    64-char hex values pass through; set `auto_hash=False` to hash externally.

    Example: `operation="add", schema=["EMAIL"], data=[["alice@example.com"]]`.

    Note: "remove" sends payload via DELETE; very large lists (>~100 users) may
    hit URL-length limits — chunk into multiple calls in that case.
    """
    op = str(operation or "").strip().lower()
    if op not in _USER_OPERATIONS:
        return _json({"error": f"operation must be one of: {sorted(_USER_OPERATIONS)}"})
    if not str(custom_audience_id or "").strip():
        return _json({"error": "custom_audience_id is required"})
    if not schema or not data:
        return _json({"error": "schema and data are required"})

    if auto_hash:
        schema_keys = [str(s).strip().upper() for s in schema]
        normalized_data = [
            [
                _normalize_audience_value(schema_keys[i] if i < len(schema_keys) else "", value)
                for i, value in enumerate(row)
            ]
            for row in data
        ]
    else:
        normalized_data = data

    users_payload: Dict[str, Any] = {"schema": list(schema), "data": normalized_data}
    body: Dict[str, Any] = {"payload": json.dumps(users_payload)}
    if app_ids:
        body["app_ids"] = json.dumps(list(app_ids))
    if page_ids:
        body["page_ids"] = json.dumps(list(page_ids))

    if op == "replace":
        endpoint = f"{str(custom_audience_id).strip()}/usersreplace"
        method = "POST"
    elif op == "remove":
        endpoint = f"{str(custom_audience_id).strip()}/users"
        method = "DELETE"
    else:
        endpoint = f"{str(custom_audience_id).strip()}/users"
        method = "POST"

    result = await make_api_request(endpoint, meta_access_token, body, method=method)
    return _json(result)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=False, destructiveHint=False, idempotentHint=False, openWorldHint=True))
@meta_api_tool
async def create_lookalike_audience(
    ad_account_id: str,
    name: str,
    origin_audience_id: str,
    lookalike_spec: Dict[str, Any],
    meta_access_token: Optional[str] = None,
    description: Optional[str] = None,
) -> str:
    """Create a lookalike audience from a seed audience.

    `lookalike_spec` is required (v24.0+) and typically looks like
    `{"country": "US", "ratio": 0.01, "type": "similarity"}` or with
    `starting_ratio` / `ratio` for custom ranges.
    """
    normalized = _normalize_account(ad_account_id)
    if not normalized:
        return _json({"error": "ad_account_id is required"})
    if not name:
        return _json({"error": "name is required"})
    if not origin_audience_id:
        return _json({"error": "origin_audience_id is required"})
    if not lookalike_spec:
        return _json({"error": "lookalike_spec is required (v24.0+)"})

    payload: Dict[str, Any] = {
        "name": name,
        "subtype": "LOOKALIKE",
        "origin_audience_id": str(origin_audience_id).strip(),
        "lookalike_spec": json.dumps(lookalike_spec),
    }
    if description:
        payload["description"] = description

    result = await make_api_request(
        f"{normalized}/customaudiences",
        meta_access_token,
        payload,
        method="POST",
    )
    return _json(result)
