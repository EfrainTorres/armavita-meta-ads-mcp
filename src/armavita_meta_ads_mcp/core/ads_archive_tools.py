# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""Facebook Ads Library search tool."""


import json
from typing import List, Optional

from .graph_client import make_api_request, meta_api_tool
from .mcp_runtime import mcp_server
from mcp.types import ToolAnnotations


def _normalize_ad_type(ad_type: str) -> str:
    normalized = str(ad_type or "ALL").strip().upper()
    if normalized == "CREDIT_ADS":
        return "FINANCIAL_PRODUCTS_AND_SERVICES_ADS"
    return normalized or "ALL"


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=True))
@meta_api_tool
async def search_ads_archive(
    ad_reached_countries: List[str],
    search_terms: str = "",
    meta_access_token: Optional[str] = None,
    ad_type: str = "ALL",
    page_size: int = 25,
    page_cursor: str = "",
    fields: str = (
        "id,ad_creation_time,ad_creative_bodies,ad_creative_link_captions,"
        "ad_creative_link_descriptions,ad_creative_link_titles,ad_delivery_start_time,"
        "ad_delivery_stop_time,ad_snapshot_url,page_id,page_name,publisher_platforms,languages"
    ),
    ad_active_status: str = "",
    ad_delivery_date_min: str = "",
    ad_delivery_date_max: str = "",
    search_page_ids: Optional[List[str]] = None,
    search_type: str = "",
    languages: Optional[List[str]] = None,
    media_type: str = "",
    publisher_platforms: Optional[List[str]] = None,
) -> str:
    """Search ads in Meta's public Ads Library endpoint.

    Requires `ad_reached_countries` plus at least one of `search_terms` or
    `search_page_ids` — a page-IDs-only search ("all ads from these pages") is valid.
    """
    if not isinstance(ad_reached_countries, list) or not ad_reached_countries:
        return json.dumps({"error": "ad_reached_countries is required (list of ISO country codes)"}, indent=2)

    countries = [str(code).strip().upper() for code in ad_reached_countries if str(code).strip()]
    if not countries:
        return json.dumps({"error": "ad_reached_countries is required (list of ISO country codes)"}, indent=2)

    has_terms = bool(str(search_terms or "").strip())
    has_pages = bool(search_page_ids) and any(str(p).strip() for p in search_page_ids)
    if not has_terms and not has_pages:
        return json.dumps(
            {"error": "Provide search_terms and/or search_page_ids (at least one is required)."},
            indent=2,
        )

    payload = {
        "ad_reached_countries": countries,
        "ad_type": _normalize_ad_type(ad_type),
        "page_size": int(page_size),
        "fields": fields,
    }
    if has_terms:
        payload["search_terms"] = search_terms
    if page_cursor:
        payload["page_cursor"] = page_cursor
    if ad_active_status:
        payload["ad_active_status"] = str(ad_active_status).strip().upper()
    if ad_delivery_date_min:
        payload["ad_delivery_date_min"] = ad_delivery_date_min
    if ad_delivery_date_max:
        payload["ad_delivery_date_max"] = ad_delivery_date_max
    if search_page_ids:
        cleaned_pages = [str(page_id).strip() for page_id in search_page_ids if str(page_id).strip()]
        if cleaned_pages:
            payload["search_page_ids"] = cleaned_pages
    if search_type:
        payload["search_type"] = str(search_type).strip().upper()
    if languages:
        cleaned_langs = [str(lang).strip() for lang in languages if str(lang).strip()]
        if cleaned_langs:
            payload["languages"] = cleaned_langs
    if media_type:
        payload["media_type"] = str(media_type).strip().upper()
    if publisher_platforms:
        cleaned_platforms = [str(item).strip().upper() for item in publisher_platforms if str(item).strip()]
        if cleaned_platforms:
            payload["publisher_platforms"] = cleaned_platforms

    result = await make_api_request("ads_archive", meta_access_token, payload, method="GET")
    return json.dumps(result, indent=2)
