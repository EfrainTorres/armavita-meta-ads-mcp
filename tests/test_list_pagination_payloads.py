import json
import sys
from pathlib import Path
from unittest.mock import AsyncMock, patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from armavita_meta_ads_mcp.core.ads_archive_tools import search_ads_archive
from armavita_meta_ads_mcp.core.account_tools import list_ad_accounts
from armavita_meta_ads_mcp.core.ad_tools import list_ad_creatives, list_ads
from armavita_meta_ads_mcp.core.adset_tools import list_ad_sets
from armavita_meta_ads_mcp.core.targeting_tools import (
    search_behaviors,
    search_demographics,
    search_geo_locations,
    search_interests,
    suggest_interests,
)


@pytest.mark.asyncio
async def test_list_ad_accounts_forwards_page_cursor():
    with patch("armavita_meta_ads_mcp.core.account_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"data": [], "paging": {}}
        raw = await list_ad_accounts(meta_access_token="token", page_size=5, page_cursor="cursor_1")

    payload = json.loads(raw)
    assert "data" in payload
    params = mock_api.call_args.args[2]
    assert params["page_size"] == 5
    assert params["page_cursor"] == "cursor_1"


@pytest.mark.asyncio
async def test_list_ad_sets_forwards_page_cursor():
    with patch("armavita_meta_ads_mcp.core.adset_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"data": [], "paging": {}}
        raw = await list_ad_sets(
            ad_account_id="act_1",
            meta_access_token="token",
            page_size=10,
            page_cursor="cursor_2",
        )

    payload = json.loads(raw)
    assert "data" in payload
    params = mock_api.call_args.args[2]
    assert params["page_size"] == 10
    assert params["page_cursor"] == "cursor_2"


@pytest.mark.asyncio
async def test_list_ads_forwards_page_cursor():
    with patch("armavita_meta_ads_mcp.core.ad_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"data": [], "paging": {}}
        raw = await list_ads(
            ad_account_id="act_1",
            meta_access_token="token",
            page_size=10,
            page_cursor="cursor_3",
        )

    payload = json.loads(raw)
    assert "data" in payload
    params = mock_api.call_args.args[2]
    assert params["page_size"] == 10
    assert params["page_cursor"] == "cursor_3"


@pytest.mark.asyncio
async def test_list_ad_creatives_forwards_page_cursor():
    with patch("armavita_meta_ads_mcp.core.ad_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"data": [], "paging": {}}
        raw = await list_ad_creatives(ad_id="ad_1", meta_access_token="token", page_cursor="cursor_4")

    payload = json.loads(raw)
    assert "data" in payload
    params = mock_api.call_args.args[2]
    assert params["page_cursor"] == "cursor_4"


@pytest.mark.asyncio
async def test_search_interests_forwards_page_cursor():
    with patch("armavita_meta_ads_mcp.core.targeting_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"data": [], "paging": {}}
        raw = await search_interests(query="repair", meta_access_token="token", page_size=20, page_cursor="cursor_5")

    payload = json.loads(raw)
    assert "data" in payload
    params = mock_api.call_args.args[2]
    assert params["page_size"] == 20
    assert params["page_cursor"] == "cursor_5"


@pytest.mark.asyncio
async def test_suggest_interests_forwards_page_cursor():
    with patch("armavita_meta_ads_mcp.core.targeting_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"data": [], "paging": {}}
        raw = await suggest_interests(
            interest_list=["Phone repair"],
            meta_access_token="token",
            page_size=15,
            page_cursor="cursor_6",
        )

    payload = json.loads(raw)
    assert "data" in payload
    params = mock_api.call_args.args[2]
    assert params["page_size"] == 15
    assert params["page_cursor"] == "cursor_6"


@pytest.mark.asyncio
async def test_search_behaviors_forwards_page_cursor():
    with patch("armavita_meta_ads_mcp.core.targeting_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"data": [], "paging": {}}
        raw = await search_behaviors(meta_access_token="token", page_size=30, page_cursor="cursor_7")

    payload = json.loads(raw)
    assert "data" in payload
    params = mock_api.call_args.args[2]
    assert params["page_size"] == 30
    assert params["page_cursor"] == "cursor_7"


@pytest.mark.asyncio
async def test_search_demographics_forwards_page_cursor():
    with patch("armavita_meta_ads_mcp.core.targeting_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"data": [], "paging": {}}
        raw = await search_demographics(
            meta_access_token="token",
            demographic_class="demographics",
            page_size=30,
            page_cursor="cursor_8",
        )

    payload = json.loads(raw)
    assert "data" in payload
    params = mock_api.call_args.args[2]
    assert params["page_size"] == 30
    assert params["page_cursor"] == "cursor_8"


@pytest.mark.asyncio
async def test_search_geo_locations_forwards_page_cursor():
    with patch("armavita_meta_ads_mcp.core.targeting_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"data": [], "paging": {}}
        raw = await search_geo_locations(
            query="New York",
            meta_access_token="token",
            page_size=25,
            page_cursor="cursor_9",
        )

    payload = json.loads(raw)
    assert "data" in payload
    params = mock_api.call_args.args[2]
    assert params["page_size"] == 25
    assert params["page_cursor"] == "cursor_9"


@pytest.mark.asyncio
async def test_search_ads_archive_forwards_page_cursor():
    with patch("armavita_meta_ads_mcp.core.ads_archive_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"data": [], "paging": {}}
        raw = await search_ads_archive(
            search_terms="repair",
            ad_reached_countries=["US"],
            meta_access_token="token",
            page_size=25,
            page_cursor="cursor_10",
        )

    payload = json.loads(raw)
    assert "data" in payload
    params = mock_api.call_args.args[2]
    assert params["page_size"] == 25
    assert params["page_cursor"] == "cursor_10"
