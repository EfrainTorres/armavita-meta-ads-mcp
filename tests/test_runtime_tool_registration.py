import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from armavita_meta_ads_mcp.core.mcp_runtime import _import_tool_modules, mcp_server


EXPECTED_V1_TOOLS = {
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
    "create_ad",
    "update_ad",
    "list_ad_creatives",
    "read_ad_creative",
    "create_ad_creative",
    "update_ad_creative",
    "upload_ad_image_asset",
    "read_ad_image",
    "export_ad_image_file",
    "search_pages",
    "list_account_pages",
    "list_insights",
    "create_report",
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
    "search_web_content",
    "read_web_content",
}


def _runtime_tool_names() -> set[str]:
    _import_tool_modules()
    return set(mcp_server._tool_manager._tools.keys())


def test_runtime_tool_surface_is_exact_v1_contract():
    assert _runtime_tool_names() == EXPECTED_V1_TOOLS


def test_runtime_includes_export_image_tool():
    assert "export_ad_image_file" in _runtime_tool_names()
