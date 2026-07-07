# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""Single source of truth for the package version.

Kept as a dependency-free leaf module so it can be imported from anywhere
(package __init__, media_helpers, graph_client) without import cycles. Keep
this in sync with the `version` field in pyproject.toml on each release.
"""

__version__ = "1.2.0"
