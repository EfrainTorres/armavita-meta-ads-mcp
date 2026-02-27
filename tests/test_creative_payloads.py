# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

import json
import sys
from pathlib import Path
from unittest.mock import AsyncMock, patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from armavita_meta_ads_mcp.core.ad_tools import create_ad_creative


@pytest.mark.asyncio
async def test_create_ad_creative_simple_image_payload_uses_story_spec_fields():
    with patch("armavita_meta_ads_mcp.core.ad_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.side_effect = [{"id": "creative_1"}, {"id": "creative_1", "name": "Creative"}]

        raw = await create_ad_creative(
            ad_account_id="act_123",
            ad_image_hash="hash_1",
            facebook_page_id="123456",
            link_url="https://example.com",
            primary_text="Primary copy",
            headline_text="Headline",
            description_text="Description text",
            call_to_action_type="LEARN_MORE",
            lead_form_id="lead_form_1",
            meta_access_token="token",
        )

    payload = json.loads(raw)
    assert payload["success"] is True

    params = mock_api.call_args_list[0].args[2]
    story = params["object_story_spec"]
    assert story["page_id"] == "123456"
    assert story["link_data"]["message"] == "Primary copy"
    assert story["link_data"]["description"] == "Description text"
    assert story["link_data"]["call_to_action"]["value"]["lead_gen_form_id"] == "lead_form_1"


@pytest.mark.asyncio
async def test_create_ad_creative_variant_payload_uses_asset_feed_spec():
    with patch("armavita_meta_ads_mcp.core.ad_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.side_effect = [{"id": "creative_2"}, {"id": "creative_2", "name": "Creative"}]

        raw = await create_ad_creative(
            ad_account_id="act_123",
            facebook_page_id="123456",
            link_url="https://example.com",
            ad_image_hashes=["hash_a", "hash_b"],
            primary_text_variants=["Copy A", "Copy B"],
            headline_variants=["Headline A"],
            description_variants=["Desc A"],
            meta_access_token="token",
        )

    payload = json.loads(raw)
    assert payload["success"] is True

    params = mock_api.call_args_list[0].args[2]
    feed = params["asset_feed_spec"]
    assert feed["images"] == [{"hash": "hash_a"}, {"hash": "hash_b"}]
    assert feed["bodies"] == [{"text": "Copy A"}, {"text": "Copy B"}]
    assert feed["titles"] == [{"text": "Headline A"}]
    assert feed["descriptions"] == [{"text": "Desc A"}]