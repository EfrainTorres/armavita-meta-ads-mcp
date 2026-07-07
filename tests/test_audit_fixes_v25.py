# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""Regression tests for v25 correctness fixes from the June 2026 audit.

Each test pins behavior that was previously broken and uncovered:
  * Ads Library default fields (plural v25 names)
  * update_ad creative payload shape (remaps to creative_id)
  * CAPI / Custom Audience PII normalization (phone digits, plaintext ids)
  * Insights compact mode preserving pixel conversions; level validation
"""

import hashlib
import json
import sys
from pathlib import Path
from unittest.mock import AsyncMock, patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

import httpx

from armavita_meta_ads_mcp.core import (
    audience_tools,
    capi_tools,
    graph_client,
    report_tools,
)
from armavita_meta_ads_mcp.core.ad_tools import update_ad
from armavita_meta_ads_mcp.core.ads_archive_tools import search_ads_archive
from armavita_meta_ads_mcp.core.adset_tools import update_ad_set
from armavita_meta_ads_mcp.core.campaign_tools import create_campaign
from armavita_meta_ads_mcp.core.graph_client import _remap_graph_keys
from armavita_meta_ads_mcp.core.insight_tools import list_insights
from armavita_meta_ads_mcp.core.recommendation_tools import apply_recommendation


def _sha256(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


# --- update_ad creative swap (ads-creatives-1) ---


@pytest.mark.asyncio
async def test_update_ad_creative_passes_dict_that_remaps_to_creative_id():
    with patch("armavita_meta_ads_mcp.core.ad_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"id": "123"}
        await update_ad(ad_id="123", ad_creative_id="987", meta_access_token="token")

    payload = mock_api.call_args.args[2]
    # Must be a plain dict (not a pre-serialized JSON string) so the request
    # pipeline can remap ad_creative_id -> creative_id.
    assert payload["creative"] == {"ad_creative_id": "987"}
    # And the shared remapper must actually convert it to Meta's creative_id.
    assert _remap_graph_keys(payload)["creative"] == {"creative_id": "987"}


# --- CAPI PII normalization (capi-1, capi-2, capi-4) ---


def test_capi_subscription_id_not_hashed():
    out = capi_tools._normalize_user_data({"subscription_id": "sub_abc123"})
    assert out["subscription_id"] == "sub_abc123"


def test_capi_phone_normalized_to_digits_before_hash():
    out = capi_tools._normalize_user_data({"ph": "+1 (555) 123-4567"})
    assert out["ph"] == _sha256("15551234567")


def test_capi_email_lowercased_and_trimmed():
    out = capi_tools._normalize_user_data({"em": "  Alice@Example.COM "})
    assert out["em"] == _sha256("alice@example.com")


def test_capi_uppercase_prehashed_value_is_lowercased_not_rehashed():
    digest = _sha256("alice@example.com").upper()
    out = capi_tools._normalize_user_data({"em": digest})
    assert out["em"] == digest.lower()


# --- Custom Audience PII normalization (audiences-2, audiences-3) ---


def test_audience_madid_and_extern_id_sent_plaintext():
    assert audience_tools._normalize_audience_value("MADID", "AAAA-BBBB-CCCC") == "AAAA-BBBB-CCCC"
    assert audience_tools._normalize_audience_value("EXTERN_ID", "customer-42") == "customer-42"


def test_audience_phone_normalized_to_digits_before_hash():
    assert audience_tools._normalize_audience_value("PHONE", "+1 555-123-4567") == _sha256("15551234567")


def test_audience_email_lowercased_and_trimmed():
    assert audience_tools._normalize_audience_value("EMAIL", " A@B.com ") == _sha256("a@b.com")


# --- Insights compact mode + level validation (insights-2, insights-4) ---


@pytest.mark.asyncio
async def test_compact_preserves_fb_pixel_conversions_but_strips_aggregates():
    response = {
        "data": [
            {
                "actions": [
                    {"action_type": "offsite_conversion.fb_pixel_purchase", "value": "7"},
                    {"action_type": "omni_purchase", "value": "7"},
                    {"action_type": "link_click", "value": "30"},
                ]
            }
        ]
    }
    with patch("armavita_meta_ads_mcp.core.insight_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = response
        raw = await list_insights(object_id="act_1", compact=True, meta_access_token="token")

    actions = json.loads(raw)["data"][0]["actions"]
    types = {a["action_type"] for a in actions}
    assert "offsite_conversion.fb_pixel_purchase" in types  # primary conversion preserved
    assert "link_click" in types
    assert "omni_purchase" not in types  # redundant aggregate stripped


@pytest.mark.asyncio
async def test_invalid_level_rejected_before_api_call():
    with patch("armavita_meta_ads_mcp.core.insight_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        raw = await list_insights(object_id="act_1", level="bogus", meta_access_token="token")
    # An invalid level must short-circuit before any Graph call.
    assert "invalid_level" in raw
    mock_api.assert_not_called()


# --- Campaign: non-fields dropped + warned (campaigns-1, campaigns-2) ---


@pytest.mark.asyncio
async def test_campaign_budget_optimization_and_bid_cap_not_sent_but_warned():
    with patch("armavita_meta_ads_mcp.core.campaign_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"id": "c1"}
        raw = await create_campaign(
            ad_account_id="act_1", name="C", objective="OUTCOME_SALES",
            daily_budget=1000, campaign_budget_optimization=True, bid_cap=500,
            meta_access_token="token",
        )
    payload = mock_api.call_args.args[2]
    assert "campaign_budget_optimization" not in payload  # not a writable Campaign field
    assert "bid_cap" not in payload                        # ad-set field, not campaign
    codes = {w["code"] for w in json.loads(raw).get("warnings", [])}
    assert {"campaign_budget_optimization_not_a_field", "bid_cap_not_a_campaign_field"} <= codes


# --- Ad set: is_dynamic_creative is create-only (adsets-1) ---


@pytest.mark.asyncio
async def test_update_ad_set_drops_is_dynamic_creative_and_warns():
    with patch("armavita_meta_ads_mcp.core.adset_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"id": "as1"}
        raw = await update_ad_set(ad_set_id="as1", is_dynamic_creative=True, status="PAUSED", meta_access_token="token")
    payload = mock_api.call_args.args[2]
    assert "is_dynamic_creative" not in payload
    assert any("is_dynamic_creative" in w for w in json.loads(raw).get("warnings", []))


# --- Ads Library: search_terms optional, at-least-one required (ads-archive-3) ---


@pytest.mark.asyncio
async def test_ads_archive_allows_page_ids_only():
    with patch("armavita_meta_ads_mcp.core.ads_archive_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"data": []}
        raw = await search_ads_archive(ad_reached_countries=["US"], search_page_ids=["123"], meta_access_token="token")
    assert "is required" not in raw  # page-ids-only is valid
    payload = mock_api.call_args.args[2]
    assert "search_terms" not in payload
    assert payload["search_page_ids"] == ["123"]


@pytest.mark.asyncio
async def test_ads_archive_requires_terms_or_page_ids():
    raw = await search_ads_archive(ad_reached_countries=["US"], meta_access_token="token")
    assert "at least one" in raw.lower()


# --- Recommendations: apply is honest, no fabricated POST (recommendations-1) ---


@pytest.mark.asyncio
async def test_apply_recommendation_makes_no_api_call():
    with patch("armavita_meta_ads_mcp.core.recommendation_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        raw = await apply_recommendation(
            object_id="c1", recommendation_data={"blame_field": "daily_budget"}, meta_access_token="token"
        )
    mock_api.assert_not_called()
    payload = json.loads(raw)
    assert payload["status"] == "not_applied"
    assert payload["blame_field"] == "daily_budget"


# --- graph_client: transient retry/backoff (architecture/rate-limiting) ---


def test_retry_helpers():
    assert graph_client._backoff_seconds(0) == 1.0
    assert graph_client._backoff_seconds(2) == 4.0
    assert 500 in graph_client._SERVER_ERROR_STATUS
    assert 4 in graph_client._RETRYABLE_GRAPH_CODES
    assert "GET" in graph_client._IDEMPOTENT_METHODS and "POST" not in graph_client._IDEMPOTENT_METHODS


def _status_client(counter, status):
    """AsyncClient stub whose get/post always raise HTTPStatusError(status)."""
    req = httpx.Request("GET", "https://graph.facebook.com/v25.0/x")
    resp = httpx.Response(status, request=req, json={"error": {"message": "boom"}})

    class _C:
        def __init__(self, *a, **k):
            pass

        async def __aenter__(self):
            return self

        async def __aexit__(self, *a):
            return False

        async def get(self, *a, **k):
            counter[0] += 1
            resp.raise_for_status()

        async def post(self, *a, **k):
            counter[0] += 1
            resp.raise_for_status()

    return _C


@pytest.mark.asyncio
async def test_get_retries_on_500_but_post_does_not():
    # GET (idempotent) retries a 500 to exhaustion.
    gc = [0]
    with patch.object(graph_client.httpx, "AsyncClient", _status_client(gc, 500)), \
         patch.object(graph_client.asyncio, "sleep", new_callable=AsyncMock):
        await graph_client.make_api_request("x", "token", {}, method="GET")
    assert gc[0] == graph_client._MAX_RETRY_ATTEMPTS + 1

    # POST does NOT retry a 500 — avoids duplicate writes (e.g. double-created ads).
    pc = [0]
    with patch.object(graph_client.httpx, "AsyncClient", _status_client(pc, 500)), \
         patch.object(graph_client.asyncio, "sleep", new_callable=AsyncMock):
        await graph_client.make_api_request("x", "token", {}, method="POST")
    assert pc[0] == 1


@pytest.mark.asyncio
async def test_make_api_request_retries_on_429_then_succeeds():
    req = httpx.Request("GET", "https://graph.facebook.com/v25.0/act_1/campaigns")
    resp_429 = httpx.Response(429, request=req, json={"error": {"code": 4, "message": "rate limit"}})
    resp_ok = httpx.Response(200, request=req, json={"data": [{"id": "c1"}]})
    calls = {"n": 0}

    class _FakeClient:
        def __init__(self, *a, **k):
            pass

        async def __aenter__(self):
            return self

        async def __aexit__(self, *a):
            return False

        async def get(self, *a, **k):
            calls["n"] += 1
            if calls["n"] == 1:
                resp_429.raise_for_status()  # -> httpx.HTTPStatusError (retryable)
            return resp_ok

    with patch.object(graph_client.httpx, "AsyncClient", _FakeClient), \
         patch.object(graph_client.asyncio, "sleep", new_callable=AsyncMock):
        out = await graph_client.make_api_request("act_1/campaigns", "token", {}, method="GET")

    assert calls["n"] == 2          # retried once
    assert out == {"data": [{"id": "c1"}]}


# --- create_report follows insights pagination (create_report undercount) ---


@pytest.mark.asyncio
async def test_create_report_follows_all_insight_pages():
    pages = [
        {"data": [{"spend": "1"}], "paging": {"next": "u", "cursors": {"after": "C1"}}},
        {"data": [{"spend": "2"}], "paging": {}},
    ]
    calls = {"n": 0}

    async def _fake(endpoint, token, params, *a, **k):
        i = calls["n"]
        calls["n"] += 1
        return pages[i]

    with patch.object(report_tools, "make_api_request", side_effect=_fake):
        rows = await report_tools._fetch_insights_rows("act_1", "token", {}, {}, "account")

    assert calls["n"] == 2
    assert [r["spend"] for r in rows] == ["1", "2"]


# --- account_controls: object params, not flattened (live+SDK confirmed) ---


@pytest.mark.asyncio
async def test_update_account_controls_sends_object_params_not_flattened():
    from armavita_meta_ads_mcp.core import account_controls_tools

    with patch.object(account_controls_tools, "make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"success": True}
        await account_controls_tools.update_account_controls(
            ad_account_id="act_1",
            audience_controls={"age_min": 21},
            placement_controls={"facebook_positions": ["feed"]},
            meta_access_token="token",
        )
    payload = mock_api.call_args.args[2]
    # Meta accepts audience_controls / placement_controls as whole objects — NOT the
    # flattened inner keys (e.g. age_min must stay nested under audience_controls).
    assert set(payload.keys()) == {"audience_controls", "placement_controls"}
    assert payload["audience_controls"] == {"age_min": 21}


@pytest.mark.asyncio
async def test_update_account_controls_requires_at_least_one():
    from armavita_meta_ads_mcp.core import account_controls_tools

    raw = await account_controls_tools.update_account_controls(ad_account_id="act_1", meta_access_token="token")
    assert "audience_controls" in raw and "placement_controls" in raw


# --- Partnership: branded content edge lives on the IG-User node (partnership-1/2) ---


@pytest.mark.asyncio
async def test_branded_content_permissions_query_ig_user_node():
    from armavita_meta_ads_mcp.core import partnership_tools

    with patch.object(partnership_tools, "make_api_request", new_callable=AsyncMock) as m:
        m.return_value = {"data": []}
        await partnership_tools.list_branded_content_ad_permissions(
            instagram_user_id="17841400000", meta_access_token="token"
        )
    assert m.call_args.args[0] == "17841400000/branded_content_ad_permissions"


@pytest.mark.asyncio
async def test_grant_branded_content_posts_to_ig_user_node():
    from armavita_meta_ads_mcp.core import partnership_tools

    with patch.object(partnership_tools, "make_api_request", new_callable=AsyncMock) as m:
        m.return_value = {"success": True}
        await partnership_tools.grant_branded_content_ad_permission(
            instagram_user_id="17841400000",
            permission_data={"creator_username": "creator"},
            meta_access_token="token",
        )
    assert m.call_args.args[0] == "17841400000/branded_content_ad_permissions"
    assert m.call_args.kwargs.get("method") == "POST"


# --- OAuth: CSRF state nonce issued + recorded (auth-security-1) ---


def test_login_state_issued_and_get_auth_url_side_effect_free():
    from armavita_meta_ads_mcp.core import auth_state
    from armavita_meta_ads_mcp.core.oauth_callback_server import token_container

    # issue_login_state records the nonce; get_auth_url(state=...) embeds it.
    state = auth_state.auth_manager.issue_login_state()
    assert token_container.get("expected_state") == state
    url = auth_state.auth_manager.get_auth_url(state=state)
    assert "state=" in url and state in url  # token_urlsafe is URL-safe, appears verbatim

    # get_auth_url() WITHOUT a state must NOT mutate expected_state — the error-display
    # path must never clobber an in-flight login nonce (regression guard).
    auth_state.auth_manager.get_auth_url()
    assert token_container.get("expected_state") == state

    # The nonce rotates only when explicitly re-issued.
    state2 = auth_state.auth_manager.issue_login_state()
    assert state2 != state and token_container.get("expected_state") == state2


def test_csrf_state_validation_is_fail_closed():
    from armavita_meta_ads_mcp.core.oauth_callback_server import _state_is_valid

    assert _state_is_valid("abc", "abc") is True
    assert _state_is_valid("abc", "xyz") is False
    assert _state_is_valid("abc", None) is False  # no nonce issued -> reject (fail closed)
    assert _state_is_valid(None, "abc") is False
    assert _state_is_valid("", "") is False


# ====== Final-verification round (regressions/incomplete-fixes caught by the audit) ======


@pytest.mark.asyncio
async def test_update_campaign_drops_cbo_bidcap_and_warns():
    from armavita_meta_ads_mcp.core.campaign_tools import update_campaign

    with patch("armavita_meta_ads_mcp.core.campaign_tools.make_api_request", new_callable=AsyncMock) as m:
        m.return_value = {"id": "c1"}
        raw = await update_campaign(
            campaign_id="c1", name="X", campaign_budget_optimization=True, bid_cap=500, meta_access_token="t"
        )
    payload = m.call_args.args[2]
    assert "campaign_budget_optimization" not in payload and "bid_cap" not in payload
    codes = {w["code"] for w in json.loads(raw).get("warnings", [])}
    assert {"campaign_budget_optimization_not_a_field", "bid_cap_not_a_campaign_field"} <= codes


@pytest.mark.asyncio
async def test_update_campaign_surfaces_warning_when_only_nonwritable_passed():
    from armavita_meta_ads_mcp.core.campaign_tools import update_campaign

    with patch("armavita_meta_ads_mcp.core.campaign_tools.make_api_request", new_callable=AsyncMock) as m:
        raw = await update_campaign(campaign_id="c1", campaign_budget_optimization=False, meta_access_token="t")
    m.assert_not_called()  # nothing writable -> no API call
    out = json.loads(raw)
    assert out.get("warnings") and any(w["code"] == "campaign_budget_optimization_not_a_field" for w in out["warnings"])


@pytest.mark.asyncio
async def test_update_ad_set_is_dynamic_creative_only_surfaces_warning():
    with patch("armavita_meta_ads_mcp.core.adset_tools.make_api_request", new_callable=AsyncMock) as m:
        raw = await update_ad_set(ad_set_id="as1", is_dynamic_creative=True, meta_access_token="t")
    m.assert_not_called()
    out = json.loads(raw)
    assert out.get("warnings") and "is_dynamic_creative" in out["warnings"][0]


@pytest.mark.asyncio
async def test_create_insights_job_rejects_invalid_level():
    from armavita_meta_ads_mcp.core.insights_async_tools import create_insights_job

    with patch("armavita_meta_ads_mcp.core.insights_async_tools.make_api_request", new_callable=AsyncMock) as m:
        raw = await create_insights_job(object_id="act_1", level="bogus", meta_access_token="t")
    assert "invalid_level" in raw
    m.assert_not_called()


@pytest.mark.asyncio
async def test_custom_conversion_default_value_forwarded():
    from armavita_meta_ads_mcp.core import conversion_tools

    with patch.object(conversion_tools, "make_api_request", new_callable=AsyncMock) as m:
        m.return_value = {"id": "cc1"}
        await conversion_tools.create_custom_conversion(
            ad_account_id="act_1", name="N", event_source_id="123", rule={"a": 1},
            custom_event_type="PURCHASE", default_conversion_value=4.5, meta_access_token="t",
        )
    assert m.call_args.args[2]["default_conversion_value"] == 4.5

    with patch.object(conversion_tools, "make_api_request", new_callable=AsyncMock) as m2:
        m2.return_value = {"id": "cc1"}
        await conversion_tools.update_custom_conversion(
            custom_conversion_id="cc1", default_conversion_value=9.0, meta_access_token="t"
        )
    assert m2.call_args.args[2]["default_conversion_value"] == 9.0


@pytest.mark.asyncio
async def test_apply_recommendation_needs_no_token():
    from armavita_meta_ads_mcp.core.recommendation_tools import apply_recommendation

    # No @meta_api_tool: works without a token and makes no API call.
    raw = await apply_recommendation(object_id="c1", recommendation_data={"blame_field": "daily_budget"})
    out = json.loads(raw)
    assert out["status"] == "not_applied" and out["blame_field"] == "daily_budget"


def test_tool_annotations_contract():
    import asyncio
    from armavita_meta_ads_mcp.core.mcp_runtime import mcp_server, _import_tool_modules

    _import_tool_modules()
    tools = {t.name: t for t in asyncio.run(mcp_server.list_tools())}
    for n in ("delete_custom_audience", "delete_custom_conversion", "manage_custom_audience_users"):
        assert tools[n].annotations.destructiveHint is True
    for n in ("list_campaigns", "read_ad", "search_interests", "get_account_controls"):
        assert tools[n].annotations.readOnlyHint is True
    for n in ("create_campaign", "update_ad_set", "upload_ad_image_asset"):
        assert tools[n].annotations.readOnlyHint is False
    # apply_recommendation is now inert (no API call) -> read-only.
    assert tools["apply_recommendation"].annotations.readOnlyHint is True


def test_pii_field_specific_normalization():
    # CAPI
    assert capi_tools._normalize_pii_text("ct", "San Francisco") == "sanfrancisco"
    assert capi_tools._normalize_pii_text("st", "New York!") == "newyork"
    assert capi_tools._normalize_pii_text("zp", " 94105-1234 ") == "941051234"
    assert capi_tools._normalize_pii_text("db", "1990-05-21") == "19900521"
    assert capi_tools._normalize_pii_text("ge", "Female") == "f"
    assert capi_tools._normalize_user_data({"fi": "J"})["fi"] == _sha256("j")
    # Customer-list
    assert audience_tools._normalize_audience_text("CT", "San Francisco") == "sanfrancisco"
    assert audience_tools._normalize_audience_text("ZIP", "94105 1234") == "941051234"
    assert audience_tools._normalize_audience_text("DOBY", "1990") == "1990"
    assert audience_tools._normalize_audience_text("GEN", "male") == "m"
