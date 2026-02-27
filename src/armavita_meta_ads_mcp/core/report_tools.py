"""OSS report generation for Meta Ads API."""

import base64
import datetime
import json
from typing import Any, Dict, List, Optional, Union

from .graph_client import make_api_request, meta_api_tool
from .insight_query_params import normalize_breakdown_inputs, normalize_time_input
from .mcp_runtime import mcp_server

_DEFAULT_INSIGHTS_FIELDS = (
    "campaign_id,campaign_name,impressions,clicks,spend,cpc,cpm,ctr,reach,frequency,"
    "actions,action_values,cost_per_action_type"
)


def _to_float(value: Any) -> float:
    try:
        return float(value)
    except (TypeError, ValueError):
        return 0.0


def _to_int(value: Any) -> int:
    try:
        return int(float(value))
    except (TypeError, ValueError):
        return 0


def _resolve_time_params(date_range: Any):
    return normalize_time_input(date_range, default_preset="last_30d")


def _get_previous_period_from_range(date_range: Dict[str, str]) -> Optional[Dict[str, str]]:
    try:
        since = datetime.datetime.strptime(date_range["since"], "%Y-%m-%d").date()
        until = datetime.datetime.strptime(date_range["until"], "%Y-%m-%d").date()
    except (KeyError, ValueError, TypeError):
        return None

    days = (until - since).days + 1
    previous_until = since - datetime.timedelta(days=1)
    previous_since = previous_until - datetime.timedelta(days=days - 1)
    return {
        "since": previous_since.strftime("%Y-%m-%d"),
        "until": previous_until.strftime("%Y-%m-%d"),
    }


def _default_comparison_period(date_range: Any) -> Any:
    if isinstance(date_range, dict):
        previous = _get_previous_period_from_range(date_range)
        if previous:
            return previous
    return "last_30d"


def _extract_top_actions(rows: List[Dict[str, Any]], page_size: int = 8) -> List[Dict[str, Any]]:
    totals: Dict[str, float] = {}
    for row in rows:
        for action in row.get("actions", []) if isinstance(row.get("actions"), list) else []:
            action_type = action.get("action_type")
            if not action_type:
                continue
            totals[action_type] = totals.get(action_type, 0.0) + _to_float(action.get("value"))

    ranked = sorted(totals.items(), key=lambda pair: pair[1], reverse=True)
    return [{"action_type": action_type, "value": value} for action_type, value in ranked[:page_size]]


def _aggregate_rows(rows: List[Dict[str, Any]]) -> Dict[str, Any]:
    spend = sum(_to_float(row.get("spend")) for row in rows)
    impressions = sum(_to_int(row.get("impressions")) for row in rows)
    clicks = sum(_to_int(row.get("clicks")) for row in rows)
    reach = sum(_to_int(row.get("reach")) for row in rows)

    ctr = (clicks / impressions * 100.0) if impressions else 0.0
    cpc = (spend / clicks) if clicks else 0.0
    cpm = (spend / impressions * 1000.0) if impressions else 0.0
    frequency = (impressions / reach) if reach else 0.0

    return {
        "spend": round(spend, 4),
        "impressions": impressions,
        "clicks": clicks,
        "ctr": round(ctr, 4),
        "cpc": round(cpc, 4),
        "cpm": round(cpm, 4),
        "reach": reach,
        "frequency": round(frequency, 4),
        "top_actions": _extract_top_actions(rows),
    }


def _build_html_report(report: Dict[str, Any]) -> str:
    summary = report.get("summary", {})
    sections = report.get("sections", {})
    title = report.get("report_name", "Meta Ads Report")

    kpis = [
        ("Spend", summary.get("spend")),
        ("Impressions", summary.get("impressions")),
        ("Clicks", summary.get("clicks")),
        ("CTR", f"{summary.get('ctr', 0)}%"),
        ("CPC", summary.get("cpc")),
        ("CPM", summary.get("cpm")),
        ("Reach", summary.get("reach")),
        ("Frequency", summary.get("frequency")),
    ]

    kpi_rows = "".join(
        f"<tr><th style='text-align:left;padding:6px;border-bottom:1px solid #ddd'>{label}</th>"
        f"<td style='padding:6px;border-bottom:1px solid #ddd'>{value}</td></tr>"
        for label, value in kpis
    )

    action_rows = "".join(
        f"<tr><td style='padding:6px;border-bottom:1px solid #eee'>{a.get('action_type')}</td>"
        f"<td style='padding:6px;border-bottom:1px solid #eee'>{a.get('value')}</td></tr>"
        for a in summary.get("top_actions", [])
    )

    comparison_html = ""
    if "comparison" in sections:
        comp = sections["comparison"]
        comparison_html = (
            "<h2>Comparison</h2>"
            f"<p>Current period spend: {comp.get('current_summary', {}).get('spend')}</p>"
            f"<p>Comparison period spend: {comp.get('comparison_summary', {}).get('spend')}</p>"
            f"<p>Spend delta: {comp.get('delta', {}).get('spend')}</p>"
        )

    return (
        "<html><head><meta charset='utf-8'><title>Meta Ads Report</title></head>"
        "<body style='font-family:Arial,sans-serif;max-width:960px;margin:24px auto;padding:0 16px'>"
        f"<h1>{title}</h1>"
        f"<p><strong>Generated:</strong> {report.get('generated_at')}</p>"
        "<h2>Summary KPIs</h2>"
        f"<table style='border-collapse:collapse;width:100%'>{kpi_rows}</table>"
        "<h2>Top Actions</h2>"
        f"<table style='border-collapse:collapse;width:100%'><tr><th style='text-align:left;padding:6px'>Action</th>"
        f"<th style='text-align:left;padding:6px'>Value</th></tr>{action_rows}</table>"
        f"{comparison_html}"
        "</body></html>"
    )


def _build_simple_pdf_bytes(title: str, lines: List[str]) -> bytes:
    """Build a tiny PDF from text lines without external dependencies."""
    safe_lines = [line.replace("(", "[").replace(")", "]") for line in lines]
    text_content = "\\n".join([f"({line}) Tj" for line in safe_lines])

    stream = (
        "BT\n"
        "/F1 12 Tf\n"
        "50 780 Td\n"
        f"({title.replace('(', '[').replace(')', ']')}) Tj\n"
        "0 -20 Td\n"
        f"{text_content}\n"
        "ET"
    )

    objects = []
    objects.append("1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj")
    objects.append("2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj")
    objects.append("3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >> endobj")
    objects.append("4 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj")
    objects.append(f"5 0 obj << /Length {len(stream.encode('utf-8'))} >> stream\n{stream}\nendstream endobj")

    pdf = "%PDF-1.4\n"
    xref_positions = [0]
    for obj in objects:
        xref_positions.append(len(pdf.encode("utf-8")))
        pdf += f"{obj}\n"

    xref_start = len(pdf.encode("utf-8"))
    pdf += f"xref\n0 {len(objects) + 1}\n"
    pdf += "0000000000 65535 f \n"
    for pos in xref_positions[1:]:
        pdf += f"{pos:010d} 00000 n \n"

    pdf += f"trailer << /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref_start}\n%%EOF"
    return pdf.encode("utf-8")


async def _fetch_insights_rows(
    object_id: str,
    meta_access_token: str,
    time_params: Dict[str, Any],
    breakdown_params: Dict[str, str],
    level: str,
) -> List[Dict[str, Any]]:
    params: Dict[str, Any] = {
        "fields": _DEFAULT_INSIGHTS_FIELDS,
        "level": level,
        "page_size": 200,
    }
    params.update(time_params)
    params.update(breakdown_params)

    data = await make_api_request(f"{object_id}/insights", meta_access_token, params)
    if isinstance(data, dict) and isinstance(data.get("data"), list):
        return [row for row in data["data"] if isinstance(row, dict)]
    return []


def _compute_delta(current: Dict[str, Any], prior: Dict[str, Any]) -> Dict[str, float]:
    keys = ["spend", "impressions", "clicks", "ctr", "cpc", "cpm", "reach", "frequency"]
    delta: Dict[str, float] = {}
    for key in keys:
        delta[key] = round(_to_float(current.get(key)) - _to_float(prior.get(key)), 4)
    return delta


@mcp_server.tool()
@meta_api_tool
async def create_report(
    ad_account_id: str,
    meta_access_token: Optional[str] = None,
    report_type: str = "account",
    date_range: Union[str, Dict[str, str]] = "last_30d",
    campaign_ids: Optional[List[str]] = None,
    export_format: str = "pdf",
    report_name: Optional[str] = None,
    include_sections: Optional[List[str]] = None,
    breakdowns: Optional[List[str]] = None,
    action_breakdowns: Optional[List[str]] = None,
    summary_action_breakdowns: Optional[List[str]] = None,
    comparison_period: Optional[Union[str, Dict[str, str]]] = None,
) -> str:
    """Generate OSS-local performance reports for Meta Ads accounts/campaigns."""
    if not ad_account_id:
        return {
            "error": "invalid_parameters",
            "message": "ad_account_id is required",
            "details": {"required_parameter": "ad_account_id", "format": "act_XXXXXXXXX"},
        }

    normalized_type = str(report_type).strip().lower()
    normalized_export = str(export_format).strip().lower()

    if normalized_type not in {"account", "campaign", "comparison"}:
        return {
            "error": "invalid_parameters",
            "message": "report_type must be one of: account, campaign, comparison",
            "report_type": report_type,
        }

    if normalized_export not in {"json", "html", "pdf"}:
        return {
            "error": "invalid_parameters",
            "message": "export_format must be one of: json, html, pdf",
            "export_format": export_format,
        }

    if normalized_type in {"campaign", "comparison"} and not campaign_ids:
        return {
            "error": "invalid_parameters",
            "message": f"campaign_ids are required for {normalized_type} reports",
            "details": {"required_parameter": "campaign_ids"},
        }

    sections_requested = set(include_sections or [])
    if not sections_requested:
        sections_requested = {"summary", "kpis", "actions", "comparison"}

    generated_at = datetime.datetime.utcnow().replace(microsecond=0).isoformat() + "Z"
    report: Dict[str, Any] = {
        "success": True,
        "report_name": report_name or f"Meta Ads {normalized_type.title()} Report",
        "report_type": normalized_type,
        "generated_at": generated_at,
        "date_range": date_range,
        "breakdowns": breakdowns or [],
        "action_breakdowns": action_breakdowns or [],
        "summary_action_breakdowns": summary_action_breakdowns or [],
        "sections": {},
    }

    base_time_params, time_error, time_warnings = _resolve_time_params(date_range)
    if time_error:
        return {
            "error": "invalid_parameters",
            "message": time_error.get("message", "Invalid date range"),
            "details": time_error,
        }
    if not base_time_params:
        return {
            "error": "invalid_parameters",
            "message": "Invalid date range",
            "details": {"error": "invalid_date_range"},
        }

    breakdown_params, breakdown_warnings = normalize_breakdown_inputs(
        breakdowns=breakdowns,
        action_breakdowns=action_breakdowns,
        summary_action_breakdowns=summary_action_breakdowns,
    )

    warnings: List[Dict[str, Any]] = []
    warnings.extend(time_warnings)
    warnings.extend(breakdown_warnings)
    comparison_time_params: Optional[Dict[str, Any]] = None
    comparison_window: Optional[Union[str, Dict[str, str]]] = None
    if normalized_type == "comparison":
        comparison_window = comparison_period if comparison_period is not None else _default_comparison_period(date_range)
        comparison_time_params, comparison_error, comparison_warnings = _resolve_time_params(comparison_window)
        if comparison_error:
            return {
                "error": "invalid_parameters",
                "message": comparison_error.get("message", "Invalid comparison period"),
                "details": {
                    "parameter": "comparison_period",
                    **comparison_error,
                },
            }
        if not comparison_time_params:
            return {
                "error": "invalid_parameters",
                "message": "Invalid comparison period",
                "details": {"parameter": "comparison_period", "error": "invalid_date_range"},
            }
        warnings.extend(comparison_warnings)

    if warnings:
        report["warnings"] = warnings

    rows: List[Dict[str, Any]] = []
    per_campaign_rows: Dict[str, List[Dict[str, Any]]] = {}

    if normalized_type == "account":
        rows = await _fetch_insights_rows(
            ad_account_id,
            meta_access_token,
            base_time_params,
            breakdown_params,
            "account",
        )

    if normalized_type in {"campaign", "comparison"} and campaign_ids:
        for campaign_id in campaign_ids:
            campaign_rows = await _fetch_insights_rows(
                campaign_id,
                meta_access_token,
                base_time_params,
                breakdown_params,
                "campaign",
            )
            per_campaign_rows[campaign_id] = campaign_rows
            rows.extend(campaign_rows)

    summary = _aggregate_rows(rows)
    report["summary"] = summary

    if "kpis" in sections_requested or "summary" in sections_requested:
        report["sections"]["kpis"] = summary

    if "actions" in sections_requested:
        report["sections"]["actions"] = {"top_actions": summary.get("top_actions", [])}

    if normalized_type in {"campaign", "comparison"}:
        campaign_summaries = {
            campaign_id: _aggregate_rows(campaign_rows)
            for campaign_id, campaign_rows in per_campaign_rows.items()
        }
        report["sections"]["campaigns"] = campaign_summaries

    if normalized_type == "comparison":
        comparison_rows: List[Dict[str, Any]] = []
        for campaign_id in campaign_ids or []:
            campaign_rows = await _fetch_insights_rows(
                campaign_id,
                meta_access_token,
                comparison_time_params or {},
                breakdown_params,
                "campaign",
            )
            comparison_rows.extend(campaign_rows)

        comparison_summary = _aggregate_rows(comparison_rows)
        report["sections"]["comparison"] = {
            "comparison_period": comparison_window,
            "current_summary": summary,
            "comparison_summary": comparison_summary,
            "delta": _compute_delta(summary, comparison_summary),
        }

    # Shape export payload
    if normalized_export == "json":
        report["export_format"] = "json"
        return json.dumps(report, indent=2)

    if normalized_export == "html":
        report["export_format"] = "html"
        report["html"] = _build_html_report(report)
        return json.dumps(report, indent=2)

    report["export_format"] = "pdf"
    text_lines = [
        f"Generated: {generated_at}",
        f"Type: {normalized_type}",
        f"Spend: {summary.get('spend')}",
        f"Impressions: {summary.get('impressions')}",
        f"Clicks: {summary.get('clicks')}",
        f"CTR: {summary.get('ctr')}%",
        f"CPC: {summary.get('cpc')}",
        f"CPM: {summary.get('cpm')}",
    ]
    pdf_bytes = _build_simple_pdf_bytes(report["report_name"], text_lines)
    report["pdf_base64"] = base64.b64encode(pdf_bytes).decode("ascii")
    report["pdf_metadata"] = {
        "filename": f"{report['report_name'].replace(' ', '_').lower()}.pdf",
        "mime_type": "application/pdf",
        "size_bytes": len(pdf_bytes),
    }

    return json.dumps(report, indent=2)
