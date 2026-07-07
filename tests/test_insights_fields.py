# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

import sys
from pathlib import Path
from unittest.mock import AsyncMock, patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from armavita_meta_ads_mcp.core.insight_tools import list_insights


@pytest.mark.asyncio
async def test_list_insights_forwards_custom_fields():
    with patch("armavita_meta_ads_mcp.core.insight_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"data": []}
        await list_insights(
            object_id="act_1",
            meta_access_token="token",
            fields="impressions,spend,custom_metric_name",
        )

    params = mock_api.call_args.args[2]
    assert params["fields"] == "impressions,spend,custom_metric_name"
