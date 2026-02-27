# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""Python module entrypoint for armavita-meta-ads-mcp."""

from armavita_meta_ads_mcp.runtime import run as main

if __name__ == "__main__":
    raise SystemExit(main())