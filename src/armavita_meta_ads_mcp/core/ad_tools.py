# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""Ads, creatives, and page-discovery tools."""

import base64
import io
import json
import logging
import os
import time
from typing import Any, Dict, List, Optional, Tuple

from mcp.server.fastmcp import Image
from PIL import Image as PILImage

from .graph_client import McpToolError, make_api_request, meta_api_tool
from .mcp_runtime import mcp_server
from .media_helpers import (
    download_image,
    extract_creative_image_urls,
    try_multiple_download_methods,
)

logger = logging.getLogger(__name__)

_AD_FIELDS = (
    "id,name,adset_id,campaign_id,status,creative,created_time,updated_time,"
    "bid_amount,conversion_domain,tracking_specs"
)
_CREATIVE_FIELDS = (
    "id,name,status,thumbnail_url,image_url,image_hash,object_story_spec,"
    "asset_feed_spec,image_urls_for_viewing,url_tags,link_url"
)
_PAGE_FIELDS = "id,name,username,category,fan_count,link,verification_status,picture"
_PREVIEW_FALLBACK_AD_FORMATS = (
    "DESKTOP_FEED_STANDARD",
    "MOBILE_FEED_STANDARD",
    "INSTAGRAM_STANDARD",
    "FACEBOOK_STORY_MOBILE",
    "INSTAGRAM_STORY",
)

_PLACEMENT_GROUP_TO_SPEC: Dict[str, Dict[str, List[str]]] = {
    "FEED": {
        "publisher_platforms": ["facebook", "instagram"],
        "facebook_positions": ["feed"],
        "instagram_positions": ["stream", "profile_feed"],
    },
    "STORY": {
        "publisher_platforms": ["facebook", "instagram"],
        "facebook_positions": ["story"],
        "instagram_positions": ["story"],
    },
    "MESSENGER": {"publisher_platforms": ["messenger"]},
    "INSTREAM_VIDEO": {
        "publisher_platforms": ["facebook"],
        "facebook_positions": ["instream_video"],
    },
    "SEARCH": {
        "publisher_platforms": ["facebook"],
        "facebook_positions": ["search"],
    },
    "SHOP": {
        "publisher_platforms": ["instagram"],
        "instagram_positions": ["shop"],
    },
    "AUDIENCE_NETWORK": {
        "publisher_platforms": ["audience_network"],
        "audience_network_positions": ["classic", "instream_video"],
    },
}


def _json(payload: Dict[str, Any]) -> str:
    return json.dumps(payload, indent=2)


def _normalize_ad_account_id(ad_account_id: str) -> str:
    ad_account_id = str(ad_account_id or "").strip()
    if not ad_account_id:
        return ""
    return ad_account_id if ad_account_id.startswith("act_") else f"act_{ad_account_id}"


def _preview_requires_ad_format(payload: Dict[str, Any]) -> bool:
    if not isinstance(payload, dict):
        return False
    error = payload.get("error")
    if not error:
        return False
    message = json.dumps(error, default=str).lower()
    return "ad_format" in message and "required" in message


def _parse_json_list(value: Any) -> Any:
    if not isinstance(value, str):
        return value
    try:
        parsed = json.loads(value)
        if isinstance(parsed, list):
            return parsed
    except (TypeError, ValueError):
        pass
    return value


def _normalize_list_argument(value: Any) -> Any:
    parsed = _parse_json_list(value)
    return parsed if parsed is not None else value


def _ensure_single_media_choice(
    ad_image_hash: Optional[str],
    ad_image_hashes: Optional[List[str]],
    ad_video_id: Optional[str],
) -> Optional[str]:
    chosen = sum(1 for item in (ad_image_hash, ad_image_hashes, ad_video_id) if item)
    if chosen > 1:
        return (
            "Only one media source allowed. Use ad_image_hash for one image, "
            "ad_image_hashes for variants, or ad_video_id for video creatives."
        )
    if chosen == 0:
        return "No media provided. Set ad_image_hash, ad_image_hashes, or ad_video_id."
    return None


def _normalize_flexible_asset_lists(
    primary_text: Optional[str],
    primary_text_variants: Optional[List[str]],
    headline_text: Optional[str],
    headline_variants: Optional[List[str]],
    description_text: Optional[str],
    description_variants: Optional[List[str]],
) -> Tuple[Optional[Dict[str, Any]], Dict[str, Any]]:
    if primary_text and primary_text_variants:
        return ({
            "error": "Cannot specify both 'primary_text' and 'primary_text_variants'.",
            "details": "Use primary_text for single text or primary_text_variants for multi-variant creatives.",
        }, {})
    if headline_text and headline_variants:
        return ({
            "error": "Cannot specify both 'headline_text' and 'headline_variants'.",
            "details": "Use headline_text for single text or headline_variants for multi-variant creatives.",
        }, {})
    if description_text and description_variants:
        return ({
            "error": "Cannot specify both 'description_text' and 'description_variants'.",
            "details": "Use description_text for single text or description_variants for multi-variant creatives.",
        }, {})

    normalized = {
        "primary_text_variants": list(primary_text_variants) if primary_text_variants else ([primary_text] if primary_text else []),
        "headline_variants": list(headline_variants) if headline_variants else ([headline_text] if headline_text else []),
        "description_variants": list(description_variants) if description_variants else ([description_text] if description_text else []),
    }

    if len(normalized["headline_variants"]) > 5:
        return ({"error": "Maximum 5 headline_variants allowed for dynamic creatives"}, {})
    if len(normalized["description_variants"]) > 5:
        return ({"error": "Maximum 5 description_variants allowed for dynamic creatives"}, {})

    for idx, text in enumerate(normalized["headline_variants"]):
        if len(str(text)) > 40:
            return ({"error": f"Headline {idx + 1} exceeds 40 character limit"}, {})

    for idx, text in enumerate(normalized["description_variants"]):
        if len(str(text)) > 125:
            return ({"error": f"Description {idx + 1} exceeds 125 character limit"}, {})

    return None, normalized


def _asset_text_entries(values: List[str]) -> List[Dict[str, str]]:
    return [{"text": str(value)} for value in values if str(value)]


def _resolve_default_ad_formats(
    ad_formats: Optional[List[str]],
    optimization_type: Optional[str],
    has_video: bool,
    has_image_variants: bool,
) -> List[str]:
    if ad_formats:
        return list(ad_formats)
    if has_video:
        return ["SINGLE_VIDEO"]
    if optimization_type == "DEGREES_OF_FREEDOM" and has_image_variants:
        return ["AUTOMATIC_FORMAT"]
    return ["SINGLE_IMAGE"]


def _translate_asset_customization_rules(
    rules: List[Dict[str, Any]],
    images_array: List[Dict[str, Any]],
) -> Tuple[List[Dict[str, Any]], List[Dict[str, Any]]]:
    """Translate placement_groups-based rules into Meta asset labels + customization_spec."""
    if not rules or not any(isinstance(rule, dict) and rule.get("placement_groups") for rule in rules):
        return rules, images_array

    translated_rules: List[Dict[str, Any]] = []
    hash_to_label: Dict[str, str] = {}
    label_counter = 0

    for rule in rules:
        if not isinstance(rule, dict) or "placement_groups" not in rule:
            translated_rules.append(rule)
            continue

        groups = rule.get("placement_groups", [])
        customization_input = rule.get("customization_spec", {})

        publisher_platforms = set()
        facebook_positions = set()
        instagram_positions = set()
        audience_network_positions = set()

        for group in groups:
            mapping = _PLACEMENT_GROUP_TO_SPEC.get(str(group).upper(), {})
            publisher_platforms.update(mapping.get("publisher_platforms", []))
            facebook_positions.update(mapping.get("facebook_positions", []))
            instagram_positions.update(mapping.get("instagram_positions", []))
            audience_network_positions.update(mapping.get("audience_network_positions", []))

        customization_spec: Dict[str, Any] = {}
        if publisher_platforms:
            customization_spec["publisher_platforms"] = sorted(publisher_platforms)
        if facebook_positions:
            customization_spec["facebook_positions"] = sorted(facebook_positions)
        if instagram_positions:
            customization_spec["instagram_positions"] = sorted(instagram_positions)
        if audience_network_positions:
            customization_spec["audience_network_positions"] = sorted(audience_network_positions)

        for key in ("bodies", "titles", "description_variants", "link_urls", "call_to_action_types"):
            if key in customization_input:
                customization_spec[key] = customization_input[key]

        translated_rule: Dict[str, Any] = {"customization_spec": customization_spec}

        ad_image_hashes = customization_input.get("ad_image_hashes", []) if isinstance(customization_input, dict) else []
        video_ids = customization_input.get("video_ids", []) if isinstance(customization_input, dict) else []

        if ad_image_hashes:
            selected_hash = ad_image_hashes[0]
            if selected_hash not in hash_to_label:
                hash_to_label[selected_hash] = f"ARMAVITA_IMG_{label_counter}"
                label_counter += 1
            translated_rule["image_label"] = {"name": hash_to_label[selected_hash]}
        elif video_ids:
            selected_video = video_ids[0]
            if selected_video not in hash_to_label:
                hash_to_label[selected_video] = f"ARMAVITA_VID_{label_counter}"
                label_counter += 1
            translated_rule["video_label"] = {"name": hash_to_label[selected_video]}

        translated_rules.append(translated_rule)

    relabeled_images: List[Dict[str, Any]] = []
    for image in images_array:
        ad_image_hash = image.get("hash")
        if ad_image_hash and ad_image_hash in hash_to_label:
            updated = dict(image)
            updated["adlabels"] = [{"name": hash_to_label[ad_image_hash]}]
            relabeled_images.append(updated)
        else:
            relabeled_images.append(image)

    return translated_rules, relabeled_images


async def _fetch_video_thumbnail(ad_video_id: str, meta_access_token: str) -> Optional[str]:
    payload = await make_api_request(ad_video_id, meta_access_token, {"fields": "picture"})
    if isinstance(payload, dict):
        picture = payload.get("picture")
        if isinstance(picture, str) and picture:
            return picture
    return None


def _decode_json_if_string(value: Any) -> Any:
    if not isinstance(value, str):
        return value
    try:
        return json.loads(value)
    except (TypeError, ValueError):
        return value


def _build_image_from_bytes(raw: bytes) -> Image:
    image = PILImage.open(io.BytesIO(raw))
    if image.mode != "RGB":
        image = image.convert("RGB")
    buffer = io.BytesIO()
    image.save(buffer, format="JPEG")
    return Image(data=buffer.getvalue(), format="jpeg")


def _ad_image_error(ad_id: str, stage: str, primary_text: str, meta_response: Optional[Dict[str, Any]] = None) -> None:
    payload: Dict[str, Any] = {
        "error": "read_ad_image_failed",
        "message": primary_text,
        "stage": stage,
        "ad_id": ad_id,
    }
    if meta_response is not None:
        payload["meta_response"] = meta_response
    raise McpToolError(json.dumps(payload, indent=2))


async def _get_creative_id_for_ad(ad_id: str, meta_access_token: str) -> Tuple[str, str]:
    ad_payload = await make_api_request(ad_id, meta_access_token, {"fields": "creative{id},account_id"})
    if isinstance(ad_payload, dict) and ad_payload.get("error"):
        _ad_image_error(ad_id, "fetch_ad", "Could not get ad data", ad_payload)

    ad_account_id = str(ad_payload.get("account_id", "")) if isinstance(ad_payload, dict) else ""
    creative = ad_payload.get("creative") if isinstance(ad_payload, dict) else None
    ad_creative_id = str(creative.get("id", "")) if isinstance(creative, dict) else ""

    if not ad_account_id:
        _ad_image_error(ad_id, "fetch_ad", "No account ID found in ad data", ad_payload)
    if not ad_creative_id:
        _ad_image_error(ad_id, "fetch_ad", "No creative ID found", ad_payload)

    return ad_account_id, ad_creative_id


async def _extract_creative_image_hashes(ad_creative_id: str, meta_access_token: str) -> List[str]:
    payload = await make_api_request(ad_creative_id, meta_access_token, {"fields": "id,name,image_hash,asset_feed_spec"})
    hashes: List[str] = []

    if isinstance(payload, dict):
        if payload.get("image_hash"):
            hashes.append(str(payload["image_hash"]))

        asset_feed = payload.get("asset_feed_spec")
        if isinstance(asset_feed, dict) and isinstance(asset_feed.get("images"), list):
            for entry in asset_feed["images"]:
                if isinstance(entry, dict) and entry.get("hash"):
                    hashes.append(str(entry["hash"]))

    deduped: List[str] = []
    seen = set()
    for value in hashes:
        if value and value not in seen:
            seen.add(value)
            deduped.append(value)
    return deduped


async def _load_fallback_creatives(ad_id: str, meta_access_token: str) -> Optional[Dict[str, Any]]:
    creatives_raw = await list_ad_creatives(ad_id=ad_id, meta_access_token=meta_access_token)
    creatives_payload = _decode_json_if_string(creatives_raw)

    if isinstance(creatives_payload, dict) and isinstance(creatives_payload.get("data"), str):
        creatives_payload = _decode_json_if_string(creatives_payload["data"])

    return creatives_payload if isinstance(creatives_payload, dict) else None


def _fallback_creative_image_url_from_payload(creatives_payload: Optional[Dict[str, Any]]) -> Optional[str]:
    if not isinstance(creatives_payload, dict):
        return None
    rows = creatives_payload.get("data") if isinstance(creatives_payload.get("data"), list) else []
    if not rows:
        return None

    first = rows[0] if isinstance(rows[0], dict) else {}

    ordered_urls = extract_creative_image_urls(first)
    return ordered_urls[0] if ordered_urls else None


def _fallback_creative_image_hash_from_payload(creatives_payload: Optional[Dict[str, Any]]) -> Optional[str]:
    if not isinstance(creatives_payload, dict):
        return None

    rows = creatives_payload.get("data") if isinstance(creatives_payload.get("data"), list) else []
    for row in rows:
        if not isinstance(row, dict):
            continue
        direct_hash = row.get("image_hash") or row.get("ad_image_hash")
        if direct_hash:
            return str(direct_hash)

        object_story_spec = row.get("object_story_spec")
        if isinstance(object_story_spec, dict):
            link_data = object_story_spec.get("link_data")
            if isinstance(link_data, dict):
                link_hash = link_data.get("image_hash") or link_data.get("ad_image_hash")
                if link_hash:
                    return str(link_hash)

        asset_feed = row.get("asset_feed_spec")
        if isinstance(asset_feed, dict) and isinstance(asset_feed.get("images"), list):
            for image in asset_feed["images"]:
                if isinstance(image, dict) and image.get("hash"):
                    return str(image["hash"])
    return None


async def _load_image_url_from_hash(ad_account_id: str, meta_access_token: str, ad_image_hash: str) -> Optional[str]:
    endpoint = f"act_{ad_account_id}/adimages"
    payload = await make_api_request(
        endpoint,
        meta_access_token,
        {
            "fields": "hash,url,width,height,name,status",
            "hashes": json.dumps([ad_image_hash]),
        },
    )

    if isinstance(payload, dict) and payload.get("error"):
        return None

    rows = payload.get("data") if isinstance(payload, dict) else None
    if not isinstance(rows, list) or not rows:
        return None

    first = rows[0] if isinstance(rows[0], dict) else {}
    url = first.get("url")
    return str(url) if url else None


def _build_tracking_specs(tracking_specs: Optional[List[Dict[str, Any]]]) -> Optional[str]:
    if tracking_specs is None:
        return None
    return json.dumps(tracking_specs)


@mcp_server.tool()
@meta_api_tool
async def list_ads(
    ad_account_id: str,
    meta_access_token: Optional[str] = None,
    page_size: int = 10,
    campaign_id: str = "",
    ad_set_id: str = "",
    page_cursor: str = "",
) -> str:
    if not ad_account_id:
        return _json({"error": "No account ID specified"})

    target_id = ad_set_id or campaign_id or ad_account_id
    endpoint = f"{target_id}/ads"

    params: Dict[str, Any] = {"fields": _AD_FIELDS, "page_size": int(page_size)}
    if page_cursor:
        params["page_cursor"] = page_cursor

    payload = await make_api_request(endpoint, meta_access_token, params)
    return _json(payload)


@mcp_server.tool()
@meta_api_tool
async def read_ad(ad_id: str, meta_access_token: Optional[str] = None) -> str:
    if not ad_id:
        return _json({"error": "No ad ID provided"})

    payload = await make_api_request(
        ad_id,
        meta_access_token,
        {"fields": _AD_FIELDS + ",preview_shareable_link"},
    )
    return _json(payload)


@mcp_server.tool()
@meta_api_tool
async def list_ad_previews(
    ad_id: str,
    meta_access_token: Optional[str] = None,
    ad_format: Optional[str] = None,
    locale: Optional[str] = None,
    render_type: Optional[str] = None,
    width: Optional[int] = None,
    height: Optional[int] = None,
) -> str:
    if not ad_id:
        return _json({"error": "No ad ID provided"})

    params: Dict[str, Any] = {}
    if ad_format:
        params["ad_format"] = ad_format
    if locale:
        params["locale"] = locale
    if render_type:
        params["render_type"] = render_type
    if width is not None:
        params["width"] = width
    if height is not None:
        params["height"] = height

    payload = await make_api_request(f"{ad_id}/previews", meta_access_token, params)

    if not ad_format and _preview_requires_ad_format(payload):
        attempted_formats: List[str] = []
        for fallback_format in _PREVIEW_FALLBACK_AD_FORMATS:
            attempted_formats.append(fallback_format)
            fallback_params = dict(params)
            fallback_params["ad_format"] = fallback_format
            fallback_payload = await make_api_request(f"{ad_id}/previews", meta_access_token, fallback_params)
            if not (isinstance(fallback_payload, dict) and fallback_payload.get("error")):
                if isinstance(fallback_payload, dict):
                    fallback_payload.setdefault("request_context", {})
                    if isinstance(fallback_payload["request_context"], dict):
                        fallback_payload["request_context"]["ad_format"] = fallback_format
                        fallback_payload["request_context"]["auto_selected"] = True
                return _json(fallback_payload)

        if isinstance(payload.get("error"), dict):
            payload["error"]["attempted_ad_formats"] = attempted_formats

    return _json(payload)


@mcp_server.tool()
@meta_api_tool
async def read_ad_creative(ad_creative_id: str, meta_access_token: Optional[str] = None) -> str:
    if not ad_creative_id:
        return _json({"error": "No creative ID provided"})

    payload = await make_api_request(ad_creative_id, meta_access_token, {"fields": _CREATIVE_FIELDS.replace(",image_urls_for_viewing", "")})

    if isinstance(payload, dict) and payload.get("id"):
        try:
            dynamic_payload = await make_api_request(ad_creative_id, meta_access_token, {"fields": "dynamic_creative_spec"})
            if isinstance(dynamic_payload, dict) and "dynamic_creative_spec" in dynamic_payload:
                payload["dynamic_creative_spec"] = dynamic_payload["dynamic_creative_spec"]
        except Exception:  # noqa: BLE001
            pass

    return _json(payload)


@mcp_server.tool()
@meta_api_tool
async def create_ad(
    ad_account_id: str,
    name: str,
    ad_set_id: str,
    ad_creative_id: str,
    status: str = "PAUSED",
    bid_amount: Optional[int] = None,
    tracking_specs: Optional[List[Dict[str, Any]]] = None,
    meta_access_token: Optional[str] = None,
) -> str:
    if not ad_account_id:
        return _json({"error": "No account ID provided"})
    if not name:
        return _json({"error": "No ad name provided"})
    if not ad_set_id:
        return _json({"error": "No ad set ID provided"})
    if not ad_creative_id:
        return _json({"error": "No creative ID provided"})

    payload: Dict[str, Any] = {
        "name": name,
        "ad_set_id": ad_set_id,
        "creative": {"ad_creative_id": ad_creative_id},
        "status": status,
    }
    if bid_amount is not None:
        payload["bid_amount"] = str(bid_amount)

    encoded_tracking = _build_tracking_specs(tracking_specs)
    if encoded_tracking is not None:
        payload["tracking_specs"] = encoded_tracking

    result = await make_api_request(f"{ad_account_id}/ads", meta_access_token, payload, method="POST")
    return _json(result)


@mcp_server.tool()
@meta_api_tool
async def list_ad_creatives(
    ad_id: str,
    meta_access_token: Optional[str] = None,
    page_cursor: str = "",
) -> str:
    if not ad_id:
        return _json({"error": "No ad ID provided"})

    params: Dict[str, Any] = {"fields": _CREATIVE_FIELDS}
    if page_cursor:
        params["page_cursor"] = page_cursor

    payload = await make_api_request(
        f"{ad_id}/adcreatives",
        meta_access_token,
        params,
    )

    if isinstance(payload, dict) and isinstance(payload.get("data"), list):
        for row in payload["data"]:
            if isinstance(row, dict):
                row["image_urls_for_viewing"] = extract_creative_image_urls(row)

    return _json(payload)


@mcp_server.tool()
@meta_api_tool
async def read_ad_image(ad_id: str, meta_access_token: Optional[str] = None) -> Image:
    if not ad_id:
        _ad_image_error(str(ad_id), "validation", "No ad ID provided")

    ad_account_id, ad_creative_id = await _get_creative_id_for_ad(ad_id, meta_access_token)

    ad_image_hashes = await _extract_creative_image_hashes(ad_creative_id, meta_access_token)

    image_source_url = None
    if ad_image_hashes:
        image_source_url = await _load_image_url_from_hash(ad_account_id, meta_access_token, ad_image_hashes[0])

    fallback_creatives: Optional[Dict[str, Any]] = None
    if not image_source_url and not ad_image_hashes:
        fallback_creatives = await _load_fallback_creatives(ad_id, meta_access_token)
        fallback_hash = _fallback_creative_image_hash_from_payload(fallback_creatives)
        if fallback_hash:
            image_source_url = await _load_image_url_from_hash(ad_account_id, meta_access_token, fallback_hash)

    if not image_source_url:
        if fallback_creatives is None:
            fallback_creatives = await _load_fallback_creatives(ad_id, meta_access_token)
        image_source_url = _fallback_creative_image_url_from_payload(fallback_creatives)

    if not image_source_url:
        _ad_image_error(ad_id, "extract_image_url", "No image URLs found in creative")

    image_bytes = await download_image(image_source_url)
    if not image_bytes:
        _ad_image_error(ad_id, "download_image", "Failed to download image from creative URL")

    try:
        return _build_image_from_bytes(image_bytes)
    except Exception as exc:  # noqa: BLE001
        _ad_image_error(ad_id, "process_image", f"Error processing image: {exc}")


@mcp_server.tool()
@meta_api_tool
async def export_ad_image_file(
    ad_id: str,
    meta_access_token: Optional[str] = None,
    output_dir: str = "ad_images",
) -> str:
    if not ad_id:
        return _json({"error": "No ad ID provided"})

    image = await read_ad_image(ad_id=ad_id, meta_access_token=meta_access_token)
    if not isinstance(image, Image):
        return _json({"error": "Unexpected image response type"})

    os.makedirs(output_dir, exist_ok=True)
    file_path = os.path.join(output_dir, f"{ad_id}.jpg")
    with open(file_path, "wb") as handle:
        handle.write(image.data)

    return _json({"filepath": file_path})


@mcp_server.tool()
@meta_api_tool
async def update_ad(
    ad_id: str,
    status: Optional[str] = None,
    bid_amount: Optional[int] = None,
    tracking_specs: Optional[List[Dict[str, Any]]] = None,
    ad_creative_id: Optional[str] = None,
    meta_access_token: Optional[str] = None,
) -> str:
    if not ad_id:
        return _json({"error": "Ad ID is required"})

    payload: Dict[str, Any] = {}
    if status is not None:
        payload["status"] = status
    if bid_amount is not None:
        payload["bid_amount"] = str(bid_amount)
    if tracking_specs is not None:
        payload["tracking_specs"] = json.dumps(tracking_specs)
    if ad_creative_id is not None:
        payload["creative"] = json.dumps({"ad_creative_id": ad_creative_id})

    if not payload:
        return _json({"error": "No update parameters provided (status, bid_amount, tracking_specs, or ad_creative_id)"})

    result = await make_api_request(ad_id, meta_access_token, payload, method="POST")
    return _json(result)


def _infer_image_name_from_url(url: str) -> str:
    basename = os.path.basename((url or "").split("?")[0])
    return basename or "upload.jpg"


def _normalize_uploaded_images_payload(payload: Dict[str, Any], ad_account_id: str, final_name: str) -> Dict[str, Any]:
    images = payload.get("images") if isinstance(payload, dict) else None
    if isinstance(images, dict) and images:
        normalized_images = []
        for hash_key, details in images.items():
            if isinstance(details, dict):
                row = {
                    "hash": details.get("hash") or hash_key,
                    "url": details.get("url"),
                    "width": details.get("width"),
                    "height": details.get("height"),
                    "name": details.get("name"),
                }
            else:
                row = {"hash": hash_key}
            normalized_images.append({k: v for k, v in row.items() if v is not None})

        normalized_images.sort(key=lambda item: item.get("hash", ""))
        primary_hash = normalized_images[0].get("hash") if normalized_images else None
        return {
            "success": True,
            "ad_account_id": ad_account_id,
            "name": final_name,
            "ad_image_hash": primary_hash,
            "images_count": len(normalized_images),
            "images": normalized_images,
        }

    if isinstance(payload, dict) and payload.get("error"):
        return {
            "error": "Failed to upload image",
            "details": payload.get("error"),
            "ad_account_id": ad_account_id,
            "name": final_name,
        }

    return {
        "success": True,
        "ad_account_id": ad_account_id,
        "name": final_name,
        "raw_response": payload,
    }


@mcp_server.tool()
@meta_api_tool
async def upload_ad_image_asset(
    ad_account_id: str,
    meta_access_token: Optional[str] = None,
    image_file_path: Optional[str] = None,
    image_source_url: Optional[str] = None,
    name: Optional[str] = None,
) -> str:
    if not ad_account_id:
        return _json({"error": "No account ID provided"})

    if not image_file_path and not image_source_url:
        return _json({"error": "Provide either 'image_file_path' (data URL or base64) or 'image_source_url'"})

    normalized_account_id = _normalize_ad_account_id(ad_account_id)
    encoded_image = ""
    inferred_name = name or ""

    if image_file_path:
        if image_file_path.startswith("data:") and "base64," in image_file_path:
            header, encoded_image = image_file_path.split("base64,", 1)
            encoded_image = encoded_image.strip()
            if not inferred_name:
                mime_type = header[len("data:"):].split(";")[0]
                extension = {
                    "image/png": ".png",
                    "image/jpeg": ".jpg",
                    "image/jpg": ".jpg",
                    "image/webp": ".webp",
                    "image/gif": ".gif",
                }.get(mime_type, ".png")
                inferred_name = f"upload{extension}"
        else:
            encoded_image = image_file_path.strip()
            if not inferred_name:
                inferred_name = "upload.png"
    else:
        try:
            image_bytes = await try_multiple_download_methods(image_source_url)
        except Exception as exc:  # noqa: BLE001
            return _json(
                {
                    "error": "We couldn’t download the image from the link provided.",
                    "reason": "The server returned an error while trying to fetch the image.",
                    "image_source_url": image_source_url,
                    "details": str(exc),
                    "suggestions": [
                        "Upload in Ads Manager, copy hash, and use ad_image_hash directly.",
                        "Ensure the URL is public (no login/VPN/IP restriction).",
                        "Move private files to a public CDN URL.",
                    ],
                }
            )

        if not image_bytes:
            return _json(
                {
                    "error": "We couldn’t access the image at the link you provided.",
                    "reason": "The URL is not publicly accessible or returned empty data.",
                    "image_source_url": image_source_url,
                    "suggestions": [
                        "Upload in Ads Manager and reuse image hash.",
                        "Use a public direct image URL.",
                    ],
                }
            )

        encoded_image = base64.b64encode(image_bytes).decode("utf-8")
        if not inferred_name:
            inferred_name = _infer_image_name_from_url(image_source_url)

    final_name = name or inferred_name or "upload.png"

    api_payload = {
        "bytes": encoded_image,
        "name": final_name,
    }

    response = await make_api_request(f"{normalized_account_id}/adimages", meta_access_token, api_payload, method="POST")
    return _json(_normalize_uploaded_images_payload(response, normalized_account_id, final_name))


def _sanitize_instagram_identity(
    instagram_user_id: Optional[str],
    instagram_actor_id: Optional[str],
) -> Tuple[Optional[str], Optional[str]]:
    meta_user_id = str(instagram_user_id).strip() if instagram_user_id else None
    actor_id = str(instagram_actor_id).strip() if instagram_actor_id else None
    if meta_user_id:
        return meta_user_id, None
    return None, actor_id


def _build_simple_image_story_spec(
    facebook_page_id: str,
    ad_image_hash: str,
    link_url: Optional[str],
    primary_text: Optional[str],
    headline_text: Optional[str],
    description_text: Optional[str],
    call_to_action_type: Optional[str],
    lead_form_id: Optional[str],
) -> Dict[str, Any]:
    link_data: Dict[str, Any] = {
        "image_hash": ad_image_hash,
        "link": link_url,
    }

    if primary_text:
        link_data["message"] = primary_text
    if headline_text:
        link_data["name"] = headline_text
    if description_text:
        link_data["description"] = description_text
    if call_to_action_type:
        cta: Dict[str, Any] = {"type": call_to_action_type}
        if lead_form_id:
            cta["value"] = {"lead_gen_form_id": lead_form_id}
        link_data["call_to_action"] = cta

    return {
        "page_id": facebook_page_id,
        "link_data": link_data,
    }


def _build_simple_video_story_spec(
    facebook_page_id: str,
    ad_video_id: str,
    link_url: Optional[str],
    primary_text: Optional[str],
    headline_text: Optional[str],
    thumbnail_url: Optional[str],
    call_to_action_type: Optional[str],
    lead_form_id: Optional[str],
) -> Dict[str, Any]:
    video_data: Dict[str, Any] = {"video_id": ad_video_id}
    if thumbnail_url:
        video_data["image_url"] = thumbnail_url
    if primary_text:
        video_data["message"] = primary_text
    if headline_text:
        video_data["title"] = headline_text

    cta_value: Dict[str, Any] = {}
    if link_url:
        cta_value["link"] = link_url
    if lead_form_id:
        cta_value["lead_gen_form_id"] = lead_form_id

    cta_type = call_to_action_type or ("LEARN_MORE" if link_url else None)
    if cta_type:
        cta_payload: Dict[str, Any] = {"type": cta_type}
        if cta_value:
            cta_payload["value"] = cta_value
        video_data["call_to_action"] = cta_payload

    return {
        "page_id": facebook_page_id,
        "video_data": video_data,
    }


def _build_asset_feed_spec_payload(
    link_url: Optional[str],
    normalized_assets: Dict[str, Any],
    ad_image_hash: Optional[str],
    ad_image_hashes: Optional[List[str]],
    ad_video_id: Optional[str],
    thumbnail_url: Optional[str],
    optimization_type: Optional[str],
    ad_formats: Optional[List[str]],
    call_to_action_type: Optional[str],
    asset_customization_rules: Optional[List[Dict[str, Any]]],
) -> Tuple[Dict[str, Any], Dict[str, Any]]:
    has_video = bool(ad_video_id)
    image_pool = [{"hash": h} for h in (ad_image_hashes or [])]
    if not has_video and not image_pool and ad_image_hash:
        image_pool = [{"hash": ad_image_hash}]

    if asset_customization_rules and image_pool and not has_video:
        asset_customization_rules, image_pool = _translate_asset_customization_rules(asset_customization_rules, image_pool)

    feed: Dict[str, Any] = {
        "link_urls": [{"website_url": link_url}],
        "ad_formats": _resolve_default_ad_formats(ad_formats, optimization_type, has_video, bool(ad_image_hashes)),
    }
    if optimization_type:
        feed["optimization_type"] = optimization_type

    if has_video:
        video_entry = {"video_id": ad_video_id}
        if thumbnail_url:
            video_entry["thumbnail_url"] = thumbnail_url
        feed["videos"] = [video_entry]
    else:
        feed["images"] = image_pool

    if normalized_assets["headline_variants"]:
        feed["titles"] = _asset_text_entries(normalized_assets["headline_variants"])
    if normalized_assets["description_variants"]:
        feed["descriptions"] = _asset_text_entries(normalized_assets["description_variants"])
    if normalized_assets["primary_text_variants"]:
        feed["bodies"] = _asset_text_entries(normalized_assets["primary_text_variants"])
    if call_to_action_type:
        feed["call_to_action_types"] = [call_to_action_type]
    if asset_customization_rules:
        feed["asset_customization_rules"] = asset_customization_rules

    if has_video:
        story_spec = {
            "page_id": None,  # filled by caller
            "video_data": {
                "video_id": ad_video_id,
                **({"image_url": thumbnail_url} if thumbnail_url else {}),
            },
        }
    else:
        anchor_hash = image_pool[0].get("hash") if image_pool else None
        link_data = {"link": link_url}
        if anchor_hash:
            link_data["image_hash"] = anchor_hash
        story_spec = {
            "page_id": None,
            "link_data": link_data,
        }

    return feed, story_spec


async def _resolve_page_id_for_creative(
    ad_account_id: str,
    meta_access_token: str,
    facebook_page_id: Optional[str],
) -> Tuple[Optional[str], Optional[Dict[str, Any]]]:
    if facebook_page_id:
        return str(facebook_page_id), None

    discovery = await _discover_pages_for_account(ad_account_id, meta_access_token)
    if not discovery.get("success"):
        return None, {
            "error": "No page ID provided and no suitable pages found for this account",
            "details": discovery.get("message", "Page discovery failed"),
            "suggestions": [
                "Use list_account_pages to list available pages.",
                "Use search_pages to filter by page name.",
                "Provide facebook_page_id explicitly.",
            ],
        }

    return str(discovery["facebook_page_id"]), None


@mcp_server.tool()
@meta_api_tool
async def create_ad_creative(
    ad_account_id: str,
    ad_image_hash: Optional[str] = None,
    meta_access_token: Optional[str] = None,
    name: Optional[str] = None,
    facebook_page_id: Optional[str] = None,
    link_url: Optional[str] = None,
    primary_text: Optional[str] = None,
    primary_text_variants: Optional[List[str]] = None,
    headline_text: Optional[str] = None,
    headline_variants: Optional[List[str]] = None,
    description_text: Optional[str] = None,
    description_variants: Optional[List[str]] = None,
    ad_image_hashes: Optional[List[str]] = None,
    ad_video_id: Optional[str] = None,
    thumbnail_url: Optional[str] = None,
    optimization_type: Optional[str] = None,
    dynamic_creative_spec: Optional[Dict[str, Any]] = None,
    call_to_action_type: Optional[str] = None,
    lead_form_id: Optional[str] = None,
    instagram_actor_id: Optional[str] = None,
    ad_formats: Optional[List[str]] = None,
    asset_customization_rules: Optional[List[Dict[str, Any]]] = None,
) -> str:
    if not ad_account_id:
        return _json({"error": "No account ID provided"})

    ad_image_hashes = _normalize_list_argument(ad_image_hashes)
    primary_text_variants = _normalize_list_argument(primary_text_variants)
    headline_variants = _normalize_list_argument(headline_variants)
    description_variants = _normalize_list_argument(description_variants)
    ad_formats = _normalize_list_argument(ad_formats)
    asset_customization_rules = _normalize_list_argument(asset_customization_rules)

    media_error = _ensure_single_media_choice(ad_image_hash, ad_image_hashes, ad_video_id)
    if media_error:
        return _json({"error": media_error})

    if ad_image_hashes and len(ad_image_hashes) > 10:
        return _json({"error": "Maximum 10 image hashes allowed for FLEX creatives"})

    if thumbnail_url and not ad_video_id:
        return _json({"error": "thumbnail_url can only be used with ad_video_id"})

    if optimization_type and optimization_type != "DEGREES_OF_FREEDOM":
        return _json({"error": f"Invalid optimization_type '{optimization_type}'. Only 'DEGREES_OF_FREEDOM' is supported."})

    normalized_lists_error, normalized_assets = _normalize_flexible_asset_lists(
        primary_text,
        primary_text_variants,
        headline_text,
        headline_variants,
        description_text,
        description_variants,
    )
    if normalized_lists_error:
        return _json(normalized_lists_error)

    if not link_url and not lead_form_id:
        return _json(
            {
                "error": "No link_url provided. A destination URL is required for ad creatives (unless using lead_form_id)."
            }
        )

    if ad_video_id and not thumbnail_url:
        try:
            thumbnail_url = await _fetch_video_thumbnail(ad_video_id, meta_access_token)
        except Exception:  # noqa: BLE001
            pass

    normalized_account_id = _normalize_ad_account_id(ad_account_id)
    final_name = name or f"Creative {int(time.time())}"

    resolved_page_id, page_error = await _resolve_page_id_for_creative(normalized_account_id, meta_access_token, facebook_page_id)
    if page_error:
        return _json(page_error)

    resolved_instagram_user_id, resolved_instagram_actor_id = _sanitize_instagram_identity(
        None,
        instagram_actor_id,
    )

    use_asset_feed = bool(primary_text_variants or headline_variants or description_variants or ad_image_hashes or optimization_type)

    creative_payload: Dict[str, Any] = {"name": final_name}

    if use_asset_feed:
        feed, story_spec = _build_asset_feed_spec_payload(
            link_url=link_url,
            normalized_assets=normalized_assets,
            ad_image_hash=ad_image_hash,
            ad_image_hashes=ad_image_hashes,
            ad_video_id=ad_video_id,
            thumbnail_url=thumbnail_url,
            optimization_type=optimization_type,
            ad_formats=ad_formats,
            call_to_action_type=call_to_action_type,
            asset_customization_rules=asset_customization_rules,
        )
        story_spec["page_id"] = resolved_page_id
        creative_payload["asset_feed_spec"] = feed
        creative_payload["object_story_spec"] = story_spec
    else:
        if ad_video_id:
            creative_payload["object_story_spec"] = _build_simple_video_story_spec(
                facebook_page_id=resolved_page_id,
                ad_video_id=ad_video_id,
                link_url=link_url,
                primary_text=primary_text,
                headline_text=headline_text,
                thumbnail_url=thumbnail_url,
                call_to_action_type=call_to_action_type,
                lead_form_id=lead_form_id,
            )
        else:
            creative_payload["object_story_spec"] = _build_simple_image_story_spec(
                facebook_page_id=resolved_page_id,
                ad_image_hash=ad_image_hash,
                link_url=link_url,
                primary_text=primary_text,
                headline_text=headline_text,
                description_text=description_text,
                call_to_action_type=call_to_action_type,
                lead_form_id=lead_form_id,
            )

    if dynamic_creative_spec:
        creative_payload["dynamic_creative_spec"] = dynamic_creative_spec

    if resolved_instagram_user_id:
        creative_payload["object_story_spec"]["instagram_user_id"] = resolved_instagram_user_id
    elif resolved_instagram_actor_id:
        creative_payload["object_story_spec"]["instagram_actor_id"] = resolved_instagram_actor_id

    creation_result = await make_api_request(f"{normalized_account_id}/adcreatives", meta_access_token, creative_payload, method="POST")

    if (resolved_instagram_user_id or resolved_instagram_actor_id) and isinstance(creation_result, dict) and creation_result.get("error"):
        details = creation_result.get("error", {}).get("details", {})
        inner = details.get("error", {}) if isinstance(details, dict) else {}
        message_text = ""
        if isinstance(inner, dict):
            message_text = inner.get("message", "") or inner.get("primary_text", "")
        lowered = message_text.lower()
        if "valid instagram account id" in lowered or "instagram_actor_id" in lowered or "instagram_user_id" in lowered:
            return _json(
                {
                    "error": "Instagram account not authorized for advertising",
                    "explanation": "The Meta API rejected the Instagram identity field in object_story_spec.",
                    "fix": "Reconnect the Facebook account and retry with refreshed permissions.",
                    "instagram_user_id": resolved_instagram_user_id,
                    "instagram_actor_id": resolved_instagram_actor_id,
                    "meta_error": message_text,
                }
            )

    if isinstance(creation_result, dict) and creation_result.get("id"):
        ad_creative_id = creation_result["id"]
        details = await make_api_request(ad_creative_id, meta_access_token, {"fields": _CREATIVE_FIELDS.replace(",image_urls_for_viewing", "")})
        return _json({"success": True, "ad_creative_id": ad_creative_id, "details": details})

    return _json(creation_result if isinstance(creation_result, dict) else {"data": creation_result})


@mcp_server.tool()
@meta_api_tool
async def update_ad_creative(
    ad_creative_id: str,
    meta_access_token: Optional[str] = None,
    name: Optional[str] = None,
    primary_text: Optional[str] = None,
    primary_text_variants: Optional[List[str]] = None,
    headline_text: Optional[str] = None,
    headline_variants: Optional[List[str]] = None,
    description_text: Optional[str] = None,
    description_variants: Optional[List[str]] = None,
    optimization_type: Optional[str] = None,
    dynamic_creative_spec: Optional[Dict[str, Any]] = None,
    call_to_action_type: Optional[str] = None,
    lead_form_id: Optional[str] = None,
    ad_formats: Optional[List[str]] = None,
) -> str:
    if not ad_creative_id:
        return _json({"error": "No creative ID provided"})

    primary_text_variants = _normalize_list_argument(primary_text_variants)
    headline_variants = _normalize_list_argument(headline_variants)
    description_variants = _normalize_list_argument(description_variants)
    ad_formats = _normalize_list_argument(ad_formats)

    attempted_content_fields = [
        field
        for field, value in {
            "primary_text": primary_text,
            "primary_text_variants": primary_text_variants,
            "headline_text": headline_text,
            "headline_variants": headline_variants,
            "description_text": description_text,
            "description_variants": description_variants,
            "call_to_action_type": call_to_action_type,
            "lead_form_id": lead_form_id,
        }.items()
        if value is not None
    ]

    if attempted_content_fields:
        return _json(
            {
                "error": "Content updates are not allowed on existing creatives",
                "explanation": (
                    "The Meta API does not allow updating content fields (primary_text, headline_text, description_text, CTA, image, video, URL) "
                    "on existing creatives."
                ),
                "workaround": (
                    "Create a new creative via create_ad_creative, then call update_ad with the new ad_creative_id."
                ),
                "ad_creative_id": ad_creative_id,
                "attempted_content_fields": attempted_content_fields,
            }
        )

    if optimization_type and optimization_type != "DEGREES_OF_FREEDOM":
        return _json({"error": f"Invalid optimization_type '{optimization_type}'. Only 'DEGREES_OF_FREEDOM' is supported."})

    update_payload: Dict[str, Any] = {}
    if name:
        update_payload["name"] = name

    if optimization_type or dynamic_creative_spec or ad_formats:
        resolved_update_formats = list(ad_formats) if ad_formats else (
            ["AUTOMATIC_FORMAT"] if optimization_type == "DEGREES_OF_FREEDOM" else ["SINGLE_IMAGE"]
        )
        feed: Dict[str, Any] = {
            "ad_formats": resolved_update_formats,
        }
        if optimization_type:
            feed["optimization_type"] = optimization_type
        update_payload["asset_feed_spec"] = feed

    if dynamic_creative_spec:
        update_payload["dynamic_creative_spec"] = dynamic_creative_spec

    if not update_payload:
        return _json({"error": "No update parameters provided"})

    try:
        result = await make_api_request(ad_creative_id, meta_access_token, update_payload, method="POST")
    except Exception as exc:  # noqa: BLE001
        return _json(
            {
                "error": "Failed to update ad creative",
                "details": str(exc),
                "update_data_sent": update_payload,
            }
        )

    if isinstance(result, dict) and result.get("id"):
        details = await make_api_request(
            ad_creative_id,
            meta_access_token,
            {
                "fields": (
                    "id,name,status,thumbnail_url,image_url,image_hash,object_story_spec,"
                    "url_tags,link_url,dynamic_creative_spec"
                )
            },
        )
        return _json({"success": True, "ad_creative_id": ad_creative_id, "details": details})

    error_obj = result.get("error", {}) if isinstance(result, dict) else {}
    details = error_obj.get("details", {}) if isinstance(error_obj, dict) else {}
    inner = details.get("error", {}) if isinstance(details, dict) else {}
    error_subcode = inner.get("error_subcode") if isinstance(inner, dict) else error_obj.get("error_subcode")

    if error_subcode == 1815573:
        return _json(
            {
                "error": "Content updates are not allowed on existing creatives",
                "explanation": (
                    "The Meta API does not allow updating content fields (primary_text, headline_text, description_text, CTA, image, video, URL) "
                    "on existing creatives."
                ),
                "workaround": (
                    "Create a new creative via create_ad_creative, then call update_ad with the new ad_creative_id."
                ),
                "ad_creative_id": ad_creative_id,
                "attempted_updates": update_payload,
            }
        )

    return _json(result if isinstance(result, dict) else {"data": result})


async def _collect_account_page_candidates(ad_account_id: str, meta_access_token: str) -> Dict[str, Any]:
    normalized_account_id = _normalize_ad_account_id(ad_account_id)
    pages: List[Dict[str, Any]] = []
    failures: List[Dict[str, str]] = []
    seen_ids: set = set()

    def add_candidate(page: Dict[str, Any], source: str, confidence: str) -> None:
        if not isinstance(page, dict):
            return
        facebook_page_id = str(page.get("id", "")).strip()
        if not facebook_page_id or facebook_page_id in seen_ids:
            return
        candidate = dict(page)
        candidate["id"] = facebook_page_id
        candidate["source"] = source
        candidate["confidence"] = confidence
        pages.append(candidate)
        seen_ids.add(facebook_page_id)

    async def add_page_by_id(facebook_page_id: str, source: str, confidence: str) -> None:
        normalized = str(facebook_page_id).strip()
        if not normalized or normalized in seen_ids:
            return
        try:
            payload = await make_api_request(normalized, meta_access_token, {"fields": _PAGE_FIELDS})
            if isinstance(payload, dict) and payload.get("id"):
                add_candidate(payload, source, confidence)
            else:
                add_candidate({"id": normalized, "name": "Unknown", "error": "Page details not accessible"}, source, confidence)
        except Exception as exc:  # noqa: BLE001
            add_candidate({"id": normalized, "name": "Unknown", "error": f"Failed to get page details: {exc}"}, source, confidence)

    def add_failure(source: str, reason: str) -> None:
        failures.append({"source": source, "reason": reason})

    try:
        payload = await make_api_request("me/accounts", meta_access_token, {"fields": _PAGE_FIELDS, "page_size": 200})
        for row in payload.get("data", []) if isinstance(payload, dict) else []:
            add_candidate(row, "me/accounts", "primary_documented")
    except Exception as exc:  # noqa: BLE001
        add_failure("me/accounts", str(exc))

    try:
        account_payload = await make_api_request(normalized_account_id, meta_access_token, {"fields": "business{id,name}"})
        business = account_payload.get("business") if isinstance(account_payload, dict) else None
        business_id = None
        if isinstance(business, dict):
            business_id = business.get("id")
        elif isinstance(business, (str, int)):
            business_id = str(business)

        if business_id:
            owned_payload = await make_api_request(
                f"{business_id}/owned_pages",
                meta_access_token,
                {"fields": _PAGE_FIELDS, "page_size": 200},
            )
            for row in owned_payload.get("data", []) if isinstance(owned_payload, dict) else []:
                add_candidate(row, "business/owned_pages", "primary_documented")
        else:
            add_failure("business/owned_pages", "Ad account is not linked to a business object")
    except Exception as exc:  # noqa: BLE001
        add_failure("business/owned_pages", str(exc))

    for source, edge in (
        ("ad_account/client_pages", f"{normalized_account_id}/client_pages"),
        ("ad_account/assigned_pages", f"{normalized_account_id}/assigned_pages"),
    ):
        try:
            payload = await make_api_request(edge, meta_access_token, {"fields": _PAGE_FIELDS, "page_size": 200})
            for row in payload.get("data", []) if isinstance(payload, dict) else []:
                add_candidate(row, source, "secondary_fallback")
        except Exception as exc:  # noqa: BLE001
            add_failure(source, str(exc))

    try:
        ads_payload = await make_api_request(
            f"{normalized_account_id}/ads",
            meta_access_token,
            {"fields": "id,tracking_specs", "page_size": 100},
        )
        tracking_ids: set = set()
        for ad in ads_payload.get("data", []) if isinstance(ads_payload, dict) else []:
            specs = ad.get("tracking_specs", []) if isinstance(ad, dict) else []
            if not isinstance(specs, list):
                continue
            for spec in specs:
                page_values = spec.get("page") if isinstance(spec, dict) else None
                if isinstance(page_values, list):
                    for raw_id in page_values:
                        facebook_page_id = str(raw_id).strip()
                        if facebook_page_id.isdigit():
                            tracking_ids.add(facebook_page_id)
        for facebook_page_id in sorted(tracking_ids):
            await add_page_by_id(facebook_page_id, "ads/tracking_specs", "secondary_fallback")
    except Exception as exc:  # noqa: BLE001
        add_failure("ads/tracking_specs", str(exc))

    try:
        creative_payload = await make_api_request(
            f"{normalized_account_id}/adcreatives",
            meta_access_token,
            {"fields": "id,object_story_spec", "page_size": 100},
        )
        story_ids: set = set()
        for creative in creative_payload.get("data", []) if isinstance(creative_payload, dict) else []:
            if not isinstance(creative, dict):
                continue
            story = creative.get("object_story_spec")
            if isinstance(story, dict):
                facebook_page_id = str(story.get("page_id") or story.get("facebook_page_id", "")).strip()
                if facebook_page_id.isdigit():
                    story_ids.add(facebook_page_id)
        for facebook_page_id in sorted(story_ids):
            await add_page_by_id(facebook_page_id, "adcreatives/object_story_spec", "secondary_fallback")
    except Exception as exc:  # noqa: BLE001
        add_failure("adcreatives/object_story_spec", str(exc))

    source_counts = {
        "primary_documented": sum(1 for page in pages if page.get("confidence") == "primary_documented"),
        "secondary_fallback": sum(1 for page in pages if page.get("confidence") == "secondary_fallback"),
    }

    return {
        "pages": pages,
        "failures": failures,
        "source_counts": source_counts,
    }


async def _discover_pages_for_account(ad_account_id: str, meta_access_token: str) -> dict:
    try:
        discovery = await _collect_account_page_candidates(ad_account_id, meta_access_token)
        candidates = discovery.get("pages", [])
        if candidates:
            selected = next(
                (page for page in candidates if page.get("confidence") == "primary_documented"),
                candidates[0],
            )
            return {
                "success": True,
                "facebook_page_id": selected.get("id"),
                "page_name": selected.get("name", "Unknown"),
                "source": selected.get("source"),
                "confidence": selected.get("confidence"),
                "total_candidates": len(candidates),
                "note": (
                    "Selected from documented primary page edges."
                    if selected.get("confidence") == "primary_documented"
                    else "Primary documented edges returned no pages; using secondary fallback edge."
                ),
            }

        return {
            "success": False,
            "message": "No suitable pages found for this account",
            "failed_sources": discovery.get("failures", []),
            "note": "Use list_account_pages for full diagnostics or provide facebook_page_id manually.",
        }
    except Exception as exc:  # noqa: BLE001
        return {
            "success": False,
            "message": f"Error during page discovery: {exc}",
        }


async def _search_pages_core(meta_access_token: str, ad_account_id: str, query: str = None) -> str:
    normalized_account_id = _normalize_ad_account_id(ad_account_id)

    try:
        discovery = await _collect_account_page_candidates(normalized_account_id, meta_access_token)
        pages = discovery.get("pages", [])
        if not pages:
            return _json(
                {
                    "data": [],
                    "message": "No pages found for this account",
                    "failed_sources": discovery.get("failures", []),
                }
            )

        if query:
            token = str(query).strip().lower()
            filtered = [page for page in pages if token in str(page.get("name", "")).lower()]
            return _json(
                {
                    "data": filtered,
                    "query": query,
                    "total_found": len(filtered),
                    "total_available": len(pages),
                }
            )

        return _json(
            {
                "data": pages,
                "total_available": len(pages),
                "source_counts": discovery.get("source_counts", {}),
                "note": "Use query to filter pages by name.",
            }
        )
    except Exception as exc:  # noqa: BLE001
        return _json({"error": "Failed to search pages by name", "details": str(exc)})


@mcp_server.tool()
@meta_api_tool
async def search_pages(
    ad_account_id: str,
    meta_access_token: Optional[str] = None,
    query: Optional[str] = None,
) -> str:
    if not ad_account_id:
        return _json({"error": "No account ID provided"})
    return await _search_pages_core(meta_access_token, ad_account_id, query)


@mcp_server.tool()
@meta_api_tool
async def list_account_pages(ad_account_id: str, meta_access_token: Optional[str] = None) -> str:
    if not ad_account_id:
        return _json({"error": "No account ID provided"})

    if ad_account_id == "me":
        try:
            payload = await make_api_request("me/accounts", meta_access_token, {"fields": _PAGE_FIELDS, "page_size": 200})
            if isinstance(payload, dict) and isinstance(payload.get("data"), list):
                for page in payload["data"]:
                    if isinstance(page, dict):
                        page["source"] = "me/accounts"
                        page["confidence"] = "primary_documented"
            return _json(payload)
        except Exception as exc:  # noqa: BLE001
            return _json({"error": "Failed to get user pages", "details": str(exc)})

    normalized_account_id = _normalize_ad_account_id(ad_account_id)

    try:
        discovery = await _collect_account_page_candidates(normalized_account_id, meta_access_token)
        pages = discovery.get("pages", [])
        if pages:
            return _json(
                {
                    "data": pages,
                    "total_pages_found": len(pages),
                    "source_counts": discovery.get("source_counts", {}),
                    "failed_sources": discovery.get("failures", []),
                }
            )

        return _json(
            {
                "data": [],
                "message": "No pages found associated with this account",
                "source_counts": discovery.get("source_counts", {}),
                "failed_sources": discovery.get("failures", []),
                "suggestion": (
                    "Connect a Facebook page to this ad account or provide facebook_page_id manually. "
                    "Primary documented edges were checked before fallbacks."
                ),
            }
        )
    except Exception as exc:  # noqa: BLE001
        return _json({"error": "Failed to get account pages", "details": str(exc)})