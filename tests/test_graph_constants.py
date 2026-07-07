# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

import sys
from pathlib import Path
from unittest.mock import AsyncMock, patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from armavita_meta_ads_mcp.core import graph_constants


def test_normalize_graph_api_version_defaults_to_v25():
    assert graph_constants._normalize_graph_api_version("") == "v25.0"
    assert graph_constants._normalize_graph_api_version("v25.0") == "v25.0"
    assert graph_constants._normalize_graph_api_version("invalid") == "v25.0"


def test_normalize_graph_api_version_accepts_valid_override(monkeypatch):
    monkeypatch.setenv("META_GRAPH_API_VERSION", "v24.0")
    assert graph_constants._normalize_graph_api_version("v24.0") == "v24.0"


@pytest.mark.asyncio
async def test_upload_ad_video_asset_uses_file_url():
    from armavita_meta_ads_mcp.core.ad_tools import upload_ad_video_asset

    with patch("armavita_meta_ads_mcp.core.ad_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"id": "video_123"}
        raw = await upload_ad_video_asset(
            ad_account_id="act_1",
            meta_access_token="token",
            video_source_url="https://example.com/video.mp4",
            name="Promo",
        )

    assert "video_123" in raw
    params = mock_api.call_args.args[2]
    assert params["file_url"] == "https://example.com/video.mp4"
