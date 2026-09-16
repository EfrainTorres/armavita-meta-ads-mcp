// Copyright (C) 2025 ArmaVita LLC
// SPDX-License-Identifier: AGPL-3.0-only

use serde_json::Value;

use crate::{
    error::PublicError,
    graph::GraphClient,
    meta_ids::{ad_account, numeric, numeric_value},
};

const MAX_PROVIDER_NAME_CHARS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetaNodeKind {
    Campaign,
    AdSet,
    Ad,
    AdCreative,
    CustomAudience,
    CustomConversion,
}

impl MetaNodeKind {
    fn label(self) -> &'static str {
        match self {
            Self::Campaign => "campaign",
            Self::AdSet => "ad set",
            Self::Ad => "ad",
            Self::AdCreative => "ad creative",
            Self::CustomAudience => "custom audience",
            Self::CustomConversion => "custom conversion",
        }
    }

    fn query_fields(self) -> &'static str {
        match self {
            Self::Campaign => "id,name,account_id,objective",
            Self::AdSet => "id,name,account_id,optimization_goal",
            Self::Ad => "id,name,account_id,adset_id",
            Self::AdCreative => "id,name,account_id,object_story_spec,object_type",
            Self::CustomAudience => "id,name,account_id,subtype",
            Self::CustomConversion => "id,name,account_id,custom_event_type",
        }
    }

    fn has_proof(self, payload: &Value) -> bool {
        let present = |field: &str| payload.get(field).is_some_and(|value| !value.is_null());
        match self {
            Self::Campaign => present("objective"),
            Self::AdSet => present("optimization_goal"),
            Self::Ad => present("adset_id"),
            Self::AdCreative => present("object_story_spec") || present("object_type"),
            Self::CustomAudience => present("subtype"),
            Self::CustomConversion => present("custom_event_type"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedMetaNode {
    pub(crate) object_id: String,
    pub(crate) account_id: String,
    pub(crate) name: Option<String>,
}

/// Prove that a raw Graph node ID is the requested ads resource before writing it.
pub(crate) async fn verify_meta_node(
    graph: &GraphClient,
    object_id: &str,
    kind: MetaNodeKind,
    expected_account_id: Option<&str>,
) -> Result<VerifiedMetaNode, PublicError> {
    let object_id = numeric(object_id).ok_or_else(|| {
        PublicError::invalid_input(
            "object_id must be a numeric Meta ID",
            "Use the numeric ID returned by Meta",
        )
    })?;
    let expected_account_id = expected_account_id
        .map(|value| {
            ad_account(value).ok_or_else(|| {
                PublicError::invalid_input(
                    "ad_account_id must be a numeric Meta account ID",
                    "Use an ID such as `act_123456789` or `123456789`",
                )
            })
        })
        .transpose()?;
    let payload = graph
        .get_json(
            object_id,
            &[("fields".to_owned(), kind.query_fields().to_owned())],
        )
        .await
        .map_err(PublicError::from)?;

    verify_payload(object_id, kind, expected_account_id.as_deref(), &payload)
}

fn verify_payload(
    object_id: &str,
    kind: MetaNodeKind,
    expected_account_id: Option<&str>,
    payload: &Value,
) -> Result<VerifiedMetaNode, PublicError> {
    let returned_id = payload.get("id").and_then(numeric_value).ok_or_else(|| {
        PublicError::invalid_upstream("Meta did not return a valid node identity")
    })?;
    if returned_id != object_id {
        return Err(PublicError::invalid_upstream(
            "Meta returned a different node identity",
        ));
    }
    if !kind.has_proof(payload) {
        return Err(identity_mismatch(kind));
    }

    let account_id = payload
        .get("account_id")
        .and_then(normalize_account_value)
        .ok_or_else(|| PublicError::invalid_upstream("Meta did not return a valid account ID"))?;
    if expected_account_id.is_some_and(|expected| expected != account_id) {
        return Err(PublicError::invalid_input(
            format!("the {} does not belong to ad_account_id", kind.label()),
            "Use the resource ID from the specified Meta ad account",
        ));
    }

    Ok(VerifiedMetaNode {
        object_id: returned_id,
        account_id,
        name: payload.get("name").and_then(bounded_provider_name),
    })
}

fn normalize_account_value(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => ad_account(value),
        _ => numeric_value(value).and_then(|value| ad_account(&value)),
    }
}

fn bounded_provider_name(value: &Value) -> Option<String> {
    let name = value.as_str()?.trim();
    (!name.is_empty()
        && name.chars().count() <= MAX_PROVIDER_NAME_CHARS
        && !name.chars().any(char::is_control))
    .then(|| name.to_owned())
}

fn identity_mismatch(kind: MetaNodeKind) -> PublicError {
    PublicError::invalid_input(
        format!("the supplied ID is not a Meta {}", kind.label()),
        format!("Use a {} ID returned by Meta", kind.label()),
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::{Value, json};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    use crate::{
        ad_mutations::{
            AdCopyStatus, CloneAdInput, UpdateAdCreativeInput, UpdateAdInput, clone_ad,
        },
        adset_mutations::{AdSetCopyStatus, CloneAdSetInput, UpdateAdSetInput, clone_ad_set},
        campaign_mutations::{
            CampaignBudgetScheduleValue, CloneCampaignInput, CreateCampaignBudgetScheduleInput,
            UpdateCampaignInput, clone_campaign, create_campaign_budget_schedule, update_campaign,
        },
        config::MetaConfig,
        error::ToolResponse,
        graph::GraphClient,
    };

    use super::{MetaNodeKind, verify_payload};

    #[test]
    fn requests_one_minimal_type_proof_per_resource() {
        for (kind, fields) in [
            (MetaNodeKind::Campaign, "id,name,account_id,objective"),
            (MetaNodeKind::AdSet, "id,name,account_id,optimization_goal"),
            (MetaNodeKind::Ad, "id,name,account_id,adset_id"),
            (
                MetaNodeKind::AdCreative,
                "id,name,account_id,object_story_spec,object_type",
            ),
            (MetaNodeKind::CustomAudience, "id,name,account_id,subtype"),
            (
                MetaNodeKind::CustomConversion,
                "id,name,account_id,custom_event_type",
            ),
        ] {
            assert_eq!(kind.query_fields(), fields);
        }
    }

    #[test]
    fn existing_node_inputs_require_one_bounded_expected_account() {
        let schemas: [(&str, Value); 8] = [
            ("update campaign", schema::<UpdateCampaignInput>()),
            ("clone campaign", schema::<CloneCampaignInput>()),
            (
                "campaign budget schedule",
                schema::<CreateCampaignBudgetScheduleInput>(),
            ),
            ("update ad set", schema::<UpdateAdSetInput>()),
            ("clone ad set", schema::<CloneAdSetInput>()),
            ("update ad", schema::<UpdateAdInput>()),
            ("clone ad", schema::<CloneAdInput>()),
            ("update creative", schema::<UpdateAdCreativeInput>()),
        ];
        for (label, schema) in schemas {
            assert_eq!(
                schema.pointer("/properties/ad_account_id/pattern"),
                Some(&json!("^(act_)?[0-9]{1,64}$")),
                "{label} account pattern",
            );
            assert_eq!(
                schema.pointer("/properties/ad_account_id/maxLength"),
                Some(&json!(68)),
                "{label} account bound",
            );
            assert!(
                schema["required"]
                    .as_array()
                    .is_some_and(|required| required.contains(&json!("ad_account_id"))),
                "{label} must require ad_account_id",
            );
        }
    }

    #[test]
    fn rejects_wrong_type_id_and_account_without_returning_payload() {
        for (payload, expected_account_id, expected_code) in [
            (
                json!({
                    "id": "42",
                    "name": "Wrong node",
                    "account_id": "123",
                    "optimization_goal": "LINK_CLICKS"
                }),
                Some("act_123"),
                "INVALID_INPUT",
            ),
            (
                json!({
                    "id": "99",
                    "name": "Different node",
                    "account_id": "123",
                    "objective": "OUTCOME_SALES"
                }),
                Some("act_123"),
                "INVALID_UPSTREAM_RESPONSE",
            ),
            (
                json!({
                    "id": "42",
                    "name": "Other account",
                    "account_id": "456",
                    "objective": "OUTCOME_SALES"
                }),
                Some("act_123"),
                "INVALID_INPUT",
            ),
        ] {
            let error = verify_payload("42", MetaNodeKind::Campaign, expected_account_id, &payload)
                .unwrap_err();
            assert_eq!(error.code, expected_code);
            let rendered = format!("{error:?}");
            assert!(!rendered.contains("Wrong node"));
            assert!(!rendered.contains("Different node"));
            assert!(!rendered.contains("Other account"));
        }
    }

    #[test]
    fn accepts_either_creative_proof_and_bounds_provider_name() {
        for proof in [
            json!({"object_type": "SHARE"}),
            json!({"object_story_spec": {"page_id": "7"}}),
        ] {
            let mut payload = json!({
                "id": "42",
                "account_id": 123,
                "name": " Creative "
            });
            payload
                .as_object_mut()
                .unwrap()
                .extend(proof.as_object().unwrap().clone());
            let verified = verify_payload("42", MetaNodeKind::AdCreative, None, &payload).unwrap();
            assert_eq!(verified.object_id, "42");
            assert_eq!(verified.account_id, "act_123");
            assert_eq!(verified.name.as_deref(), Some("Creative"));
        }

        let payload = json!({
            "id": "42",
            "account_id": "123",
            "objective": "OUTCOME_SALES",
            "name": "x".repeat(super::MAX_PROVIDER_NAME_CHARS + 1)
        });
        assert!(
            verify_payload("42", MetaNodeKind::Campaign, None, &payload)
                .unwrap()
                .name
                .is_none()
        );
    }

    #[tokio::test]
    async fn direct_update_preflights_exact_node_before_posting() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut preflight, _) = listener.accept().await.unwrap();
            let preflight_request = read_http_request(&mut preflight).await;
            write_json(
                &mut preflight,
                r#"{"id":"42","name":"Campaign","account_id":"123","objective":"OUTCOME_SALES"}"#,
            )
            .await;

            let (mut mutation, _) = listener.accept().await.unwrap();
            let mutation_request = read_http_request(&mut mutation).await;
            write_json(&mut mutation, r#"{"success":true}"#).await;

            assert!(
                tokio::time::timeout(Duration::from_millis(200), listener.accept())
                    .await
                    .is_err(),
                "update emitted an unexpected third request"
            );
            (preflight_request, mutation_request)
        });
        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        ))
        .unwrap();

        let response = update_campaign(&graph, campaign_update_input()).await;
        assert!(matches!(response, ToolResponse::Success { .. }));

        let (preflight, mutation) = server.await.unwrap();
        let preflight = String::from_utf8(preflight).unwrap();
        let mutation = String::from_utf8(mutation).unwrap();
        assert!(
            preflight.starts_with("GET /42?fields=id%2Cname%2Caccount_id%2Cobjective HTTP/1.1\r\n")
        );
        assert!(mutation.starts_with("POST /42 HTTP/1.1\r\n"));
        assert!(mutation.ends_with("name=Renamed"));
    }

    #[tokio::test]
    async fn wrong_type_or_account_preflight_never_reaches_the_write() {
        for (label, identity) in [
            (
                "wrong node type",
                r#"{"id":"42","name":"Ad set","account_id":"123","optimization_goal":"LINK_CLICKS"}"#,
            ),
            (
                "wrong ad account",
                r#"{"id":"42","name":"Campaign","account_id":"456","objective":"OUTCOME_SALES"}"#,
            ),
        ] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut preflight, _) = listener.accept().await.unwrap();
                let request = read_http_request(&mut preflight).await;
                write_json(&mut preflight, identity).await;
                let write_attempted =
                    tokio::time::timeout(Duration::from_millis(300), listener.accept())
                        .await
                        .is_ok();
                (request, write_attempted)
            });
            let graph = test_graph(address);

            let ToolResponse::Error { error } =
                update_campaign(&graph, campaign_update_input()).await
            else {
                panic!("{label} must fail closed");
            };
            assert_eq!(error.code, "INVALID_INPUT");

            let (request, write_attempted) = server.await.unwrap();
            assert!(!write_attempted, "{label} reached the write");
            assert!(
                String::from_utf8(request)
                    .unwrap()
                    .starts_with("GET /42?fields=id%2Cname%2Caccount_id%2Cobjective HTTP/1.1\r\n")
            );
        }
    }

    #[tokio::test]
    async fn raw_source_edge_writes_reject_wrong_kind_before_posting() {
        for case in RawSourceEdgeCase::ALL {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut preflight, _) = listener.accept().await.unwrap();
                let request = read_http_request(&mut preflight).await;
                write_json(
                    &mut preflight,
                    r#"{"id":"42","account_id":"123","subtype":"CUSTOM"}"#,
                )
                .await;
                let write_attempted =
                    tokio::time::timeout(Duration::from_millis(200), listener.accept())
                        .await
                        .is_ok();
                (request, write_attempted)
            });
            let graph = test_graph(address);

            assert!(
                !case.invoke(&graph).await,
                "{} must fail closed",
                case.label()
            );

            let (request, write_attempted) = server.await.unwrap();
            assert!(!write_attempted, "{} reached its write", case.label());
            assert!(
                String::from_utf8(request)
                    .unwrap()
                    .starts_with(case.preflight_prefix()),
                "{} used the wrong preflight",
                case.label()
            );
        }
    }

    #[tokio::test]
    async fn raw_source_edge_writes_preflight_before_one_exact_post() {
        for case in RawSourceEdgeCase::ALL {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut preflight, _) = listener.accept().await.unwrap();
                let preflight_request = read_http_request(&mut preflight).await;
                write_json(&mut preflight, case.identity_response()).await;

                let (mut mutation, _) = listener.accept().await.unwrap();
                let mutation_request = read_http_request(&mut mutation).await;
                write_json(&mut mutation, case.mutation_response()).await;

                assert!(
                    tokio::time::timeout(Duration::from_millis(200), listener.accept())
                        .await
                        .is_err(),
                    "{} emitted an unexpected third request",
                    case.label()
                );
                (preflight_request, mutation_request)
            });
            let graph = test_graph(address);

            assert!(case.invoke(&graph).await, "{} must succeed", case.label());

            let (preflight, mutation) = server.await.unwrap();
            assert!(
                String::from_utf8(preflight)
                    .unwrap()
                    .starts_with(case.preflight_prefix()),
                "{} used the wrong preflight",
                case.label()
            );
            assert!(
                String::from_utf8(mutation)
                    .unwrap()
                    .starts_with(case.mutation_prefix()),
                "{} used the wrong mutation edge",
                case.label()
            );
        }
    }

    #[derive(Clone, Copy)]
    enum RawSourceEdgeCase {
        CampaignClone,
        AdSetClone,
        AdClone,
        CampaignBudgetSchedule,
    }

    impl RawSourceEdgeCase {
        const ALL: [Self; 4] = [
            Self::CampaignClone,
            Self::AdSetClone,
            Self::AdClone,
            Self::CampaignBudgetSchedule,
        ];

        async fn invoke(self, graph: &GraphClient) -> bool {
            match self {
                Self::CampaignClone => matches!(
                    clone_campaign(
                        graph,
                        CloneCampaignInput {
                            ad_account_id: "act_123".to_owned(),
                            campaign_id: "42".to_owned(),
                            deep_copy: None,
                            status: None,
                            rename: None,
                        },
                    )
                    .await,
                    ToolResponse::Success { .. }
                ),
                Self::AdSetClone => matches!(
                    clone_ad_set(
                        graph,
                        CloneAdSetInput {
                            ad_account_id: "act_123".to_owned(),
                            ad_set_id: "42".to_owned(),
                            target_campaign_id: None,
                            deep_copy: None,
                            status: AdSetCopyStatus::default(),
                            start_time: None,
                            end_time: None,
                            rename: None,
                        },
                    )
                    .await,
                    ToolResponse::Success { .. }
                ),
                Self::AdClone => matches!(
                    clone_ad(
                        graph,
                        CloneAdInput {
                            ad_account_id: "act_123".to_owned(),
                            ad_id: "42".to_owned(),
                            target_ad_set_id: None,
                            status: AdCopyStatus::default(),
                            name_suffix: None,
                        },
                    )
                    .await,
                    ToolResponse::Success { .. }
                ),
                Self::CampaignBudgetSchedule => matches!(
                    create_campaign_budget_schedule(
                        graph,
                        CreateCampaignBudgetScheduleInput {
                            ad_account_id: "act_123".to_owned(),
                            campaign_id: "42".to_owned(),
                            budget: CampaignBudgetScheduleValue::Multiplier { value: 150 },
                            time_start: 2_000_000_000,
                            time_end: 2_000_003_600,
                        },
                    )
                    .await,
                    ToolResponse::Success { .. }
                ),
            }
        }

        const fn label(self) -> &'static str {
            match self {
                Self::CampaignClone => "campaign clone",
                Self::AdSetClone => "ad-set clone",
                Self::AdClone => "ad clone",
                Self::CampaignBudgetSchedule => "campaign budget schedule",
            }
        }

        const fn identity_response(self) -> &'static str {
            match self {
                Self::CampaignClone | Self::CampaignBudgetSchedule => {
                    r#"{"id":"42","account_id":"123","objective":"OUTCOME_SALES"}"#
                }
                Self::AdSetClone => {
                    r#"{"id":"42","account_id":"123","optimization_goal":"LINK_CLICKS"}"#
                }
                Self::AdClone => r#"{"id":"42","account_id":"123","adset_id":"7"}"#,
            }
        }

        const fn mutation_response(self) -> &'static str {
            match self {
                Self::CampaignClone => r#"{"copied_campaign_id":"101"}"#,
                Self::AdSetClone => r#"{"copied_adset_id":"102"}"#,
                Self::AdClone => r#"{"copied_ad_id":"103"}"#,
                Self::CampaignBudgetSchedule => r#"{"id":"104"}"#,
            }
        }

        const fn preflight_prefix(self) -> &'static str {
            match self {
                Self::CampaignClone | Self::CampaignBudgetSchedule => {
                    "GET /42?fields=id%2Cname%2Caccount_id%2Cobjective HTTP/1.1\r\n"
                }
                Self::AdSetClone => {
                    "GET /42?fields=id%2Cname%2Caccount_id%2Coptimization_goal HTTP/1.1\r\n"
                }
                Self::AdClone => "GET /42?fields=id%2Cname%2Caccount_id%2Cadset_id HTTP/1.1\r\n",
            }
        }

        const fn mutation_prefix(self) -> &'static str {
            match self {
                Self::CampaignClone | Self::AdSetClone | Self::AdClone => {
                    "POST /42/copies HTTP/1.1\r\n"
                }
                Self::CampaignBudgetSchedule => "POST /42/budget_schedules HTTP/1.1\r\n",
            }
        }
    }

    fn test_graph(address: std::net::SocketAddr) -> GraphClient {
        GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        ))
        .unwrap()
    }

    fn campaign_update_input() -> UpdateCampaignInput {
        UpdateCampaignInput {
            ad_account_id: "act_123".to_owned(),
            campaign_id: "42".to_owned(),
            name: Some("Renamed".to_owned()),
            status: None,
            objective: None,
            budget: None,
            is_adset_budget_sharing_enabled: None,
            special_ad_categories: None,
            special_ad_category_countries: None,
            bid_strategy: None,
            spend_cap: None,
        }
    }

    fn schema<T: rmcp::schemars::JsonSchema>() -> Value {
        serde_json::to_value(rmcp::schemars::schema_for!(T)).unwrap()
    }

    async fn read_http_request(socket: &mut TcpStream) -> Vec<u8> {
        let mut request = Vec::with_capacity(2 * 1024);
        let mut buffer = [0_u8; 1024];
        loop {
            let bytes_read = socket.read(&mut buffer).await.unwrap();
            if bytes_read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..bytes_read]);
            let Some(headers_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let body_start = headers_end + 4;
            let headers = String::from_utf8_lossy(&request[..headers_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or_default();
            if request.len() >= body_start + content_length {
                break;
            }
        }
        request
    }

    async fn write_json(socket: &mut TcpStream, body: &str) {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    }
}
