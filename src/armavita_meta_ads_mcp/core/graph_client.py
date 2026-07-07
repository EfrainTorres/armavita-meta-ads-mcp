# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""Shared Graph API client and MCP tool decorator helpers."""


import asyncio
import functools
import json
import os
from typing import Any, Dict, Optional
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit

import httpx

from . import auth_state as auth
from .auth_state import auth_manager
from .graph_constants import META_GRAPH_API_BASE, META_GRAPH_API_VERSION
from .media_helpers import USER_AGENT, logger


class McpToolError(Exception):
    """Base error type surfaced as MCP tool errors."""

def _log_rate_headers(headers: dict, endpoint: str) -> None:
    usage_headers = {
        "x-app-usage": headers.get("x-app-usage"),
        "x-business-use-case-usage": headers.get("x-business-use-case-usage"),
        "x-ad-account-usage": headers.get("x-ad-account-usage"),
    }
    used = {k: v for k, v in usage_headers.items() if v}
    if used:
        logger.info("meta_rate_usage endpoint=%s data=%s", endpoint, json.dumps(used))



# Transient-failure retry policy for Graph requests.
_MAX_RETRY_ATTEMPTS = 3
# Meta rate-limit error codes (app / user / page / custom). The request was rejected
# without being applied, so these are safe to retry for ANY method.
_RETRYABLE_GRAPH_CODES = frozenset({4, 17, 32, 613})
# Server-side errors and network failures are AMBIGUOUS for writes (the request may
# have been applied), so they are retried only for idempotent methods to avoid
# duplicate side effects (e.g. double-creating a campaign/ad).
_SERVER_ERROR_STATUS = frozenset({500, 502, 503, 504})
_IDEMPOTENT_METHODS = frozenset({"GET", "DELETE"})


def _backoff_seconds(attempt: int) -> float:
    """Exponential backoff: 1s, 2s, 4s … capped at 8s."""
    return min(8.0, 1.0 * (2 ** attempt))


def _retry_after_seconds(response: "httpx.Response") -> Optional[float]:
    raw = response.headers.get("retry-after")
    if not raw:
        return None
    try:
        return min(30.0, float(raw))
    except (TypeError, ValueError):
        return None


def _remap_graph_keys(value: Any) -> Any:
    """Translate the MCP's ergonomic parameter names to Graph API field names.

    Applied recursively so the alias map also covers nested specs the tools build
    (e.g. creative={"ad_creative_id": ...} -> creative={"creative_id": ...}, or
    link_data primary_text -> message). The alias *source* names are this MCP's own
    parameter names, so translating them at any depth is intended. NOTE: this means
    a caller-supplied nested dict that reuses one of these source names as a literal
    key will also be translated; pass already-JSON-encoded strings to opt out.
    """
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
    files: Optional[Dict[str, Any]] = None,
) -> Dict[str, Any]:
    """Execute a Meta Graph API request and return normalized JSON payload.

    Transient failures (HTTP 429/5xx and Meta rate-limit error codes 4/17/32/613)
    are retried with exponential backoff, honoring a `Retry-After` header when present.
    """
    if not meta_access_token:
        return {
            "error": {
                "message": "Not authenticated",
                "details": "This tool cannot call the Meta API without a valid access token",
                "action_required": "Run the login flow (or set META_ACCESS_TOKEN) and retry",
            }
        }

    url = f"{META_GRAPH_API_BASE}/{endpoint}"
    request_params = _normalize_request_params(params, meta_access_token)
    safe_params = {k: ("***TOKEN***" if k == "access_token" else v) for k, v in request_params.items()}

    logger.debug("Graph request method=%s url=%s params=%s", method, url, safe_params)

    async with httpx.AsyncClient(timeout=30.0) as client:
        for attempt in range(_MAX_RETRY_ATTEMPTS + 1):
            try:
                if method == "GET":
                    response = await client.get(url, params=request_params, headers={"User-Agent": USER_AGENT})
                elif method == "POST":
                    if files:
                        response = await client.post(
                            url,
                            data=request_params,
                            files=files,
                            headers={"User-Agent": USER_AGENT},
                        )
                    else:
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
                status = exc.response.status_code

                # Rate-limit (429 / Meta codes) -> safe to retry for any method.
                # Server 5xx -> retry only idempotent methods (avoid duplicate writes).
                is_rate_limited = status == 429 or code in _RETRYABLE_GRAPH_CODES
                is_retryable_server = status in _SERVER_ERROR_STATUS and method in _IDEMPOTENT_METHODS
                if (is_rate_limited or is_retryable_server) and attempt < _MAX_RETRY_ATTEMPTS:
                    delay = _retry_after_seconds(exc.response) or _backoff_seconds(attempt)
                    logger.warning(
                        "meta_retry endpoint=%s status=%s code=%s attempt=%s delay=%.1fs",
                        endpoint, status, code, attempt + 1, delay,
                    )
                    await asyncio.sleep(delay)
                    continue

                if code in {190, 102, 10}:
                    auth_manager.invalidate_token()

                error_message = error_obj.get("message") or error_obj.get("primary_text", "")
                if code == 200 and isinstance(error_obj, dict) and "Provide valid app ID" in error_message:
                    return {
                        "error": {
                            "message": "Your Meta app credentials appear misconfigured - verify META_APP_ID / META_APP_SECRET.",
                            "original_error": error_message,
                            "code": code,
                        }
                    }

                return {
                    "error": {
                        "message": f"HTTP Error: {status}",
                        "details": error_payload,
                        "full_response": {
                            "status_code": status,
                            "url": _sanitize_url(str(exc.response.url)),
                            "request_method": exc.request.method,
                        },
                    }
                }

            except httpx.TransportError as exc:
                # Network/timeout errors are ambiguous for writes (the request may have
                # been applied), so only retry idempotent methods; surface otherwise.
                if method in _IDEMPOTENT_METHODS and attempt < _MAX_RETRY_ATTEMPTS:
                    delay = _backoff_seconds(attempt)
                    logger.warning(
                        "meta_retry_transport endpoint=%s attempt=%s delay=%.1fs error=%s",
                        endpoint, attempt + 1, delay, type(exc).__name__,
                    )
                    await asyncio.sleep(delay)
                    continue
                logger.exception("Graph request failed (transport): %s", exc)
                message = str(exc)
                if "access_token=" in message.lower():
                    message = _sanitize_url(message)
                return {"error": {"message": message}}

            except Exception as exc:  # noqa: BLE001
                logger.exception("Graph request failed: %s", exc)
                message = str(exc)
                if "access_token=" in message.lower():
                    message = _sanitize_url(message)
                return {"error": {"message": message}}

    return {"error": {"message": "Max retries exceeded contacting the Meta Graph API"}}



def _auth_error_payload() -> str:
    app_id = auth_manager.app_id
    auth_url = auth_manager.get_auth_url()
    return json.dumps(
        {
            "error": {
                "message": "Not authenticated",
                "details": {
                    "description": "Authenticate with Meta before calling this tool",
                    "action_required": "Run the login flow (or set META_ACCESS_TOKEN) and retry",
                    "auth_url": auth_url,
                    "configuration_status": {
                        "app_id_configured": bool(app_id) and app_id != "MISSING_META_APP_ID",
                    },
                    "troubleshooting": "Set META_ACCESS_TOKEN or complete OAuth login with META_APP_ID and META_APP_SECRET.",
                    "markdown_link": f"[Click here to authenticate with Meta Ads API]({auth_url})",
                },
            }
        },
        indent=2,
    )


# Shared decorator applied to every Graph-calling tool
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