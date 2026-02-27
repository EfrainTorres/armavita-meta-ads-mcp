"""Shared Graph API client and MCP tool decorator helpers."""


import functools
import json
import os
from typing import Any, Dict, Optional
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit

import httpx

from . import auth_state as auth
from .auth_state import auth_manager
from .graph_constants import META_GRAPH_API_BASE, META_GRAPH_API_VERSION
from .media_helpers import logger


class McpToolError(Exception):
    """Base error type surfaced as MCP tool errors."""


USER_AGENT = "armavita-meta-ads-mcp/1.0"

def _log_rate_headers(headers: dict, endpoint: str) -> None:
    usage_headers = {
        "x-app-usage": headers.get("x-app-usage"),
        "x-business-use-case-usage": headers.get("x-business-use-case-usage"),
        "x-ad-account-usage": headers.get("x-ad-account-usage"),
    }
    used = {k: v for k, v in usage_headers.items() if v}
    if used:
        logger.info("meta_rate_usage endpoint=%s data=%s", endpoint, json.dumps(used))



def _remap_graph_keys(value: Any) -> Any:
    key_aliases = {
        "meta_access_token": "access_token",
        "page_size": "limit",
        "page_cursor": "after",
        "date_range": "time_range",
        "ad_set_id": "adset_id",
        "ad_creative_id": "creative_id",
        "facebook_page_id": "page_id",
        "ad_image_hash": "image_hash",
        "ad_image_hashes": "image_hashes",
        "ad_video_id": "video_id",
        "lead_form_id": "lead_gen_form_id",
        "primary_text": "message",
        "description_text": "description",
        "description_variants": "descriptions",
        "image_source_url": "image_url",
        "meta_user_id": "user_id",
    }

    if isinstance(value, dict):
        remapped: Dict[str, Any] = {}
        for key, item in value.items():
            remapped_key = key_aliases.get(key, key)
            remapped[remapped_key] = _remap_graph_keys(item)
        return remapped

    if isinstance(value, list):
        return [_remap_graph_keys(item) for item in value]

    return value


def _normalize_request_params(params: Optional[Dict[str, Any]], meta_access_token: str) -> Dict[str, Any]:
    payload: Dict[str, Any] = _remap_graph_keys(dict(params or {}))
    payload["access_token"] = meta_access_token
    normalized: Dict[str, Any] = {}
    for key, value in payload.items():
        if isinstance(value, (dict, list)):
            normalized[key] = json.dumps(value)
        else:
            normalized[key] = value
    return normalized


def _sanitize_url(raw_url: str) -> str:
    try:
        parts = urlsplit(raw_url)
        query_pairs = parse_qsl(parts.query, keep_blank_values=True)
        filtered_pairs = [(key, value) for key, value in query_pairs if key.lower() != "access_token"]
        sanitized_query = urlencode(filtered_pairs, doseq=True)
        return urlunsplit((parts.scheme, parts.netloc, parts.path, sanitized_query, parts.fragment))
    except Exception:  # noqa: BLE001
        return raw_url


def _sanitize_response_payload(value: Any) -> Any:
    """Recursively strip access tokens from URL-like response values."""
    if isinstance(value, dict):
        return {key: _sanitize_response_payload(item) for key, item in value.items()}

    if isinstance(value, list):
        return [_sanitize_response_payload(item) for item in value]

    if isinstance(value, str) and "access_token=" in value.lower():
        return _sanitize_url(value)

    return value


async def make_api_request(
    endpoint: str,
    meta_access_token: str,
    params: Optional[Dict[str, Any]] = None,
    method: str = "GET",
) -> Dict[str, Any]:
    """Execute a Meta Graph API request and return normalized JSON payload."""
    if not meta_access_token:
        return {
            "error": {
                "message": "Authentication Required",
                "details": "A valid access token is required to access the Meta API",
                "action_required": "Please authenticate first",
            }
        }

    url = f"{META_GRAPH_API_BASE}/{endpoint}"
    request_params = _normalize_request_params(params, meta_access_token)
    safe_params = {k: ("***TOKEN***" if k == "access_token" else v) for k, v in request_params.items()}

    logger.debug("Graph request method=%s url=%s params=%s", method, url, safe_params)

    async with httpx.AsyncClient(timeout=30.0) as client:
        try:
            if method == "GET":
                response = await client.get(url, params=request_params, headers={"User-Agent": USER_AGENT})
            elif method == "POST":
                response = await client.post(url, data=request_params, headers={"User-Agent": USER_AGENT})
            elif method == "DELETE":
                response = await client.delete(url, params=request_params, headers={"User-Agent": USER_AGENT})
            else:
                return {"error": {"message": f"Unsupported HTTP method: {method}"}}

            _log_rate_headers(response.headers, endpoint)
            response.raise_for_status()
            try:
                return _sanitize_response_payload(response.json())
            except json.JSONDecodeError:
                return {"text_response": response.text, "status_code": response.status_code}

        except httpx.HTTPStatusError as exc:
            _log_rate_headers(exc.response.headers, endpoint)
            try:
                error_payload = exc.response.json()
            except Exception:  # noqa: BLE001
                error_payload = {
                    "status_code": exc.response.status_code,
                    "text": exc.response.text,
                }
            error_payload = _sanitize_response_payload(error_payload)

            error_obj = error_payload.get("error", {}) if isinstance(error_payload, dict) else {}
            code = error_obj.get("code") if isinstance(error_obj, dict) else None
            if code in {190, 102, 10}:
                auth_manager.invalidate_token()

            error_message = error_obj.get("message") or error_obj.get("primary_text", "")
            if code == 200 and isinstance(error_obj, dict) and "Provide valid app ID" in error_message:
                return {
                    "error": {
                        "message": "Meta API authentication configuration issue. Please check your app credentials.",
                        "original_error": error_message,
                        "code": code,
                    }
                }

            return {
                "error": {
                    "message": f"HTTP Error: {exc.response.status_code}",
                    "details": error_payload,
                    "full_response": {
                        "status_code": exc.response.status_code,
                        "url": _sanitize_url(str(exc.response.url)),
                        "request_method": exc.request.method,
                    },
                }
            }

        except Exception as exc:  # noqa: BLE001
            logger.exception("Graph request failed: %s", exc)
            message = str(exc)
            if "access_token=" in message.lower():
                message = _sanitize_url(message)
            return {"error": {"message": message}}



def _auth_error_payload() -> str:
    app_id = auth_manager.app_id
    auth_url = auth_manager.get_auth_url()
    return json.dumps(
        {
            "error": {
                "message": "Authentication Required",
                "details": {
                    "description": "You need to authenticate with the Meta API before using this tool",
                    "action_required": "Please authenticate first",
                    "auth_url": auth_url,
                    "configuration_status": {
                        "app_id_configured": bool(app_id) and app_id != "YOUR_META_APP_ID",
                    },
                    "troubleshooting": "Set META_ACCESS_TOKEN or complete OAuth login with META_APP_ID and META_APP_SECRET.",
                    "markdown_link": f"[Click here to authenticate with Meta Ads API]({auth_url})",
                },
            }
        },
        indent=2,
    )


# Generic wrapper for all Meta API tools
def meta_api_tool(func):
    """Decorator adding auth bootstrap and stable error serialization for MCP tools."""

    @functools.wraps(func)
    async def wrapper(*args, **kwargs):
        try:
            safe_kwargs = {k: ("***TOKEN***" if k == "meta_access_token" else v) for k, v in kwargs.items()}
            logger.debug("Tool call name=%s kwargs=%s", func.__name__, safe_kwargs)

            if not kwargs.get("meta_access_token"):
                token = await auth.get_current_access_token()
                if token:
                    kwargs["meta_access_token"] = token
                else:
                    return _auth_error_payload()

            result = await func(*args, **kwargs)

            if isinstance(result, dict):
                return json.dumps(result, indent=2)

            if isinstance(result, str):
                try:
                    parsed = json.loads(result)
                    if isinstance(parsed, dict) and isinstance(parsed.get("error"), str):
                        return json.dumps({"data": result}, indent=2)
                    return result
                except Exception:  # noqa: BLE001
                    return json.dumps({"data": result}, indent=2)

            return result
        except McpToolError:
            raise
        except Exception as exc:  # noqa: BLE001
            logger.exception("Unhandled tool exception in %s", func.__name__)
            return json.dumps({"error": str(exc)}, indent=2)

    return wrapper


logger.info("Core API initialized using Graph version %s", META_GRAPH_API_VERSION)
logger.info("META_APP_ID configured: %s", "yes" if os.environ.get("META_APP_ID") else "no")
