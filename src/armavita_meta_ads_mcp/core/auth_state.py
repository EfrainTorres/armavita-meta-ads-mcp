# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""Authentication state and OAuth helpers for Meta Ads MCP."""


import json
import os
import platform
import pathlib
import time
import webbrowser
from dataclasses import dataclass
from typing import Any, Dict, Optional
from urllib.parse import urlencode

import requests

from .oauth_callback_server import (
    reset_token_container,
    shutdown_callback_server,
    start_callback_server,
    token_container,
)
from .graph_constants import META_OAUTH_DIALOG_BASE, META_OAUTH_TOKEN_BASE
from .media_helpers import logger


AUTH_SCOPE = os.environ.get(
    "META_AUTH_SCOPE",
    "business_management,public_profile,pages_show_list,pages_read_engagement",
)
AUTH_CONFIG_ID = os.environ.get("META_LOGIN_CONFIG_ID", "").strip()
AUTH_REDIRECT_URI = "http://localhost:8888/callback"
AUTH_RESPONSE_TYPE = "code"

needs_authentication = False


class MetaConfig:
    """In-process configuration holder for Meta app credentials."""

    _instance = None

    def __new__(cls):
        if cls._instance is None:
            obj = super().__new__(cls)
            obj.app_id = os.environ.get("META_APP_ID", "YOUR_META_APP_ID")
            cls._instance = obj
        return cls._instance

    def set_app_id(self, app_id: str) -> None:
        self.app_id = app_id
        os.environ["META_APP_ID"] = app_id

    def get_app_id(self) -> str:
        if getattr(self, "app_id", ""):
            return self.app_id
        env_app_id = os.environ.get("META_APP_ID", "")
        if env_app_id:
            self.app_id = env_app_id
        return env_app_id

    def is_configured(self) -> bool:
        return bool(self.get_app_id())


meta_config = MetaConfig()


@dataclass
class TokenInfo:
    meta_access_token: str
    expires_in: Optional[int] = None
    meta_user_id: Optional[str] = None
    created_at: int = 0

    def __post_init__(self) -> None:
        if not self.created_at:
            self.created_at = int(time.time())

    def is_expired(self) -> bool:
        if not self.expires_in:
            return False
        return int(time.time()) > (self.created_at + self.expires_in)

    def serialize(self) -> Dict[str, Any]:
        return {
            "meta_access_token": self.meta_access_token,
            "expires_in": self.expires_in,
            "meta_user_id": self.meta_user_id,
            "created_at": self.created_at,
        }

    @classmethod
    def deserialize(cls, data: Dict[str, Any]) -> "TokenInfo":
        return cls(
            meta_access_token=data.get("meta_access_token") or data.get("access_token", ""),
            expires_in=_coerce_expires_in(data.get("expires_in")),
            meta_user_id=data.get("meta_user_id") or data.get("user_id"),
            created_at=int(data.get("created_at", int(time.time()))),
        )



def _coerce_expires_in(value: Any) -> Optional[int]:
    if value is None:
        return None
    try:
        parsed = int(value)
        return parsed if parsed > 0 else None
    except (TypeError, ValueError):
        return None


class AuthManager:
    """Manages local token cache and OAuth browser flow."""

    def __init__(self, app_id: str, redirect_uri: str = AUTH_REDIRECT_URI):
        self.app_id = app_id
        self.redirect_uri = redirect_uri
        self.token_info: Optional[TokenInfo] = None
        self._load_cached_token()

    def _cache_path(self) -> pathlib.Path:
        if platform.system() == "Windows":
            base = pathlib.Path(os.environ.get("APPDATA", ""))
        elif platform.system() == "Darwin":
            base = pathlib.Path.home() / "Library" / "Application Support"
        else:
            base = pathlib.Path.home() / ".config"
        cache_dir = base / "armavita-meta-ads-mcp"
        cache_dir.mkdir(parents=True, exist_ok=True)
        return cache_dir / "token_cache.json"

    def _load_cached_token(self) -> bool:
        path = self._cache_path()
        if not path.exists():
            return False
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
            token = TokenInfo.deserialize(payload)
            if not token.meta_access_token or len(token.meta_access_token) < 20 or token.is_expired():
                path.unlink(missing_ok=True)
                return False
            self.token_info = token
            return True
        except Exception:  # noqa: BLE001
            path.unlink(missing_ok=True)
            return False

    def _persist_token(self) -> None:
        if not self.token_info:
            return
        path = self._cache_path()
        path.write_text(json.dumps(self.token_info.serialize()), encoding="utf-8")

    def get_auth_url(self) -> str:
        params = {
            "client_id": self.app_id,
            "redirect_uri": self.redirect_uri,
            "response_type": AUTH_RESPONSE_TYPE,
        }
        if AUTH_CONFIG_ID:
            params["config_id"] = AUTH_CONFIG_ID
        else:
            params["scope"] = AUTH_SCOPE
        return f"{META_OAUTH_DIALOG_BASE}?{urlencode(params)}"

    def authenticate(self, force_refresh: bool = False) -> Optional[str]:
        if not force_refresh and self.token_info and not self.token_info.is_expired():
            return self.token_info.meta_access_token
        port = start_callback_server()
        self.redirect_uri = f"http://localhost:{port}/callback"
        url = self.get_auth_url()
        logger.info("Opening OAuth login URL: %s", url)
        webbrowser.open(url)
        return None

    def get_access_token(self) -> Optional[str]:
        if not self.token_info or self.token_info.is_expired():
            return None
        return self.token_info.meta_access_token

    def invalidate_token(self) -> None:
        global needs_authentication
        self.token_info = None
        needs_authentication = True
        self._cache_path().unlink(missing_ok=True)

    def clear_token(self) -> None:
        self.invalidate_token()



def _store_token(token_info: TokenInfo, persist_token: bool = True) -> None:
    global needs_authentication
    auth_manager.token_info = token_info
    if persist_token:
        auth_manager._persist_token()
    needs_authentication = False



def exchange_code_for_short_lived(auth_code: str, redirect_uri: Optional[str] = None) -> Optional[TokenInfo]:
    app_id = meta_config.get_app_id()
    app_secret = os.environ.get("META_APP_SECRET", "")
    if not app_id or not app_secret:
        return None

    params = {
        "client_id": app_id,
        "redirect_uri": redirect_uri or AUTH_REDIRECT_URI,
        "client_secret": app_secret,
        "code": auth_code,
    }
    try:
        response = requests.get(META_OAUTH_TOKEN_BASE, params=params, timeout=20)
        if response.status_code != 200:
            logger.error("Authorization code exchange failed: %s", response.text)
            return None
        payload = response.json()
        token = payload.get("access_token") or payload.get("meta_access_token")
        if not token:
            return None
        return TokenInfo(
            meta_access_token=token,
            expires_in=_coerce_expires_in(payload.get("expires_in")),
            meta_user_id=payload.get("user_id") or payload.get("meta_user_id"),
        )
    except Exception as exc:  # noqa: BLE001
        logger.error("Authorization code exchange error: %s", exc)
        return None



def exchange_token_for_long_lived(short_lived_token: str) -> Optional[TokenInfo]:
    app_id = meta_config.get_app_id()
    app_secret = os.environ.get("META_APP_SECRET", "")
    if not app_id or not app_secret:
        return None

    params = {
        "grant_type": "fb_exchange_token",
        "client_id": app_id,
        "client_secret": app_secret,
        "fb_exchange_token": short_lived_token,
    }
    try:
        response = requests.get(META_OAUTH_TOKEN_BASE, params=params, timeout=20)
        if response.status_code != 200:
            logger.warning("Long-lived token exchange failed: %s", response.text)
            return None
        payload = response.json()
        new_token = payload.get("access_token") or payload.get("meta_access_token")
        if not new_token:
            return None
        return TokenInfo(
            meta_access_token=new_token,
            expires_in=_coerce_expires_in(payload.get("expires_in")),
        )
    except Exception as exc:  # noqa: BLE001
        logger.warning("Long-lived token exchange error: %s", exc)
        return None



def complete_oauth_from_auth_code(
    auth_code: str,
    redirect_uri: Optional[str] = None,
    persist_token: bool = True,
) -> Dict[str, Any]:
    short_token_info = exchange_code_for_short_lived(auth_code, redirect_uri=redirect_uri)
    if not short_token_info:
        return {
            "success": False,
            "error": "authorization_code_exchange_failed",
            "token_info": None,
            "used_long_lived_token": False,
        }

    long_token_info = exchange_token_for_long_lived(short_token_info.meta_access_token)
    final_token = long_token_info or short_token_info
    _store_token(final_token, persist_token=persist_token)

    return {
        "success": True,
        "error": None,
        "token_info": final_token,
        "used_long_lived_token": bool(long_token_info),
    }


async def get_current_access_token() -> Optional[str]:
    env_token = os.environ.get("META_ACCESS_TOKEN")
    if env_token:
        if len(env_token) < 20:
            logger.error("META_ACCESS_TOKEN appears malformed")
            return None
        return env_token

    app_id = meta_config.get_app_id()
    if not app_id or app_id == "YOUR_META_APP_ID":
        logger.error("No valid META_APP_ID configured")
        return None

    token = auth_manager.get_access_token()
    if token and len(token) >= 20:
        return token

    if token and len(token) < 20:
        auth_manager.invalidate_token()
    return None



def login() -> None:
    print("Starting Meta Ads authentication flow...")
    try:
        port = start_callback_server()
        auth_manager.redirect_uri = f"http://localhost:{port}/callback"
        reset_token_container()
        token_container["redirect_uri"] = auth_manager.redirect_uri

        auth_url = auth_manager.get_auth_url()
        print(f"Opening browser: {auth_url}")
        webbrowser.open(auth_url)

        deadline = time.time() + 300
        while time.time() < deadline:
            if token_container.get("token"):
                print("Authentication successful.")
                return
            if token_container.get("error"):
                print(f"Authentication failed: {token_container.get('error')}")
                return
            time.sleep(2)
        print("Authentication timed out.")
    except Exception as exc:  # noqa: BLE001
        print(f"Authentication error: {exc}")
    finally:
        shutdown_callback_server()


META_APP_ID = os.environ.get("META_APP_ID", "YOUR_META_APP_ID")
auth_manager = AuthManager(META_APP_ID)