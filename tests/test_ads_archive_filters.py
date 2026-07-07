# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

import sys
from pathlib import Path
from unittest.mock import AsyncMock, patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from armavita_meta_ads_mcp.core.ads_archive_tools import search_ads_archive


@pytest.mark.asyncio
async def test_search_ads_archive_forwards_v25_filters():
    with patch("armavita_meta_ads_mcp.core.ads_archive_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"data": []}
        await search_ads_archive(
            search_terms="coffee",
            ad_reached_countries=["US"],
            meta_access_token="token",
            ad_active_status="ACTIVE",
            ad_delivery_date_min="2024-01-01",
            ad_delivery_date_max="2024-12-31",
            search_page_ids=["123"],
            search_type="KEYWORD_UNORDERED",
            languages=["en"],
            media_type="IMAGE",
            publisher_platforms=["facebook", "instagram"],
        )

    params = mock_api.call_args.args[2]
    assert params["ad_active_status"] == "ACTIVE"
    assert params["ad_delivery_date_min"] == "2024-01-01"
    assert params["search_page_ids"] == ["123"]
    # publisher_platforms enum values must be uppercased to match Meta (FACEBOOK/INSTAGRAM)
    assert params["publisher_platforms"] == ["FACEBOOK", "INSTAGRAM"]
    # default fields must use the valid plural v25 field names, never the singular forms
    assert "ad_creative_bodies" in params["fields"]
    assert "ad_creative_body" not in params["fields"]
    assert "funding_entity" not in params["fields"]
