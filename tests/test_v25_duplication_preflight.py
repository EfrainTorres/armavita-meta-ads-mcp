# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

import json
import sys
from pathlib import Path
from unittest.mock import AsyncMock, patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from armavita_meta_ads_mcp.core.campaign_tools import create_campaign
from armavita_meta_ads_mcp.core.duplication_tools import DuplicationError, clone_campaign
from armavita_meta_ads_mcp.core.meta_v25_guards import detect_deprecated_advantage_plus_block


def _inner(raw: str) -> dict:
    payload = json.loads(raw)
    if isinstance(payload.get("data"), str):
        return json.loads(payload["data"])
    return payload


def test_detect_deprecated_advantage_plus_block_flags_asc():
    reason = detect_deprecated_advantage_plus_block(
        {
            "objective": "OUTCOME_SALES",
            "smart_promotion_type": "ADVANTAGE_PLUS_SHOPPING",
        }
    )
    assert reason
    assert "SHOPPING" in reason


@pytest.mark.asyncio
async def test_create_campaign_blocks_deprecated_advantage_plus():
    raw = await create_campaign(
        ad_account_id="act_1",
        name="ASC",
        objective="OUTCOME_SALES",
        meta_access_token="token",
        use_ad_set_level_budgets=True,
        smart_promotion_type="ADVANTAGE_PLUS_SHOPPING",
    )
    payload = _inner(raw)
    assert payload["error"] == "Deprecated Advantage+ campaign type"


@pytest.mark.asyncio
async def test_clone_campaign_preflight_blocks_deprecated_advantage_plus():
    with patch("armavita_meta_ads_mcp.core.duplication_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {
            "id": "cmp_1",
            "name": "Old ASC",
            "objective": "OUTCOME_SALES",
            "smart_promotion_type": "ADVANTAGE_PLUS_SHOPPING",
        }
        with pytest.raises(DuplicationError) as exc_info:
            await clone_campaign(campaign_id="cmp_1", meta_access_token="EAAGtest")

    payload = json.loads(str(exc_info.value))
    assert payload["error"] == "v25_blocked_operation"
    assert "ADVANTAGE_PLUS_SHOPPING" in payload["details"]["smart_promotion_type"]
