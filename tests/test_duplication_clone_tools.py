# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

import json
import sys
from pathlib import Path
from unittest.mock import AsyncMock, patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from armavita_meta_ads_mcp.core.duplication_tools import clone_ad, clone_campaign


@pytest.mark.asyncio
async def test_clone_campaign_warning_list_has_no_token_option_names():
    with patch("armavita_meta_ads_mcp.core.duplication_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"copied_campaign_id": "cmp_copy_1"}

        raw = await clone_campaign(
            campaign_id="cmp_1",
            meta_access_token="token",
            include_ad_sets=True,
            include_ads=True,
            include_creatives=True,
        )

    payload = json.loads(raw)
    assert payload["success"] is True
    warning_options = [str(item.get("option", "")) for item in payload.get("warnings", [])]
    assert all("token" not in option.lower() for option in warning_options)


@pytest.mark.asyncio
async def test_clone_ad_maps_target_ad_set_id_to_graph_param():
    with patch("armavita_meta_ads_mcp.core.duplication_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"copied_ad_id": "ad_copy_1"}

        raw = await clone_ad(
            ad_id="ad_1",
            meta_access_token="token",
            target_ad_set_id="adset_target_1",
            clone_ad_creative=False,
        )

    payload = json.loads(raw)
    assert payload["success"] is True

    params = mock_api.call_args.args[2]
    assert params["adset_id"] == "adset_target_1"
    assert params["deep_copy"] is False