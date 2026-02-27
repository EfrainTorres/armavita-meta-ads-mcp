import json
import sys
from pathlib import Path
from unittest.mock import AsyncMock, patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from armavita_meta_ads_mcp.core.report_tools import create_report


@pytest.mark.asyncio
async def test_create_report_normalizes_previous_30d_alias():
    with patch("armavita_meta_ads_mcp.core.report_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"data": [], "paging": {}}

        raw = await create_report(
            ad_account_id="act_1",
            meta_access_token="token",
            export_format="json",
            date_range="previous_30d",
        )

    payload = json.loads(raw)
    warnings = payload.get("warnings", [])
    assert any(w.get("code") == "date_preset_alias_applied" for w in warnings if isinstance(w, dict))
    params = mock_api.call_args.args[2]
    assert params["date_preset"] == "last_30d"


@pytest.mark.asyncio
async def test_create_report_routes_action_breakdowns_separately():
    with patch("armavita_meta_ads_mcp.core.report_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"data": [], "paging": {}}

        raw = await create_report(
            ad_account_id="act_1",
            meta_access_token="token",
            export_format="json",
            breakdowns=["action_type", "platform_position"],
        )

    payload = json.loads(raw)
    assert payload.get("success") is True
    params = mock_api.call_args.args[2]
    assert params["breakdowns"] == "platform_position"
    assert params["action_breakdowns"] == "action_type"


@pytest.mark.asyncio
async def test_create_report_rejects_unknown_date_preset_before_api_call():
    with patch("armavita_meta_ads_mcp.core.report_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        raw = await create_report(
            ad_account_id="act_1",
            meta_access_token="token",
            export_format="json",
            date_range="not_a_valid_preset",
        )

    payload = json.loads(raw)
    assert payload["error"] == "invalid_parameters"
    details = payload.get("details", {})
    assert details.get("error") == "invalid_date_preset"
    mock_api.assert_not_called()


@pytest.mark.asyncio
async def test_create_report_rejects_unknown_comparison_period_before_api_call():
    with patch("armavita_meta_ads_mcp.core.report_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        raw = await create_report(
            ad_account_id="act_1",
            meta_access_token="token",
            report_type="comparison",
            campaign_ids=["cmp_1"],
            export_format="json",
            comparison_period="not_a_valid_preset",
        )

    payload = json.loads(raw)
    assert payload["error"] == "invalid_parameters"
    details = payload.get("details", {})
    assert details.get("parameter") == "comparison_period"
    assert details.get("error") == "invalid_date_preset"
    mock_api.assert_not_called()
