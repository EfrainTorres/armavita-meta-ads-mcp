import json
import sys
from pathlib import Path
from unittest.mock import AsyncMock, patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from armavita_meta_ads_mcp.core.ad_tools import list_ad_previews


@pytest.mark.asyncio
async def test_list_ad_previews_retries_with_default_format_when_required():
    required_error = {
        "error": {
            "message": "HTTP Error: 400",
            "details": {"error": {"message": "(#100) For field 'previews': The parameter ad_format is required"}},
        }
    }
    success_payload = {"data": [{"body": "<div>preview</div>"}]}

    with patch("armavita_meta_ads_mcp.core.ad_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.side_effect = [required_error, success_payload]

        raw = await list_ad_previews(ad_id="123", meta_access_token="token")

    payload = json.loads(raw)
    assert payload["data"][0]["body"] == "<div>preview</div>"
    assert payload["request_context"]["auto_selected"] is True

    first_params = mock_api.call_args_list[0].args[2]
    second_params = mock_api.call_args_list[1].args[2]
    assert "ad_format" not in first_params
    assert second_params["ad_format"] == "DESKTOP_FEED_STANDARD"


@pytest.mark.asyncio
async def test_list_ad_previews_respects_explicit_ad_format_without_retry():
    error_payload = {"error": {"message": "HTTP Error: 400"}}

    with patch("armavita_meta_ads_mcp.core.ad_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = error_payload

        raw = await list_ad_previews(
            ad_id="123",
            meta_access_token="token",
            ad_format="MOBILE_FEED_STANDARD",
        )

    payload = json.loads(raw)
    assert payload["error"]["message"] == "HTTP Error: 400"
    assert mock_api.await_count == 1
    call_params = mock_api.call_args.args[2]
    assert call_params["ad_format"] == "MOBILE_FEED_STANDARD"

