// SPDX-License-Identifier: AGPL-3.0-only

//! `inspect_access_token`: what the configured token can do, without revealing it.

use std::time::{SystemTime, UNIX_EPOCH};

use rmcp::{
    Json,
    handler::server::wrapper::Parameters,
    schemars::{self, JsonSchema},
    tool, tool_router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    server::MetaAdsServer,
};

/// Permissions the typed tools rely on most. Missing ones are reported, not enforced.
const RECOMMENDED_SCOPES: &[&str] = &[
    "ads_management",
    "ads_read",
    "business_management",
    "pages_show_list",
    "pages_read_engagement",
    "pages_manage_ads",
    "leads_retrieval",
    "catalog_management",
];
const MAX_SCOPES: usize = 100;
const MAX_TARGET_IDS: usize = 200;
const MAX_TEXT_CHARS: usize = 128;
const SECONDS_PER_DAY: i64 = 86_400;

/// No input: the tool inspects the token configured for this server.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectAccessTokenInput {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GranularScope {
    pub scope: String,
    /// Assets the permission is limited to (ad accounts, Pages, businesses).
    /// Empty means the permission applies to every asset the user can reach.
    pub target_ids: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AccessTokenInfo {
    pub is_valid: bool,
    /// USER, SYSTEM_USER, PAGE or APP.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub application: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// Unix time; absent or 0 means the token does not expire.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    pub never_expires: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in_days: Option<i64>,
    /// Unix time after which Meta stops returning user data without re-consent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_access_expires_at: Option<i64>,
    pub scopes: Vec<String>,
    pub granular_scopes: Vec<GranularScope>,
    /// Recommended permissions this token lacks.
    pub missing_recommended_scopes: Vec<String>,
    /// Whether this server signs requests with `appsecret_proof`.
    pub appsecret_proof: bool,
}

fn text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.chars().take(MAX_TEXT_CHARS).collect()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn time(value: Option<&Value>) -> Option<i64> {
    value?.as_i64()
}

fn parse_debug_token(
    payload: &Value,
    now: i64,
    signed: bool,
) -> Result<AccessTokenInfo, PublicError> {
    let data = payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            PublicError::invalid_upstream("Meta returned an unexpected debug_token response")
        })?;
    let scopes = data
        .get("scopes")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| text(Some(item)))
                .take(MAX_SCOPES)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let granular_scopes = data
        .get("granular_scopes")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(GranularScope {
                        scope: text(item.get("scope"))?,
                        target_ids: item
                            .get("target_ids")
                            .and_then(Value::as_array)
                            .map(|ids| {
                                ids.iter()
                                    .filter_map(|id| text(Some(id)))
                                    .take(MAX_TARGET_IDS)
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                })
                .take(MAX_SCOPES)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let expires_at = time(data.get("expires_at"));
    let never_expires = expires_at.is_none_or(|at| at == 0);
    let missing_recommended_scopes = RECOMMENDED_SCOPES
        .iter()
        .filter(|scope| !scopes.iter().any(|granted| granted == *scope))
        .map(|scope| (*scope).to_owned())
        .collect();
    Ok(AccessTokenInfo {
        is_valid: data
            .get("is_valid")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        token_type: text(data.get("type")),
        app_id: text(data.get("app_id")),
        application: text(data.get("application")),
        user_id: text(data.get("user_id")),
        expires_at,
        never_expires,
        expires_in_days: expires_at
            .filter(|at| *at > 0)
            .map(|at| (at - now).div_euclid(SECONDS_PER_DAY)),
        data_access_expires_at: time(data.get("data_access_expires_at")).filter(|at| *at > 0),
        scopes,
        granular_scopes,
        missing_recommended_scopes,
        appsecret_proof: signed,
    })
}

pub(crate) async fn inspect_access_token(graph: &GraphClient) -> ToolResponse<AccessTokenInfo> {
    let payload = match graph.debug_current_token().await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        });
    match parse_debug_token(&payload, now, graph.signs_appsecret_proof()) {
        Ok(info) => ToolResponse::success(info),
        Err(error) => ToolResponse::error(error),
    }
}

#[tool_router(router = access_token_router, vis = "pub(crate)")]
impl MetaAdsServer {
    #[tool(
        name = "inspect_access_token",
        description = "Inspect the configured Meta token with debug_token: validity, type, app, expiry, granted scopes (with the assets each is limited to) and missing recommended scopes. Never returns the token.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn inspect_access_token(
        &self,
        Parameters(_input): Parameters<InspectAccessTokenInput>,
    ) -> Result<Json<ToolResponse<AccessTokenInfo>>, Json<ToolResponse<AccessTokenInfo>>> {
        inspect_access_token(&self.graph).await.into_mcp_result()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::parse_debug_token;

    #[test]
    fn summarizes_a_system_user_token_and_lists_missing_scopes() {
        let payload = json!({"data": {
            "app_id": "123",
            "type": "SYSTEM_USER",
            "application": "Agency app",
            "expires_at": 0,
            "data_access_expires_at": 0,
            "is_valid": true,
            "issued_at": 1_700_000_000,
            "scopes": ["ads_management", "ads_read", "business_management", "pages_show_list"],
            "granular_scopes": [
                {"scope": "ads_management", "target_ids": ["act_1", "act_2"]},
                {"scope": "pages_show_list"}
            ],
            "user_id": "999"
        }});
        let info = parse_debug_token(&payload, 1_800_000_000, true).unwrap();
        assert!(info.is_valid);
        assert!(info.never_expires);
        assert_eq!(info.expires_in_days, None);
        assert_eq!(info.data_access_expires_at, None);
        assert_eq!(info.token_type.as_deref(), Some("SYSTEM_USER"));
        assert_eq!(info.granular_scopes[0].target_ids, ["act_1", "act_2"]);
        assert!(info.granular_scopes[1].target_ids.is_empty());
        assert_eq!(
            info.missing_recommended_scopes,
            [
                "pages_read_engagement",
                "pages_manage_ads",
                "leads_retrieval",
                "catalog_management"
            ]
        );
        assert!(info.appsecret_proof);
    }

    #[test]
    fn reports_days_until_expiry_and_invalid_tokens() {
        let payload =
            json!({"data": {"is_valid": false, "expires_at": 1_800_000_000 + 10 * 86_400 + 5}});
        let info = parse_debug_token(&payload, 1_800_000_000, false).unwrap();
        assert!(!info.is_valid);
        assert!(!info.never_expires);
        assert_eq!(info.expires_in_days, Some(10));
        assert!(parse_debug_token(&json!({"error": {}}), 0, false).is_err());
    }
}
