from pathlib import Path
import sys
from unittest.mock import patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from armavita_meta_ads_mcp.core.graph_client import _sanitize_response_payload, _sanitize_url, make_api_request


def test_sanitize_url_removes_access_token_and_keeps_other_params():
    raw = (
        "https://graph.facebook.com/v25.0/act_123/campaigns"
        "?fields=id%2Cname&access_token=secret-token&limit=10"
    )
    sanitized = _sanitize_url(raw)

    assert "access_token=" not in sanitized
    assert "fields=id%2Cname" in sanitized
    assert "limit=10" in sanitized


def test_sanitize_url_no_query_stays_stable():
    raw = "https://graph.facebook.com/v25.0/act_123/campaigns"
    assert _sanitize_url(raw) == raw


def test_sanitize_response_payload_redacts_nested_paging_urls():
    payload = {
        "data": [{"id": "1"}],
        "paging": {
            "next": (
                "https://graph.facebook.com/v25.0/act_123/campaigns"
                "?fields=id%2Cname&access_token=secret-token&after=abc"
            ),
            "previous": (
                "https://graph.facebook.com/v25.0/act_123/campaigns"
                "?fields=id%2Cname&access_token=secret-token&before=xyz"
            ),
        },
        "nested": [{"url": "https://example.com/path?foo=bar&access_token=secret"}],
    }
    sanitized = _sanitize_response_payload(payload)

    assert "access_token=" not in sanitized["paging"]["next"]
    assert "access_token=" not in sanitized["paging"]["previous"]
    assert "access_token=" not in sanitized["nested"][0]["url"]
    assert "after=abc" in sanitized["paging"]["next"]
    assert "before=xyz" in sanitized["paging"]["previous"]
    assert "foo=bar" in sanitized["nested"][0]["url"]


class _FakeResponse:
    status_code = 200
    headers = {}
    text = "{}"

    def raise_for_status(self) -> None:
        return None

    def json(self):
        return {
            "data": [{"id": "1"}],
            "paging": {
                "next": (
                    "https://graph.facebook.com/v25.0/act_123/campaigns"
                    "?limit=10&access_token=secret-token&after=abc"
                )
            },
        }


class _FakeClient:
    async def __aenter__(self):
        return self

    async def __aexit__(self, exc_type, exc, tb):
        return None

    async def get(self, url, params, headers):
        return _FakeResponse()


@pytest.mark.asyncio
async def test_make_api_request_sanitizes_success_payload_urls():
    with patch("armavita_meta_ads_mcp.core.graph_client.httpx.AsyncClient", return_value=_FakeClient()):
        payload = await make_api_request("act_123/campaigns", "token", {"page_size": 10}, method="GET")

    assert "paging" in payload
    assert "next" in payload["paging"]
    assert "access_token=" not in payload["paging"]["next"]
    assert "after=abc" in payload["paging"]["next"]
