"""armavita-meta-ads-mcp package exports."""

from armavita_meta_ads_mcp.runtime import run as main

__version__ = "1.0.0"

__all__ = ["__version__", "main", "entrypoint"]


def entrypoint() -> int:
    """Main process entrypoint used by script runners."""
    return main()
