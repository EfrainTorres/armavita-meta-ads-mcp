# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from armavita_meta_ads_mcp.core import auth_state


def test_complete_oauth_uses_cache_save_hook(monkeypatch):
    short = auth_state.TokenInfo(meta_access_token="short_token_12345678901234567890", expires_in=3600)
    long = auth_state.TokenInfo(meta_access_token="long_token_12345678901234567890", expires_in=5184000)
    saved = []

    monkeypatch.setattr(auth_state, "exchange_code_for_short_lived", lambda *_args, **_kwargs: short)
    monkeypatch.setattr(auth_state, "exchange_token_for_long_lived", lambda *_args, **_kwargs: long)
    monkeypatch.setattr(auth_state.auth_manager, "_persist_token", lambda: saved.append(True))

    result = auth_state.complete_oauth_from_auth_code("auth_code_1", persist_token=True)

    assert result["success"] is True
    assert result["used_long_lived_token"] is True
    assert saved == [True]