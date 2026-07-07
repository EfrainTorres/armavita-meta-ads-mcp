# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

import json
import sys
from pathlib import Path
from unittest.mock import AsyncMock, patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from armavita_meta_ads_mcp.core.conversion_tools import list_custom_conversions


@pytest.mark.asyncio
async def test_list_custom_conversions_hits_customconversions_edge():
    with patch("armavita_meta_ads_mcp.core.conversion_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"data": []}
        raw = await list_custom_conversions(ad_account_id="act_1", meta_access_token="token")

    endpoint = mock_api.call_args.args[0]
    assert endpoint.endswith("/customconversions")
    payload = json.loads(raw)
    assert payload == {"data": []}
