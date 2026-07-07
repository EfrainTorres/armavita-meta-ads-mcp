# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

import sys
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from armavita_meta_ads_mcp.core.graph_client import USER_AGENT as GRAPH_USER_AGENT
from armavita_meta_ads_mcp.core import media_helpers


def test_media_helpers_user_agent_matches_graph_client():
    assert media_helpers.USER_AGENT == GRAPH_USER_AGENT
