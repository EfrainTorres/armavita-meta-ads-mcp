"""Server bootstrap entrypoints for armavita-meta-ads-mcp."""

from armavita_meta_ads_mcp.core.mcp_runtime import main as _main


def run() -> int:
    """Run the armavita-meta-ads-mcp process."""
    return _main()
