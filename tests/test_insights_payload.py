# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

import json
import sys
from pathlib import Path
from unittest.mock import AsyncMock, patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from armavita_meta_ads_mcp.core.insight_tools import list_insights


@pytest.mark.asyncio
async def test_list_insights_serializes_custom_time_range():
    with patch("armavita_meta_ads_mcp.core.insight_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"data": [], "paging": {}}

        custom_range = {"since": "2024-01-01", "until": "2024-01-31"}
        raw = await list_insights(
            object_id="act_1",
            meta_access_token="token",
            level="campaign",
            page_size=5,
            page_cursor="cursor_1",
            date_range=custom_range,
        )

    payload = json.loads(raw)
    assert "data" in payload

    params = mock_api.call_args.args[2]
    assert params["page_size"] == 5
    assert params["page_cursor"] == "cursor_1"
    assert params["date_range"] == custom_range


@pytest.mark.asyncio
async def test_list_insights_normalizes_previous_30d_alias():
    with patch("armavita_meta_ads_mcp.core.insight_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"data": [], "paging": {}}

        raw = await list_insights(
            object_id="act_1",
            meta_access_token="token",
            date_range="previous_30d",
        )

    payload = json.loads(raw)
    warnings = payload.get("warnings", [])
    assert any(w.get("code") == "date_preset_alias_applied" for w in warnings if isinstance(w, dict))

    params = mock_api.call_args.args[2]
    assert params["date_preset"] == "last_30d"


@pytest.mark.asyncio
async def test_list_insights_rejects_unknown_date_preset_before_api_call():
    with patch("armavita_meta_ads_mcp.core.insight_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        raw = await list_insights(
            object_id="act_1",
            meta_access_token="token",
            date_range="invalid_preset",
        )

    payload = json.loads(raw)
    inner = json.loads(payload["data"]) if "data" in payload else payload
    assert inner["error"] == "invalid_date_preset"
    mock_api.assert_not_called()


@pytest.mark.asyncio
async def test_list_insights_routes_action_breakdowns_separately():
    with patch("armavita_meta_ads_mcp.core.insight_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"data": [], "paging": {}}

        raw = await list_insights(
            object_id="act_1",
            meta_access_token="token",
            breakdown="action_type,platform_position",
        )

    payload = json.loads(raw)
    warnings = payload.get("warnings", [])
    assert any(w.get("code") == "breakdown_autorouted" for w in warnings if isinstance(w, dict))

    params = mock_api.call_args.args[2]
    assert params["breakdowns"] == "platform_position"
    assert params["action_breakdowns"] == "action_type"