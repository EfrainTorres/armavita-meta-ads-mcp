# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

import json
import sys
from pathlib import Path
from unittest.mock import AsyncMock, patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from armavita_meta_ads_mcp.core.campaign_tools import create_campaign


def _inner(raw: str) -> dict:
    payload = json.loads(raw)
    if isinstance(payload.get("data"), str):
        return json.loads(payload["data"])
    return payload


@pytest.mark.asyncio
async def test_create_campaign_requires_budget_mode():
    raw = await create_campaign(
        ad_account_id="act_1",
        name="No budget",
        objective="OUTCOME_TRAFFIC",
        meta_access_token="token",
    )
    payload = _inner(raw)
    assert "error" in payload
    assert "Campaign budget required" in payload["error"]


@pytest.mark.asyncio
async def test_create_campaign_apply_default_budget_opt_in():
    with patch("armavita_meta_ads_mcp.core.campaign_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"id": "cmp_1"}
        raw = await create_campaign(
            ad_account_id="act_1",
            name="Default budget",
            objective="OUTCOME_TRAFFIC",
            meta_access_token="token",
            apply_default_budget=True,
        )

    payload = _inner(raw)
    assert payload.get("budget_default_applied") == "daily_budget=1000"
    assert mock_api.call_args.args[2]["daily_budget"] == "1000"
