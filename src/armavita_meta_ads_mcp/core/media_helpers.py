"""Shared utility helpers used across MCP tools."""

import logging
import os
import pathlib
import platform
from typing import Any, Dict, Iterable, List, Optional

import httpx

PACKAGE_NAME = "armavita-meta-ads-mcp"
LOGGER_NAME = "armavita_meta_ads_mcp"

META_APP_ID = os.environ.get("META_APP_ID", "")
META_APP_SECRET = os.environ.get("META_APP_SECRET", "")


def _resolve_log_file() -> pathlib.Path:
    """Return a writable per-user log file path across OSes."""
    system_name = platform.system().lower()
    if system_name == "windows":
        root = pathlib.Path(os.environ.get("APPDATA", pathlib.Path.home()))
    elif system_name == "darwin":
        root = pathlib.Path.home() / "Library" / "Application Support"
    else:
        root = pathlib.Path.home() / ".config"

    log_dir = root / PACKAGE_NAME
    log_dir.mkdir(parents=True, exist_ok=True)
    return log_dir / "server.log"


def _create_logger() -> logging.Logger:
    logger = logging.getLogger(LOGGER_NAME)
    if logger.handlers:
        return logger

    logger.setLevel(logging.INFO)
    handler = logging.FileHandler(_resolve_log_file(), encoding="utf-8")
    formatter = logging.Formatter("%(asctime)s %(levelname)s %(name)s %(message)s")
    handler.setFormatter(formatter)
    logger.addHandler(handler)
    logger.propagate = False

    logger.info("logger_initialized")
    if not META_APP_ID:
        logger.warning("META_APP_ID is not configured")
    if not META_APP_SECRET:
        logger.warning("META_APP_SECRET is not configured")
    return logger


logger = _create_logger()


def _dedupe_preserving_order(values: Iterable[str]) -> List[str]:
    seen = set()
    unique: List[str] = []
    for value in values:
        if not value or value in seen:
            continue
        seen.add(value)
        unique.append(value)
    return unique


def _extract_asset_feed_urls(asset_feed_spec: Dict[str, Any]) -> List[str]:
    urls: List[str] = []
    images = asset_feed_spec.get("images")
    if isinstance(images, list):
        for image_entry in images:
            if isinstance(image_entry, dict):
                url = image_entry.get("url")
                if isinstance(url, str) and url:
                    urls.append(url)
    return urls


def extract_creative_image_urls(creative: Dict[str, Any]) -> List[str]:
    """Extract likely image URLs from creative payloads in quality-first order."""
    if not isinstance(creative, dict):
        return []

    candidates: List[str] = []

    high_priority = creative.get("image_urls_for_viewing")
    if isinstance(high_priority, list):
        candidates.extend([url for url in high_priority if isinstance(url, str) and url])

    direct_image_url = creative.get("image_url") or creative.get("image_source_url")
    if isinstance(direct_image_url, str) and direct_image_url:
        candidates.append(direct_image_url)

    story = creative.get("object_story_spec")
    if isinstance(story, dict):
        link_data = story.get("link_data")
        if isinstance(link_data, dict):
            for key in ("picture", "image_url", "image_source_url"):
                maybe_url = link_data.get(key)
                if isinstance(maybe_url, str) and maybe_url:
                    candidates.append(maybe_url)

        video_data = story.get("video_data")
        if isinstance(video_data, dict):
            maybe_video_image = video_data.get("image_url") or video_data.get("image_source_url")
            if isinstance(maybe_video_image, str) and maybe_video_image:
                candidates.append(maybe_video_image)

    asset_feed = creative.get("asset_feed_spec")
    if isinstance(asset_feed, dict):
        candidates.extend(_extract_asset_feed_urls(asset_feed))

    thumbnail_url = creative.get("thumbnail_url")
    if isinstance(thumbnail_url, str) and thumbnail_url:
        candidates.append(thumbnail_url)

    return _dedupe_preserving_order(candidates)


async def download_image(url: str) -> Optional[bytes]:
    """Download image bytes from a remote URL."""
    if not url:
        return None

    headers = {
        "User-Agent": "armavita-meta-ads-mcp/1.0",
        "Accept": "image/avif,image/webp,image/apng,image/*,*/*;q=0.8",
    }

    try:
        async with httpx.AsyncClient(follow_redirects=True, timeout=30.0) as client:
            response = await client.get(url, headers=headers)
            response.raise_for_status()
            return response.content
    except Exception as exc:  # noqa: BLE001
        logger.warning("image_download_failed url=%s error=%s", url, exc)
        return None


async def try_multiple_download_methods(url: str) -> Optional[bytes]:
    """Try multiple header profiles to retrieve image bytes from strict CDNs."""
    if not url:
        return None

    header_profiles = [
        {
            "User-Agent": "armavita-meta-ads-mcp/1.0",
            "Accept": "image/*,*/*;q=0.8",
        },
        {
            "User-Agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36",
            "Accept": "image/avif,image/webp,image/apng,image/*,*/*;q=0.8",
            "Referer": "https://www.facebook.com/",
        },
        {
            "User-Agent": "curl/8.4.0",
            "Accept": "*/*",
        },
    ]

    for profile in header_profiles:
        try:
            async with httpx.AsyncClient(follow_redirects=True, timeout=30.0) as client:
                response = await client.get(url, headers=profile)
                response.raise_for_status()
                return response.content
        except Exception as exc:  # noqa: BLE001
            logger.debug("image_download_attempt_failed url=%s ua=%s error=%s", url, profile.get("User-Agent"), exc)

    return None
