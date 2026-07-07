# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

import json
import sys
from pathlib import Path
from unittest.mock import AsyncMock, patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from armavita_meta_ads_mcp.core.adset_tools import create_ad_set, update_ad_set


def _inner(raw: str) -> dict:
    payload = json.loads(raw)
    if isinstance(payload.get("data"), str):
        return json.loads(payload["data"])
    return payload


@pytest.mark.asyncio
async def test_create_adset_serializes_placement_soft_opt_out_object():
    with patch("armavita_meta_ads_mcp.core.adset_tools.make_api_request", new_callable=AsyncMock) as mock_api, patch(
        "armavita_meta_ads_mcp.core.adset_tools._parent_campaign_bid_strategy", new_callable=AsyncMock
    ) as mock_parent:
        mock_parent.return_value = None
        mock_api.return_value = {"success": True, "id": "new_adset"}

        placement = {"facebook_positions": ["marketplace"], "instagram_positions": ["stream"]}
        raw = await create_ad_set(
            ad_account_id="act_1",
            campaign_id="cmp_1",
            name="Placement adset",
            optimization_goal="LINK_CLICKS",
            billing_event="IMPRESSIONS",
            meta_access_token="token",
            targeting={"geo_locations": {"countries": ["US"]}},
            placement_soft_opt_out=placement,
        )

    payload = json.loads(raw)
    assert payload["success"] is True

    sent = json.loads(mock_api.call_args.args[2]["placement_soft_opt_out"])
    assert sent == placement


@pytest.mark.asyncio
async def test_create_adset_rejects_invalid_placement_soft_opt_out_shape():
    raw = await create_ad_set(
        ad_account_id="act_1",
        campaign_id="cmp_1",
        name="Bad placement",
        optimization_goal="LINK_CLICKS",
        billing_event="IMPRESSIONS",
        meta_access_token="token",
        placement_soft_opt_out=["marketplace"],
    )
    payload = _inner(raw)
    assert "error" in payload


@pytest.mark.asyncio
async def test_update_adset_serializes_attribution_spec():
    with patch("armavita_meta_ads_mcp.core.adset_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"success": True}
        raw = await update_ad_set(
            ad_set_id="adset_1",
            meta_access_token="token",
            attribution_spec=[{"event_type": "CLICK_THROUGH", "window_days": 7}],
        )

    payload = json.loads(raw)
    assert payload["success"] is True
    assert isinstance(mock_api.call_args.args[2]["attribution_spec"], str)


@pytest.mark.asyncio
async def test_create_adset_strips_whitespace_in_placement_soft_opt_out():
    with patch("armavita_meta_ads_mcp.core.adset_tools.make_api_request", new_callable=AsyncMock) as mock_api, patch(
        "armavita_meta_ads_mcp.core.adset_tools._parent_campaign_bid_strategy", new_callable=AsyncMock
    ) as mock_parent:
        mock_parent.return_value = None
        mock_api.return_value = {"id": "adset_1"}

        await create_ad_set(
            ad_account_id="act_1",
            campaign_id="cmp_1",
            name="Placement",
            optimization_goal="LINK_CLICKS",
            billing_event="IMPRESSIONS",
            meta_access_token="token",
            targeting={"geo_locations": {"countries": ["US"]}},
            placement_soft_opt_out={"facebook_positions": [" marketplace "]},
        )

    sent = json.loads(mock_api.call_args.args[2]["placement_soft_opt_out"])
    assert sent == {"facebook_positions": ["marketplace"]}


@pytest.mark.asyncio
async def test_update_adset_requires_promoted_object_for_app_installs():
    raw = await update_ad_set(
        ad_set_id="adset_1",
        meta_access_token="token",
        optimization_goal="APP_INSTALLS",
    )
    payload = _inner(raw)
    assert "promoted_object" in payload["error"]
