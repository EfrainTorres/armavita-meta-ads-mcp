# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

import json
import sys
from pathlib import Path
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from armavita_meta_ads_mcp.core.ad_tools import read_ad_image
from armavita_meta_ads_mcp.core.graph_client import McpToolError


@pytest.mark.asyncio
async def test_read_ad_image_uses_single_creative_lookup_in_fallback_path():
    mock_ad_data = {"account_id": "act_1", "creative": {"id": "creative_1"}}
    mock_creative_details = {"id": "creative_1", "name": "No Hash"}
    wrapped_creatives = json.dumps(
        {
            "data": json.dumps(
                {
                    "data": [
                        {
                            "id": "creative_1",
                            "object_story_spec": {"link_data": {"ad_image_hash": "hash_1"}},
                        }
                    ]
                }
            )
        }
    )
    mock_image_data = {"data": [{"hash": "hash_1", "url": "https://example.com/image.jpg"}]}

    mock_pil_image = MagicMock()
    mock_pil_image.mode = "RGB"
    mock_pil_image.convert.return_value = mock_pil_image
    mock_bytes = MagicMock()
    mock_bytes.getvalue.return_value = b"jpeg"

    with patch("armavita_meta_ads_mcp.core.ad_tools.make_api_request", new_callable=AsyncMock) as mock_api, patch(
        "armavita_meta_ads_mcp.core.ad_tools.list_ad_creatives", new_callable=AsyncMock
    ) as mock_get_creatives, patch(
        "armavita_meta_ads_mcp.core.ad_tools.download_image", new_callable=AsyncMock
    ) as mock_download, patch("armavita_meta_ads_mcp.core.ad_tools.PILImage.open") as mock_open, patch(
        "armavita_meta_ads_mcp.core.ad_tools.io.BytesIO"
    ) as mock_io:
        mock_api.side_effect = [mock_ad_data, mock_creative_details, mock_image_data]
        mock_get_creatives.return_value = wrapped_creatives
        mock_download.return_value = b"img"
        mock_open.return_value = mock_pil_image
        mock_io.return_value = mock_bytes

        result = await read_ad_image(ad_id="ad_1", meta_access_token="token")

    assert result is not None
    mock_get_creatives.assert_called_once_with(ad_id="ad_1", meta_access_token="token")


@pytest.mark.asyncio
async def test_read_ad_image_raises_tool_error_when_no_image_sources_found():
    mock_ad_data = {"account_id": "act_1", "creative": {"id": "creative_1"}}
    mock_creative_details = {"id": "creative_1", "name": "No Hash"}

    with patch("armavita_meta_ads_mcp.core.ad_tools.make_api_request", new_callable=AsyncMock) as mock_api, patch(
        "armavita_meta_ads_mcp.core.ad_tools.list_ad_creatives", new_callable=AsyncMock
    ) as mock_get_creatives:
        mock_api.side_effect = [mock_ad_data, mock_creative_details]
        mock_get_creatives.return_value = json.dumps({"data": json.dumps({"data": []})})

        with pytest.raises(McpToolError):
            await read_ad_image(ad_id="ad_1", meta_access_token="token")