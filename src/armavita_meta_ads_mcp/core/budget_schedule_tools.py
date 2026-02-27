"""Campaign budget schedule tooling."""


import json
from typing import Optional

from .graph_client import make_api_request, meta_api_tool
from .mcp_runtime import mcp_server

_ALLOWED_BUDGET_VALUE_TYPES = {"ABSOLUTE", "MULTIPLIER"}


@mcp_server.tool()
@meta_api_tool
async def create_campaign_budget_schedule(
    campaign_id: str,
    budget_value: int,
    budget_value_type: str,
    time_start: int,
    time_end: int,
    meta_access_token: Optional[str] = None,
) -> str:
    """Create a high-demand budget schedule for a campaign."""
    if not campaign_id:
        return json.dumps({"error": "Campaign ID is required"}, indent=2)
    if budget_value is None:
        return json.dumps({"error": "Budget value is required"}, indent=2)
    if time_start is None or time_end is None:
        return json.dumps({"error": "time_start and time_end are required"}, indent=2)

    normalized_type = str(budget_value_type or "").upper().strip()
    if normalized_type not in _ALLOWED_BUDGET_VALUE_TYPES:
        return json.dumps(
            {
                "error": "Invalid budget_value_type",
                "details": "budget_value_type must be ABSOLUTE or MULTIPLIER",
            },
            indent=2,
        )

    payload = {
        "budget_value": budget_value,
        "budget_value_type": normalized_type,
        "time_start": time_start,
        "time_end": time_end,
    }

    result = await make_api_request(f"{campaign_id}/budget_schedules", meta_access_token, payload, method="POST")
    return json.dumps(result, indent=2)
