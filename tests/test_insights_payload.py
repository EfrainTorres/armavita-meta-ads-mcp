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
    assert params["date_range"] == json.dumps(custom_range)
