"""Ad-account read utilities."""


import json
from typing import Any, Dict, Optional

from .graph_client import make_api_request, meta_api_tool
from .mcp_runtime import mcp_server

_ZERO_DECIMAL_CURRENCIES = {
    "BIF",
    "CLP",
    "DJF",
    "GNF",
    "JPY",
    "KMF",
    "KRW",
    "MGA",
    "PYG",
    "RWF",
    "UGX",
    "VND",
    "VUV",
    "XAF",
    "XOF",
    "XPF",
}

_EU_DSA_COUNTRIES = {
    "AT",
    "BE",
    "DE",
    "DK",
    "ES",
    "FI",
    "FR",
    "IE",
    "IT",
    "NL",
    "NO",
    "SE",
}


def _normalize_account_id(ad_account_id: str) -> str:
    ad_account_id = str(ad_account_id or "").strip()
    if not ad_account_id:
        return ""
    if ad_account_id.startswith("act_"):
        return ad_account_id
    return f"act_{ad_account_id}"


def _convert_minor_units(value: Any, currency: str) -> str:
    try:
        numeric = int(value)
    except (TypeError, ValueError):
        return str(value)

    if str(currency or "").upper() in _ZERO_DECIMAL_CURRENCIES:
        return str(numeric)
    return f"{numeric / 100:.2f}"


def _normalize_money_fields(record: Dict[str, Any]) -> Dict[str, Any]:
    currency = str(record.get("currency", "USD"))
    for field in ("amount_spent", "balance"):
        if field in record:
            record[field] = _convert_minor_units(record[field], currency)
    return record


def _looks_like_access_error(payload: Dict[str, Any]) -> bool:
    text = json.dumps(payload, default=str).lower()
    return "permission" in text or "access" in text


async def _list_accessible_accounts(meta_access_token: str) -> Dict[str, Any]:
    return await make_api_request(
        "me/adaccounts",
        meta_access_token,
        {
            "fields": (
                "id,name,account_id,account_status,amount_spent,balance,currency,"
                "age,business_city,business_country_code"
            ),
            "page_size": 50,
        },
    )


@mcp_server.tool()
@meta_api_tool
async def list_ad_accounts(
    meta_access_token: Optional[str] = None,
    meta_user_id: str = "me",
    page_size: int = 200,
    page_cursor: str = "",
) -> str:
    """List ad accounts visible to a user context."""
    params: Dict[str, Any] = {
        "fields": (
            "id,name,account_id,account_status,amount_spent,balance,currency,"
            "age,business_city,business_country_code"
        ),
        "page_size": int(page_size),
    }
    if page_cursor:
        params["page_cursor"] = page_cursor

    payload = await make_api_request(f"{meta_user_id}/adaccounts", meta_access_token, params)

    if isinstance(payload, dict) and isinstance(payload.get("data"), list):
        payload["data"] = [
            _normalize_money_fields(item) if isinstance(item, dict) else item
            for item in payload["data"]
        ]

    return json.dumps(payload, indent=2)


@mcp_server.tool()
@meta_api_tool
async def read_ad_account(ad_account_id: str, meta_access_token: Optional[str] = None) -> str:
    """Return account metadata for a single ad account."""
    normalized_account_id = _normalize_account_id(ad_account_id)
    if not normalized_account_id:
        return json.dumps(
            {
                "error": {
                    "message": "Account ID is required",
                    "details": "Please specify an ad_account_id parameter",
                    "example": "Use ad_account_id='act_123456789' or ad_account_id='123456789'",
                }
            },
            indent=2,
        )

    payload = await make_api_request(
        normalized_account_id,
        meta_access_token,
        {
            "fields": (
                "id,name,account_id,account_status,amount_spent,balance,currency,age,"
                "business_city,business_country_code,timezone_name"
            )
        },
    )

    if isinstance(payload, dict) and payload.get("error") and _looks_like_access_error(payload):
        accounts_payload = await _list_accessible_accounts(meta_access_token)
        if isinstance(accounts_payload, dict) and isinstance(accounts_payload.get("data"), list):
            visible = [
                {"id": item.get("id"), "name": item.get("name")}
                for item in accounts_payload["data"]
                if isinstance(item, dict)
            ]
            return json.dumps(
                {
                    "error": {
                        "message": f"Account {normalized_account_id} is not accessible to your user account",
                        "details": "This account either doesn't exist or you don't have permission to access it",
                        "accessible_accounts": visible[:10],
                        "total_accessible_accounts": len(visible),
                        "suggestion": "Try using one of the accessible account IDs listed above",
                    }
                },
                indent=2,
            )

    if isinstance(payload, dict) and not payload.get("error"):
        _normalize_money_fields(payload)
        country = str(payload.get("business_country_code", "")).upper()
        payload["dsa_required"] = country in _EU_DSA_COUNTRIES
        payload["dsa_compliance_note"] = (
            "This account is subject to European DSA (Digital Services Act) requirements"
            if payload["dsa_required"]
            else "This account is not subject to European DSA requirements"
        )

    return json.dumps(payload, indent=2)
