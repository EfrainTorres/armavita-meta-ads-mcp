# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

import json
import sys
from pathlib import Path
from unittest.mock import AsyncMock, patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from armavita_meta_ads_mcp.core.adset_tools import create_ad_set, update_ad_set


@pytest.mark.asyncio
async def test_update_adset_serializes_dict_fields_before_request():
    with patch("armavita_meta_ads_mcp.core.adset_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"success": True, "id": "adset_1"}

        raw = await update_ad_set(
            ad_set_id="adset_1",
            meta_access_token="token",
            targeting={"age_min": 21, "geo_locations": {"countries": ["US"]}},
            bid_constraints={"roas_average_floor": 20000},
        )

    payload = json.loads(raw)
    assert payload["success"] is True

    params = mock_api.call_args.args[2]
    assert isinstance(params["targeting"], str)
    assert isinstance(params["bid_constraints"], str)


@pytest.mark.asyncio
async def test_create_adset_serializes_targeting_and_promoted_object():
    with patch("armavita_meta_ads_mcp.core.adset_tools.make_api_request", new_callable=AsyncMock) as mock_api, patch(
        "armavita_meta_ads_mcp.core.adset_tools._parent_campaign_bid_strategy", new_callable=AsyncMock
    ) as mock_parent:
        mock_parent.return_value = None
        mock_api.return_value = {"success": True, "id": "new_adset"}

        raw = await create_ad_set(
            ad_account_id="act_1",
            campaign_id="cmp_1",
            name="New Adset",
            optimization_goal="LINK_CLICKS",
            billing_event="IMPRESSIONS",
            meta_access_token="token",
            targeting={"geo_locations": {"countries": ["US"]}},
            promoted_object={"application_id": "123", "object_store_url": "https://apps.apple.com/app/id123"},
        )

    payload = json.loads(raw)
    assert payload["success"] is True

    params = mock_api.call_args.args[2]
    assert isinstance(params["targeting"], str)
    assert isinstance(params["promoted_object"], str)


@pytest.mark.asyncio
async def test_create_adset_rejects_missing_bid_amount_for_bid_cap():
    raw = await create_ad_set(
        ad_account_id="act_1",
        campaign_id="cmp_1",
        name="Bid cap adset",
        optimization_goal="LINK_CLICKS",
        billing_event="IMPRESSIONS",
        bid_strategy="COST_CAP",
        meta_access_token="token",
    )
    payload = json.loads(raw)
    inner = json.loads(payload["data"]) if "data" in payload else payload
    assert "error" in inner
    assert "bid_amount is required" in inner["error"]