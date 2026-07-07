# Copyright (C) 2025 ArmaVita LLC
# SPDX-License-Identifier: AGPL-3.0-only

"""Product Catalog & Commerce — for Advantage+ Catalog Ads.

Surfaces:
  * `list_product_catalogs(business_id)`
  * `list_products(product_catalog_id)` — items in a catalog
  * `upsert_product(...)` — single-item create-or-update via `allow_upsert`
  * `batch_products(...)` — bulk via items_batch (30 MB / 5,000-item limit)
  * `list_product_sets(...)` — product sets (used as targets for catalog ads)
"""


import json
from typing import Any, Dict, List, Optional

from .graph_client import make_api_request, meta_api_tool
from .mcp_runtime import mcp_server
from mcp.types import ToolAnnotations


_CATALOG_FIELDS = (
    "id,name,vertical,product_count,business{id,name},da_display_settings,fallback_image_url"
)
_PRODUCT_FIELDS = (
    "id,retailer_id,name,description,price,currency,availability,condition,"
    "url,image_url,additional_image_urls,brand,category,videos,live_special_price,"
    "review_status"
)
_PRODUCT_SET_FIELDS = "id,name,filter,product_count,auto_creation_url"

# items_batch payloads are limited to 30 MB on the wire.
_ITEMS_BATCH_MAX_BYTES = 30 * 1024 * 1024


def _json(payload: Dict[str, Any]) -> str:
    return json.dumps(payload, indent=2)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=True))
@meta_api_tool
async def list_product_catalogs(
    business_id: str,
    meta_access_token: Optional[str] = None,
    page_size: int = 25,
    page_cursor: str = "",
) -> str:
    """List product catalogs owned by a business."""
    if not str(business_id or "").strip():
        return _json({"error": "business_id is required"})

    params: Dict[str, Any] = {"fields": _CATALOG_FIELDS, "page_size": int(page_size)}
    if page_cursor:
        params["page_cursor"] = page_cursor

    payload = await make_api_request(
        f"{str(business_id).strip()}/owned_product_catalogs",
        meta_access_token,
        params,
    )
    return _json(payload)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=True))
@meta_api_tool
async def list_products(
    product_catalog_id: str,
    meta_access_token: Optional[str] = None,
    page_size: int = 25,
    page_cursor: str = "",
) -> str:
    """List products in a catalog."""
    if not str(product_catalog_id or "").strip():
        return _json({"error": "product_catalog_id is required"})

    params: Dict[str, Any] = {"fields": _PRODUCT_FIELDS, "page_size": int(page_size)}
    if page_cursor:
        params["page_cursor"] = page_cursor

    payload = await make_api_request(
        f"{str(product_catalog_id).strip()}/products",
        meta_access_token,
        params,
    )
    return _json(payload)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=False, destructiveHint=False, idempotentHint=True, openWorldHint=True))
@meta_api_tool
async def upsert_product(
    product_catalog_id: str,
    product: Dict[str, Any],
    meta_access_token: Optional[str] = None,
    allow_upsert: bool = True,
) -> str:
    """Create or update a single product (v24.0+ allow_upsert).

    `product` must include `retailer_id`. Other recommended fields:
    `name`, `description`, `price`, `currency`, `availability`, `url`,
    `image_url`, `brand`, `category`. `live_special_price` is supported (v23.0+).
    """
    if not str(product_catalog_id or "").strip():
        return _json({"error": "product_catalog_id is required"})
    if not product or not product.get("retailer_id"):
        return _json({"error": "product.retailer_id is required"})

    body: Dict[str, Any] = {k: json.dumps(v) if isinstance(v, (dict, list)) else v for k, v in product.items()}
    body["allow_upsert"] = "true" if allow_upsert else "false"

    result = await make_api_request(
        f"{str(product_catalog_id).strip()}/products",
        meta_access_token,
        body,
        method="POST",
    )
    return _json(result)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=False, destructiveHint=False, idempotentHint=False, openWorldHint=True))
@meta_api_tool
async def batch_products(
    product_catalog_id: str,
    requests: List[Dict[str, Any]],
    meta_access_token: Optional[str] = None,
    allow_upsert: bool = True,
) -> str:
    """Bulk create/update/delete catalog items via items_batch (30 MB / 5,000 items).

    Each entry in `requests` is a Meta Items Batch request, e.g.:
    `{"method": "UPDATE", "retailer_id": "SKU123", "data": {"price": "1299 USD", ...}}`.
    """
    if not str(product_catalog_id or "").strip():
        return _json({"error": "product_catalog_id is required"})
    if not requests:
        return _json({"error": "requests list is required"})
    if len(requests) > 5000:
        return _json({"error": "items_batch supports up to 5000 items per request"})

    serialized = json.dumps(requests)
    if len(serialized.encode("utf-8")) > _ITEMS_BATCH_MAX_BYTES:
        return _json(
            {
                "error": "items_batch payload exceeds 30 MB (Meta limit)",
                "size_bytes": len(serialized.encode("utf-8")),
            }
        )

    body: Dict[str, Any] = {
        "requests": serialized,
        "allow_upsert": "true" if allow_upsert else "false",
    }

    result = await make_api_request(
        f"{str(product_catalog_id).strip()}/items_batch",
        meta_access_token,
        body,
        method="POST",
    )
    return _json(result)


@mcp_server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=True))
@meta_api_tool
async def list_product_sets(
    product_catalog_id: str,
    meta_access_token: Optional[str] = None,
    page_size: int = 25,
    page_cursor: str = "",
) -> str:
    """List product sets in a catalog.

    Product sets are the targeting unit for Advantage+ Catalog ads — set the
    chosen `product_set_id` on the ad creative's `product_set_id` field.
    """
    if not str(product_catalog_id or "").strip():
        return _json({"error": "product_catalog_id is required"})

    params: Dict[str, Any] = {"fields": _PRODUCT_SET_FIELDS, "page_size": int(page_size)}
    if page_cursor:
        params["page_cursor"] = page_cursor

    payload = await make_api_request(
        f"{str(product_catalog_id).strip()}/product_sets",
        meta_access_token,
        params,
    )
    return _json(payload)
