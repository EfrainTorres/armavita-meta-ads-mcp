import sys
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

import armavita_meta_ads_mcp
from armavita_meta_ads_mcp.core import mcp_runtime


def test_package_exports():
    assert armavita_meta_ads_mcp.__version__ == "1.0.0"
    assert callable(armavita_meta_ads_mcp.entrypoint)


def test_runtime_rejects_non_stdio(capsys):
    with patch.object(sys, "argv", ["armavita-meta-ads-mcp", "--transport", "http"]):
        exit_code = mcp_runtime.main()

    captured = capsys.readouterr()
    assert exit_code == 2
    assert "stdio MCP only" in captured.out


def test_runtime_version_flag(capsys):
    with patch.object(sys, "argv", ["armavita-meta-ads-mcp", "--version"]):
        exit_code = mcp_runtime.main()

    captured = capsys.readouterr()
    assert exit_code == 0
    assert "Armavita Meta Ads MCP v1.0.0" in captured.out
