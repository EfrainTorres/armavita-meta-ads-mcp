"""Facebook Ads Library search tool."""


import json
from typing import List, Optional

from .graph_client import make_api_request, meta_api_tool
from .mcp_runtime import mcp_server


def _normalize_ad_type(ad_type: str) -> str:
    normalized = str(ad_type or "ALL").strip().upper()
    if normalized == "CREDIT_ADS":
        return "FINANCIAL_PRODUCTS_AND_SERVICES_ADS"
    return normalized or "ALL"


@mcp_server.tool()
@meta_api_tool
async def search_ads_archive(
    search_terms: str,
    ad_reached_countries: List[str],
    meta_access_token: Optional[str] = None,
    ad_type: str = "ALL",
    page_size: int = 25,
    page_cursor: str = "",
    fields: str = (
        "ad_creation_time,ad_creative_body,ad_creative_link_caption,ad_creative_link_description,"
        "ad_creative_link_title,ad_delivery_start_time,ad_delivery_stop_time,ad_snapshot_url,currency,"
        "demographic_distribution,funding_entity,impressions,page_id,page_name,publisher_platform,"
        "region_distribution,spend"
    ),
) -> str:
    """Search ads in Meta's public Ads Library endpoint."""
    if not str(search_terms or "").strip():
        return json.dumps({"error": "search_terms parameter is required"}, indent=2)

    if not isinstance(ad_reached_countries, list) or not ad_reached_countries:
        return json.dumps({"error": "ad_reached_countries parameter is required"}, indent=2)

    countries = [str(code).strip().upper() for code in ad_reached_countries if str(code).strip()]
    if not countries:
        return json.dumps({"error": "ad_reached_countries parameter is required"}, indent=2)

    payload = {
        "search_terms": search_terms,
        "ad_reached_countries": countries,
        "ad_type": _normalize_ad_type(ad_type),
        "page_size": int(page_size),
        "fields": fields,
    }
    if page_cursor:
        payload["page_cursor"] = page_cursor

    result = await make_api_request("ads_archive", meta_access_token, payload, method="GET")
    return json.dumps(result, indent=2)
