"""OpenAI MCP Deep Research tools for Meta Ads API."""

import json
import os
import re
from typing import Any, Dict, List, Optional, Tuple

from . import auth_state as auth
from .graph_client import make_api_request, meta_api_tool
from .mcp_runtime import mcp_server
from .media_helpers import logger


_SUPPORTED_PREFIXES = ("account", "campaign", "ad", "page", "business")


def _matches_query(text: str, query_terms: List[str]) -> bool:
    haystack = (text or "").lower()
    return any(term in haystack for term in query_terms)


class MetaAdsDataManager:
    """Manages Meta Ads data for MCP search and record-read operations."""

    def __init__(self):
        self._cache: Dict[str, Dict[str, Any]] = {}
        logger.debug("MetaAdsDataManager initialized")

    async def _load_ad_accounts(self, meta_access_token: str, page_size: int = 200) -> List[Dict[str, Any]]:
        try:
            data = await make_api_request(
                "me/adaccounts",
                meta_access_token,
                {
                    "fields": "id,name,account_id,account_status,amount_spent,balance,currency,business_city,business_country_code",
                    "page_size": page_size,
                },
            )
            if isinstance(data, dict) and isinstance(data.get("data"), list):
                return [item for item in data["data"] if isinstance(item, dict)]
            return []
        except Exception as e:
            logger.error(f"Error fetching ad accounts: {e}")
            return []

    async def _load_campaigns(self, meta_access_token: str, ad_account_id: str, page_size: int = 25) -> List[Dict[str, Any]]:
        try:
            data = await make_api_request(
                f"{ad_account_id}/campaigns",
                meta_access_token,
                {
                    "fields": "id,name,status,objective,daily_budget,lifetime_budget,start_time,stop_time,created_time,updated_time",
                    "page_size": page_size,
                },
            )
            if isinstance(data, dict) and isinstance(data.get("data"), list):
                return [item for item in data["data"] if isinstance(item, dict)]
            return []
        except Exception as e:
            logger.error(f"Error fetching campaigns for {ad_account_id}: {e}")
            return []

    async def _load_ads(self, meta_access_token: str, ad_account_id: str, page_size: int = 25) -> List[Dict[str, Any]]:
        try:
            data = await make_api_request(
                f"{ad_account_id}/ads",
                meta_access_token,
                {
                    "fields": "id,name,status,creative,targeting,bid_amount,created_time,updated_time",
                    "page_size": page_size,
                },
            )
            if isinstance(data, dict) and isinstance(data.get("data"), list):
                return [item for item in data["data"] if isinstance(item, dict)]
            return []
        except Exception as e:
            logger.error(f"Error fetching ads for {ad_account_id}: {e}")
            return []

    async def _get_pages_for_account(self, meta_access_token: str, ad_account_id: str) -> List[Dict[str, Any]]:
        try:
            from .ad_tools import _discover_pages_for_account

            normalized_account_id = ad_account_id if ad_account_id.startswith("act_") else f"act_{ad_account_id}"
            page_discovery_result = await _discover_pages_for_account(normalized_account_id, meta_access_token)
            if not page_discovery_result.get("success"):
                return []
            return [
                {
                    "id": page_discovery_result.get("facebook_page_id"),
                    "name": page_discovery_result.get("page_name", "Unknown"),
                    "source": page_discovery_result.get("source", "unknown"),
                    "ad_account_id": normalized_account_id,
                }
            ]
        except Exception as e:
            logger.error(f"Error fetching pages for {ad_account_id}: {e}")
            return []

    async def _get_businesses(self, meta_access_token: str, meta_user_id: str = "me", page_size: int = 25) -> List[Dict[str, Any]]:
        try:
            data = await make_api_request(
                f"{meta_user_id}/businesses",
                meta_access_token,
                {
                    "fields": "id,name,created_time,verification_status",
                    "page_size": page_size,
                },
            )
            if isinstance(data, dict) and isinstance(data.get("data"), list):
                return [item for item in data["data"] if isinstance(item, dict)]
            return []
        except Exception as e:
            logger.error(f"Error fetching businesses: {e}")
            return []

    def _build_account_record(self, account: Dict[str, Any]) -> Dict[str, Any]:
        record_id = f"account:{account.get('id')}"
        return {
            "id": record_id,
            "type": "account",
            "title": f"Ad Account: {account.get('name', 'Unnamed Account')}",
            "text": (
                f"Meta Ads Account {account.get('name', 'Unnamed')} (ID: {account.get('id', 'N/A')}) - "
                f"Status: {account.get('account_status', 'Unknown')}, Currency: {account.get('currency', 'Unknown')}, "
                f"Spent: ${account.get('amount_spent', 0)}, Balance: ${account.get('balance', 0)}"
            ),
            "metadata": {
                "ad_account_id": account.get("id"),
                "account_name": account.get("name"),
                "status": account.get("account_status"),
                "currency": account.get("currency"),
                "business_location": f"{account.get('business_city', '')}, {account.get('business_country_code', '')}".strip(
                    ", "
                ),
                "data_type": "meta_ads_account",
            },
            "raw_data": account,
        }

    def _build_campaign_record(self, campaign: Dict[str, Any], account: Optional[Dict[str, Any]]) -> Dict[str, Any]:
        account_name = account.get("name", "Unknown") if isinstance(account, dict) else "Unknown"
        ad_account_id = account.get("id") if isinstance(account, dict) else None
        record_id = f"campaign:{campaign.get('id')}"
        return {
            "id": record_id,
            "type": "campaign",
            "title": f"Campaign: {campaign.get('name', 'Unnamed Campaign')}",
            "text": (
                f"Meta Ads Campaign {campaign.get('name', 'Unnamed')} (ID: {campaign.get('id', 'N/A')}) - "
                f"Objective: {campaign.get('objective', 'Unknown')}, Status: {campaign.get('status', 'Unknown')}, "
                f"Daily Budget: ${campaign.get('daily_budget', 'Not set')}, Account: {account_name}"
            ),
            "metadata": {
                "campaign_id": campaign.get("id"),
                "campaign_name": campaign.get("name"),
                "objective": campaign.get("objective"),
                "status": campaign.get("status"),
                "ad_account_id": ad_account_id,
                "account_name": account_name,
                "data_type": "meta_ads_campaign",
            },
            "raw_data": campaign,
        }

    def _build_ad_record(self, ad: Dict[str, Any], account: Optional[Dict[str, Any]]) -> Dict[str, Any]:
        account_name = account.get("name", "Unknown") if isinstance(account, dict) else "Unknown"
        ad_account_id = account.get("id") if isinstance(account, dict) else None
        record_id = f"ad:{ad.get('id')}"
        return {
            "id": record_id,
            "type": "ad",
            "title": f"Ad: {ad.get('name', 'Unnamed Ad')}",
            "text": (
                f"Meta Ad {ad.get('name', 'Unnamed')} (ID: {ad.get('id', 'N/A')}) - "
                f"Status: {ad.get('status', 'Unknown')}, Bid Amount: ${ad.get('bid_amount', 'Not set')}, "
                f"Account: {account_name}"
            ),
            "metadata": {
                "ad_id": ad.get("id"),
                "ad_name": ad.get("name"),
                "status": ad.get("status"),
                "ad_account_id": ad_account_id,
                "account_name": account_name,
                "data_type": "meta_ads_ad",
            },
            "raw_data": ad,
        }

    def _build_page_record(self, page: Dict[str, Any], account: Optional[Dict[str, Any]]) -> Dict[str, Any]:
        account_name = account.get("name", "Unknown") if isinstance(account, dict) else "Unknown"
        ad_account_id = account.get("id") if isinstance(account, dict) else page.get("ad_account_id")
        record_id = f"page:{page.get('id')}"
        return {
            "id": record_id,
            "type": "page",
            "title": f"Facebook Page: {page.get('name', 'Unnamed Page')}",
            "text": (
                f"Facebook Page {page.get('name', 'Unnamed')} (ID: {page.get('id', 'N/A')}) - "
                f"Source: {page.get('source', 'Unknown')}, Account: {account_name}"
            ),
            "metadata": {
                "facebook_page_id": page.get("id"),
                "page_name": page.get("name"),
                "source": page.get("source"),
                "ad_account_id": ad_account_id,
                "account_name": account_name,
                "data_type": "meta_ads_page",
            },
            "raw_data": page,
        }

    def _build_business_record(self, business: Dict[str, Any]) -> Dict[str, Any]:
        record_id = f"business:{business.get('id')}"
        return {
            "id": record_id,
            "type": "business",
            "title": f"Business: {business.get('name', 'Unnamed Business')}",
            "text": (
                f"Meta Business {business.get('name', 'Unnamed')} (ID: {business.get('id', 'N/A')}) - "
                f"Created: {business.get('created_time', 'Unknown')}, "
                f"Verification: {business.get('verification_status', 'Unknown')}"
            ),
            "metadata": {
                "business_id": business.get("id"),
                "business_name": business.get("name"),
                "created_time": business.get("created_time"),
                "verification_status": business.get("verification_status"),
                "data_type": "meta_ads_business",
            },
            "raw_data": business,
        }

    def _cache_record(self, record: Dict[str, Any]) -> None:
        record_id = record.get("id")
        if isinstance(record_id, str) and record_id:
            self._cache[record_id] = record

    async def search_records(self, query: str, meta_access_token: str) -> List[str]:
        """Search Meta Ads data and return matching record IDs."""
        logger.info(f"Searching Meta Ads data with query: {query}")

        query_terms = re.findall(r"\w+", query.lower())
        if not query_terms:
            return []

        matching_ids: List[str] = []
        seen: set[str] = set()

        def add_match(record: Dict[str, Any]) -> None:
            record_id = record["id"]
            self._cache_record(record)
            if record_id not in seen:
                seen.add(record_id)
                matching_ids.append(record_id)

        try:
            accounts = await self._load_ad_accounts(meta_access_token, page_size=200)

            # Always scan campaigns for top N accounts, independent of account text match.
            campaign_scan_limit = int(os.environ.get("META_MCP_DR_CAMPAIGN_SCAN_LIMIT", "10"))
            campaign_scan_accounts = accounts[: max(campaign_scan_limit, 0)]

            for account in accounts:
                account_record = self._build_account_record(account)
                if _matches_query(account_record["text"], query_terms):
                    add_match(account_record)
                else:
                    self._cache_record(account_record)

            for account in campaign_scan_accounts:
                campaigns = await self._load_campaigns(meta_access_token, account.get("id", ""), page_size=25)
                for campaign in campaigns:
                    campaign_record = self._build_campaign_record(campaign, account)
                    if _matches_query(campaign_record["text"], query_terms):
                        add_match(campaign_record)
                    else:
                        self._cache_record(campaign_record)

            if any(term in {"ad", "ads", "advertisement", "creative"} for term in query_terms):
                for account in accounts[:3]:
                    ads = await self._load_ads(meta_access_token, account.get("id", ""), page_size=25)
                    for ad in ads:
                        ad_record = self._build_ad_record(ad, account)
                        if _matches_query(ad_record["text"], query_terms):
                            add_match(ad_record)
                        else:
                            self._cache_record(ad_record)

            if any(term in {"page", "pages", "facebook", "fb"} for term in query_terms):
                for account in accounts[:5]:
                    pages = await self._get_pages_for_account(meta_access_token, account.get("id", ""))
                    for page in pages:
                        page_record = self._build_page_record(page, account)
                        if _matches_query(page_record["text"], query_terms):
                            add_match(page_record)
                        else:
                            self._cache_record(page_record)

            if any(term in {"business", "businesses", "company", "companies"} for term in query_terms):
                businesses = await self._get_businesses(meta_access_token, page_size=25)
                for business in businesses:
                    business_record = self._build_business_record(business)
                    if _matches_query(business_record["text"], query_terms):
                        add_match(business_record)
                    else:
                        self._cache_record(business_record)

        except Exception as e:
            logger.error(f"Error during search_web_content operation: {e}")
            return []

        logger.info(f"Search completed. Found {len(matching_ids)} matching records")
        return matching_ids[:50]

    @staticmethod
    def _parse_record_id(record_id: str) -> Tuple[str, str]:
        if ":" not in record_id:
            raise ValueError(
                f"Invalid record ID '{record_id}'. Expected format '<prefix>:<id>' with prefixes: {', '.join(_SUPPORTED_PREFIXES)}"
            )

        prefix, raw_id = record_id.split(":", 1)
        prefix = prefix.strip().lower()
        raw_id = raw_id.strip()
        if prefix not in _SUPPORTED_PREFIXES or not raw_id:
            raise ValueError(
                f"Unsupported record ID prefix '{prefix}'. Supported prefixes: {', '.join(_SUPPORTED_PREFIXES)}"
            )
        return prefix, raw_id

    async def _fetch_record_from_api(self, record_id: str, meta_access_token: str) -> Optional[Dict[str, Any]]:
        record_type, raw_id = self._parse_record_id(record_id)

        if record_type == "account":
            normalized = raw_id if raw_id.startswith("act_") else f"act_{raw_id}"
            data = await make_api_request(
                normalized,
                meta_access_token,
                {
                    "fields": "id,name,account_id,account_status,amount_spent,balance,currency,business_city,business_country_code",
                },
            )
            if isinstance(data, dict) and "error" not in data:
                return self._build_account_record(data)
            return None

        if record_type == "campaign":
            data = await make_api_request(
                raw_id,
                meta_access_token,
                {
                    "fields": "id,name,status,objective,daily_budget,lifetime_budget,start_time,stop_time,created_time,updated_time",
                },
            )
            if isinstance(data, dict) and "error" not in data:
                return self._build_campaign_record(data, None)
            return None

        if record_type == "ad":
            data = await make_api_request(
                raw_id,
                meta_access_token,
                {
                    "fields": "id,name,status,creative,targeting,bid_amount,created_time,updated_time",
                },
            )
            if isinstance(data, dict) and "error" not in data:
                return self._build_ad_record(data, None)
            return None

        if record_type == "page":
            data = await make_api_request(
                raw_id,
                meta_access_token,
                {"fields": "id,name,about,category,fan_count,verification_status"},
            )
            if isinstance(data, dict) and "error" not in data:
                return self._build_page_record(data, None)
            return None

        data = await make_api_request(
            raw_id,
            meta_access_token,
            {"fields": "id,name,created_time,verification_status"},
        )
        if isinstance(data, dict) and "error" not in data:
            return self._build_business_record(data)
        return None

    async def fetch_record(self, record_id: str, meta_access_token: Optional[str]) -> Optional[Dict[str, Any]]:
        """Fetch a record by ID. Uses cache first, then Graph API fallback."""
        logger.info(f"Fetching record: {record_id}")

        cached = self._cache.get(record_id)
        if cached:
            logger.debug(f"Record found in cache: {cached.get('type')}")
            return cached

        if not meta_access_token:
            logger.warning("Record not found in cache and no access token available for fallback")
            return None

        fetched = await self._fetch_record_from_api(record_id, meta_access_token)
        if fetched:
            logger.debug("Record fetched from API fallback")
            self._cache_record(fetched)
            return fetched

        logger.warning(f"Record not found via cache or API fallback: {record_id}")
        return None


_data_manager = MetaAdsDataManager()


@mcp_server.tool()
@meta_api_tool
async def search_web_content(
    query: str,
    meta_access_token: Optional[str] = None,
) -> str:
    """Search through Meta Ads data and return matching record IDs."""
    if not query:
        return json.dumps({"error": "query parameter is required", "ids": []}, indent=2)

    try:
        matching_ids = await _data_manager.search_records(query, meta_access_token)
        return json.dumps(
            {
                "ids": matching_ids,
                "query": query,
                "total_results": len(matching_ids),
            },
            indent=2,
        )
    except Exception as e:
        logger.error(f"Error in search_web_content tool: {e}")
        return json.dumps(
            {
                "error": "Failed to search_web_content Meta Ads data",
                "details": str(e),
                "ids": [],
                "query": query,
            },
            indent=2,
        )


@mcp_server.tool()
async def read_web_content(resource_id: str) -> str:
    """Fetch complete record data by ID using cache-first + API fallback behavior."""
    if not resource_id:
        return json.dumps({"error": "resource_id parameter is required"}, indent=2)

    meta_access_token: Optional[str] = None
    try:
        meta_access_token = await auth.get_current_access_token()
    except Exception as token_error:
        logger.debug(f"No access token for read_web_content fallback: {token_error}")

    try:
        record = await _data_manager.fetch_record(resource_id, meta_access_token)
        if record:
            logger.info(f"Record fetched successfully: {resource_id}")
            return json.dumps(record, indent=2)

        return json.dumps({"error": f"Record not found: {resource_id}", "resource_id": resource_id}, indent=2)
    except ValueError as value_error:
        return json.dumps(
            {
                "error": "invalid_record_id",
                "details": str(value_error),
                "supported_prefixes": list(_SUPPORTED_PREFIXES),
                "resource_id": resource_id,
            },
            indent=2,
        )
    except Exception as e:
        logger.error(f"Error in read_web_content tool: {e}")
        return json.dumps(
            {
                "error": "Failed to read_web_content record",
                "details": str(e),
                "resource_id": resource_id,
            },
            indent=2,
        )
