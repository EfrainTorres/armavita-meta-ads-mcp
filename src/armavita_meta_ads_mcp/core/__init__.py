# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

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
    upload_ad_video_asset,
    list_ad_images,
    list_ad_videos,
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
from .account_controls_tools import get_account_controls, update_account_controls
from .audience_tools import (
    create_custom_audience,
    create_lookalike_audience,
    delete_custom_audience,
    list_custom_audiences,
    manage_custom_audience_users,
    read_custom_audience,
    update_custom_audience,
)
from .capi_tools import list_business_datasets, read_dataset_quality, send_capi_events
from .catalog_tools import (
    batch_products,
    list_product_catalogs,
    list_product_sets,
    list_products,
    upsert_product,
)
from .conversion_tools import (
    create_custom_conversion,
    delete_custom_conversion,
    list_custom_conversions,
    read_custom_conversion,
    update_custom_conversion,
)
from .derived_metrics_tools import list_ad_custom_derived_metrics
from .insights_async_tools import (
    create_insights_job,
    read_insights_job,
    read_insights_job_results,
)
from .partnership_tools import grant_branded_content_ad_permission, list_branded_content_ad_permissions
from .reach_frequency_tools import (
    create_reach_frequency_prediction,
    list_reach_frequency_predictions,
    read_reach_frequency_prediction,
)
from .recommendation_tools import apply_recommendation, list_recommendations
from .threads_tools import create_threads_account, get_threads_account

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
    "upload_ad_video_asset",
    "list_ad_images",
    "list_ad_videos",
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
    "get_threads_account",
    "create_threads_account",
    "list_recommendations",
    "apply_recommendation",
    "get_account_controls",
    "update_account_controls",
    "list_custom_audiences",
    "read_custom_audience",
    "create_custom_audience",
    "update_custom_audience",
    "delete_custom_audience",
    "manage_custom_audience_users",
    "create_lookalike_audience",
    "send_capi_events",
    "read_dataset_quality",
    "list_business_datasets",
    "list_product_catalogs",
    "list_products",
    "upsert_product",
    "batch_products",
    "list_product_sets",
    "list_ad_custom_derived_metrics",
    "list_custom_conversions",
    "read_custom_conversion",
    "create_custom_conversion",
    "update_custom_conversion",
    "delete_custom_conversion",
    "create_insights_job",
    "read_insights_job",
    "read_insights_job_results",
    "list_branded_content_ad_permissions",
    "grant_branded_content_ad_permission",
    "create_reach_frequency_prediction",
    "list_reach_frequency_predictions",
    "read_reach_frequency_prediction",
]