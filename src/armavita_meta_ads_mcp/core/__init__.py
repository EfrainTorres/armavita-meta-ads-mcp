"""Public exports for armavita-meta-ads-mcp core modules."""

from .account_tools import read_ad_account, list_ad_accounts
from .ad_tools import (
    create_ad,
    create_ad_creative,
    export_ad_image_file,
    list_account_pages,
    list_ad_creatives,
    read_ad,
    read_ad_image,
    list_ad_previews,
    list_ads,
    read_ad_creative,
    search_pages,
    update_ad,
    update_ad_creative,
    upload_ad_image_asset,
)
from .ads_archive_tools import search_ads_archive
from .adset_tools import create_ad_set, read_ad_set, list_ad_sets, update_ad_set
from .auth_state import login
from .budget_schedule_tools import create_campaign_budget_schedule
from .campaign_tools import create_campaign, read_campaign, list_campaigns, update_campaign
from .duplication_tools import clone_ad, clone_ad_set, clone_campaign, clone_ad_creative
from .insight_tools import list_insights
from .mcp_runtime import login_cli, main, mcp_server
from .report_tools import create_report
from .research_tools import read_web_content, search_web_content
from .targeting_tools import (
    estimate_audience_size,
    suggest_interests,
    search_behaviors,
    search_demographics,
    search_geo_locations,
    search_interests,
)

__all__ = [
    "mcp_server",
    "main",
    "login_cli",
    "login",
    "list_ad_accounts",
    "read_ad_account",
    "list_campaigns",
    "read_campaign",
    "create_campaign",
    "update_campaign",
    "list_ad_sets",
    "read_ad_set",
    "create_ad_set",
    "update_ad_set",
    "list_ads",
    "read_ad",
    "list_ad_previews",
    "read_ad_creative",
    "create_ad",
    "list_ad_creatives",
    "read_ad_image",
    "export_ad_image_file",
    "update_ad",
    "upload_ad_image_asset",
    "create_ad_creative",
    "update_ad_creative",
    "search_pages",
    "list_account_pages",
    "list_insights",
    "create_campaign_budget_schedule",
    "search_interests",
    "suggest_interests",
    "estimate_audience_size",
    "search_behaviors",
    "search_demographics",
    "search_geo_locations",
    "clone_campaign",
    "clone_ad_set",
    "clone_ad",
    "clone_ad_creative",
    "search_ads_archive",
    "create_report",
    "search_web_content",
    "read_web_content",
]
