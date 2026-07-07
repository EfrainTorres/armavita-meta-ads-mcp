# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

import json
import sys
from pathlib import Path
from unittest.mock import AsyncMock, patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from armavita_meta_ads_mcp.core.insights_async_tools import create_insights_job, read_insights_job


def _inner(raw: str) -> dict:
    payload = json.loads(raw)
    if isinstance(payload.get("data"), str):
        return json.loads(payload["data"])
    return payload


@pytest.mark.asyncio
async def test_create_insights_job_forwards_export_format_and_default_fields():
    with patch("armavita_meta_ads_mcp.core.insights_async_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"report_run_id": "123"}
        await create_insights_job(
            object_id="act_1",
            meta_access_token="token",
            breakdowns=["mmm"],
            export_format="csv",
        )

    params = mock_api.call_args.args[2]
    assert params["export_format"] == "csv"
    assert params["breakdowns"] == "mmm"
    assert "impressions" in params["fields"]


@pytest.mark.asyncio
async def test_create_insights_job_rejects_invalid_export_format():
    raw = await create_insights_job(
        object_id="act_1",
        meta_access_token="token",
        export_format="pdf",
    )
    payload = _inner(raw)
    assert payload["error"] == "invalid_export_format"


@pytest.mark.asyncio
async def test_create_insights_job_surfaces_date_preset_warning():
    with patch("armavita_meta_ads_mcp.core.insights_async_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {"report_run_id": "123"}
        raw = await create_insights_job(
            object_id="act_1",
            meta_access_token="token",
            date_range="previous_30d",
        )

    payload = json.loads(raw)
    warnings = payload.get("warnings", [])
    assert any(w.get("code") == "date_preset_alias_applied" for w in warnings)


@pytest.mark.asyncio
async def test_read_insights_job_omits_error_details_on_completed_jobs():
    with patch("armavita_meta_ads_mcp.core.insights_async_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {
            "async_status": "Job Completed",
            "async_report_url": "https://example.com/report.csv",
            "error_code": 10,
            "error_message": "failed",
        }
        raw = await read_insights_job(report_run_id="job_1", meta_access_token="token")

    params = mock_api.call_args.args[2]
    assert "async_report_url" in params["fields"]
    payload = json.loads(raw)
    assert "_error_details" not in payload


@pytest.mark.asyncio
async def test_read_insights_job_includes_error_details_on_failed_jobs():
    with patch("armavita_meta_ads_mcp.core.insights_async_tools.make_api_request", new_callable=AsyncMock) as mock_api:
        mock_api.return_value = {
            "async_status": "Job Failed",
            "error_code": 10,
            "error_message": "failed",
        }
        raw = await read_insights_job(report_run_id="job_1", meta_access_token="token")

    payload = json.loads(raw)
    assert payload.get("_error_details")
