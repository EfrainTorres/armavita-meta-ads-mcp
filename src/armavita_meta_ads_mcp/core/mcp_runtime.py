# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""FastMCP runtime assembly and stdio-first CLI entrypoint."""

import argparse
import logging
import os
from typing import Optional

from mcp.server.fastmcp import FastMCP

from .auth_state import login as login_auth
from .media_helpers import logger

mcp_server = FastMCP("meta-ads")

# Prevent upstream HTTP libraries from logging full request URLs that can
# include access_token query parameters.
logging.getLogger("httpx").setLevel(logging.WARNING)
logging.getLogger("httpcore").setLevel(logging.WARNING)


def login_cli() -> None:
    """Run local OAuth login flow from CLI."""
    logger.info("cli_login_flow_start")
    print("Starting Armavita CLI authentication flow...")
    login_auth()


def _configure_meta_app_id(cli_app_id: Optional[str]) -> None:
    from .auth_state import auth_manager, meta_config

    chosen = cli_app_id or os.environ.get("META_APP_ID")
    if not chosen:
        logger.warning("META_APP_ID not configured")
        return

    auth_manager.app_id = chosen
    meta_config.set_app_id(chosen)
    logger.info("meta_app_id_configured=%s", chosen)


def _import_tool_modules() -> None:
    from . import (  # noqa: F401
        account_tools,
        ad_tools,
        adset_tools,
        ads_archive_tools,
        budget_schedule_tools,
        campaign_tools,
        duplication_tools,
        insight_tools,
        report_tools,
        research_tools,
        targeting_tools,
    )


def _run_stdio() -> int:
    _import_tool_modules()
    mcp_server.run(transport="stdio")
    return 0


def main() -> int:
    """CLI entrypoint used by `python -m armavita_meta_ads_mcp` and scripts."""
    parser = argparse.ArgumentParser(
        description="Armavita Meta Ads MCP (local stdio mode).",
        epilog="Use with local MCP clients such as Claude Code, Cursor, and Codex.",
    )
    parser.add_argument("--login", action="store_true", help="Run login flow and cache token")
    parser.add_argument("--app-id", type=str, help="Override META_APP_ID")
    parser.add_argument("--version", action="store_true", help="Print package version")

    # Explicitly reject non-stdio transports in this OSS-local build.
    parser.add_argument("--transport", type=str, default="stdio", help=argparse.SUPPRESS)

    args = parser.parse_args()

    if args.version:
        from armavita_meta_ads_mcp import __version__

        print(f"Armavita Meta Ads MCP v{__version__}")
        return 0

    _configure_meta_app_id(args.app_id)

    if args.login:
        login_cli()
        return 0

    if args.transport != "stdio":
        print("This OSS build supports local stdio MCP only.")
        return 2

    return _run_stdio()


if __name__ == "__main__":
    raise SystemExit(main())