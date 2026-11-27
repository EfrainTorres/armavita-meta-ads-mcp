// SPDX-License-Identifier: AGPL-3.0-only

//! Engagement and website custom audiences built from typed rules.
//!
//! `create_engagement_audience` creates one rule-based audience. `ensure_audience_set`
//! brings an ad account to a declared set of such audiences: it previews first, skips
//! audiences that already exist (by a marker in the description, or by name), and only
//! creates what is missing, so a repeated run never duplicates. `audit_audiences` reads
//! the size and health of an account's audiences.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rmcp::{
    Json,
    handler::server::wrapper::Parameters,
    schemars::{self, JsonSchema},
    tool, tool_router,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{
        ad_account as normalize_ad_account_id, numeric_owned as normalize_numeric_id, numeric_value,
    },
    mutation_result::{ambiguous_mutation_result, mutation_error_without_blind_retry},
    server::MetaAdsServer,
};

const APPLY_ACKNOWLEDGEMENT: &str = "APPLY_LIVE_META_ADS_CHANGES";
const SECONDS_PER_DAY: u64 = 86_400;
/// Instagram and Facebook Page engagement audiences keep up to 730 days.
const MAX_ENGAGEMENT_RETENTION_DAYS: u16 = 730;
/// Website audiences keep up to 180 days, except the Purchase event (730).
const MAX_WEBSITE_RETENTION_DAYS: u16 = 180;
const MAX_PURCHASE_RETENTION_DAYS: u16 = 730;
const MAX_RULES: usize = 10;
const MAX_SET_ITEMS: usize = 200;
const MAX_EXTRA_FILTERS: usize = 10;
const MAX_NAME_CHARS: usize = 256;
const MAX_PREFIX_CHARS: usize = 60;
const MAX_DESCRIPTION_CHARS: usize = 900;
const MAX_URL_FRAGMENT_CHARS: usize = 500;
const MAX_EVENT_NAME_CHARS: usize = 50;
const MAX_KEY_CHARS: usize = 64;
const MAX_SET_NAME_CHARS: usize = 40;
const MAX_LIST_PAGES: usize = 15;
const MAX_STATS_PAGES: usize = 5;
const PIXEL_STATS_DAYS: u64 = 30;
/// Meta's documented per-account custom-audience ceiling, used only for a warning.
const ACCOUNT_AUDIENCE_SOFT_LIMIT: usize = 500;
const DELIVERY_READY: i64 = 200;
const DELIVERY_TOO_SMALL: i64 = 300;
const OPERATION_NORMAL: i64 = 200;
const OPERATION_POPULATING: i64 = 441;
const SMALL_FOR_SEED: i64 = 100;
const MARKER_PREFIX: &str = "armavita:set=";
const PLAN_TOKEN_CHARS: usize = 32;
const CREATE_SPACING: Duration = Duration::from_millis(300);
const LIST_FIELDS: &str = "id,name,description";
const AUDIT_FIELDS: &str = "id,name,description,subtype,approximate_count_lower_bound,approximate_count_upper_bound,delivery_status,operation_status,rule";
const DEFAULT_MATRIX_DAYS: [u16; 9] = [3, 7, 14, 30, 60, 90, 180, 365, 730];

// ---------------------------------------------------------------------------
// Typed rules
// ---------------------------------------------------------------------------

/// Instagram professional-account engagement.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstagramEngagement {
    /// Anyone who engaged with the account.
    All,
    /// Visited the profile.
    ProfileVisit,
    /// Engaged with a post or ad.
    Engaged,
    /// Sent a message.
    Messaged,
    /// Saved a post or ad.
    Saved,
}

impl InstagramEngagement {
    const fn event(self) -> &'static str {
        match self {
            Self::All => "ig_business_profile_all",
            Self::ProfileVisit => "ig_business_profile_visit",
            Self::Engaged => "ig_business_profile_engaged",
            Self::Messaged => "ig_business_profile_user_messaged",
            Self::Saved => "ig_business_profile_ad_saved",
        }
    }
}

/// Facebook Page engagement.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PageEngagement {
    /// Anyone who engaged with the Page.
    Engaged,
    /// Visited the Page.
    Visited,
    /// Saved the Page or a post.
    Saved,
    /// Sent a message to the Page.
    Messaged,
    /// Engaged with a post or ad.
    PostInteraction,
}

impl PageEngagement {
    const fn event(self) -> &'static str {
        match self {
            Self::Engaged => "page_engaged",
            Self::Visited => "page_visited",
            Self::Saved => "page_or_post_save",
            Self::Messaged => "page_messaged",
            Self::PostInteraction => "page_post_interaction",
        }
    }
}

/// Website (pixel) audience condition.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebsiteEvent {
    /// Every website visitor.
    AllVisitors,
    /// Visitors of URLs containing `url_contains` (case-insensitive).
    UrlContains,
    /// The top `top_percent` (5, 10 or 25) of visitors by time on site.
    TopTimeSpent,
    PageView,
    ViewContent,
    AddToCart,
    InitiateCheckout,
    AddPaymentInfo,
    Purchase,
    Lead,
    CompleteRegistration,
    Contact,
    Schedule,
    Subscribe,
    /// A custom pixel event named by `event_name`.
    CustomEvent,
}

impl WebsiteEvent {
    const fn standard_name(self) -> Option<&'static str> {
        match self {
            Self::PageView => Some("PageView"),
            Self::ViewContent => Some("ViewContent"),
            Self::AddToCart => Some("AddToCart"),
            Self::InitiateCheckout => Some("InitiateCheckout"),
            Self::AddPaymentInfo => Some("AddPaymentInfo"),
            Self::Purchase => Some("Purchase"),
            Self::Lead => Some("Lead"),
            Self::CompleteRegistration => Some("CompleteRegistration"),
            Self::Contact => Some("Contact"),
            Self::Schedule => Some("Schedule"),
            Self::Subscribe => Some("Subscribe"),
            Self::AllVisitors | Self::UrlContains | Self::TopTimeSpent | Self::CustomEvent => None,
        }
    }

    const fn max_retention_days(self) -> u16 {
        match self {
            Self::Purchase => MAX_PURCHASE_RETENTION_DAYS,
            _ => MAX_WEBSITE_RETENTION_DAYS,
        }
    }
}

/// One audience rule. Several inclusion rules are combined with OR.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum AudienceRule {
    /// Instagram professional-account engagement, up to 730 days.
    Instagram {
        /// Numeric Instagram professional-account ID.
        instagram_account_id: String,
        event: InstagramEngagement,
        /// 1 through 730 days.
        retention_days: u16,
    },
    /// Facebook Page engagement, up to 730 days.
    FacebookPage {
        /// Numeric Facebook Page ID.
        page_id: String,
        event: PageEngagement,
        /// 1 through 730 days.
        retention_days: u16,
    },
    /// Website activity from a Meta pixel: up to 180 days, Purchase up to 730.
    Website {
        /// Numeric Meta pixel (dataset) ID.
        pixel_id: String,
        event: WebsiteEvent,
        /// 1 through 180 days (730 for `purchase`).
        retention_days: u16,
        /// Required for `url_contains`: URL fragment, matched case-insensitively.
        #[serde(default)]
        url_contains: Option<String>,
        /// Required for `top_time_spent`: 5, 10 or 25.
        #[serde(default)]
        top_percent: Option<u8>,
        /// Required for `custom_event`: the custom pixel event name.
        #[serde(default)]
        event_name: Option<String>,
    },
}

impl AudienceRule {
    fn pixel_event_name(&self) -> Option<String> {
        match self {
            Self::Website {
                event, event_name, ..
            } => event
                .standard_name()
                .map(str::to_owned)
                .or_else(|| (*event == WebsiteEvent::CustomEvent).then(|| event_name.clone())?),
            _ => None,
        }
    }
}

fn invalid(message: impl Into<String>, action: impl Into<String>) -> PublicError {
    PublicError::invalid_input(message, action)
}

fn numeric_id(raw: &str, field: &str) -> Result<String, PublicError> {
    normalize_numeric_id(raw).ok_or_else(|| {
        invalid(
            format!("{field} must be a numeric Meta ID"),
            format!("Use the numeric {field}"),
        )
    })
}

fn retention_seconds(days: u16, max: u16, what: &str) -> Result<u64, PublicError> {
    if days == 0 || days > max {
        return Err(invalid(
            format!("{what} audiences keep 1 through {max} days"),
            format!("Use retention_days between 1 and {max}"),
        ));
    }
    Ok(u64::from(days) * SECONDS_PER_DAY)
}

fn bounded_plain(raw: &str, field: &str, max: usize) -> Result<String, PublicError> {
    let value = raw.trim();
    if value.is_empty() || value.chars().count() > max || value.chars().any(char::is_control) {
        return Err(invalid(
            format!("{field} must be 1 through {max} plain characters"),
            format!("Shorten {field} and remove control characters"),
        ));
    }
    Ok(value.to_owned())
}

fn event_filter(value: &str) -> Value {
    json!({
        "operator": "and",
        "filters": [{"field": "event", "operator": "eq", "value": value}]
    })
}

fn url_filter(fragment: &str) -> Value {
    json!({
        "operator": "and",
        "filters": [{"field": "url", "operator": "i_contains", "value": fragment}]
    })
}

/// Build the Meta rule object for one typed rule.
fn rule_value(rule: &AudienceRule) -> Result<Value, PublicError> {
    match rule {
        AudienceRule::Instagram {
            instagram_account_id,
            event,
            retention_days,
        } => Ok(json!({
            "event_sources": [{"id": numeric_id(instagram_account_id, "instagram_account_id")?, "type": "ig_business"}],
            "retention_seconds": retention_seconds(*retention_days, MAX_ENGAGEMENT_RETENTION_DAYS, "Instagram engagement")?,
            "filter": event_filter(event.event()),
        })),
        AudienceRule::FacebookPage {
            page_id,
            event,
            retention_days,
        } => Ok(json!({
            "event_sources": [{"id": numeric_id(page_id, "page_id")?, "type": "page"}],
            "retention_seconds": retention_seconds(*retention_days, MAX_ENGAGEMENT_RETENTION_DAYS, "Facebook Page engagement")?,
            "filter": event_filter(event.event()),
        })),
        AudienceRule::Website {
            pixel_id,
            event,
            retention_days,
            url_contains,
            top_percent,
            event_name,
        } => {
            let source = json!([{"id": numeric_id(pixel_id, "pixel_id")?, "type": "pixel"}]);
            let seconds = retention_seconds(
                *retention_days,
                event.max_retention_days(),
                if *event == WebsiteEvent::Purchase {
                    "Website Purchase"
                } else {
                    "Website"
                },
            )?;
            let only = |present: bool, field: &str| -> Result<(), PublicError> {
                if present {
                    Err(invalid(
                        format!("{field} is not used by this website event"),
                        format!("Remove {field}"),
                    ))
                } else {
                    Ok(())
                }
            };
            if *event != WebsiteEvent::UrlContains {
                only(url_contains.is_some(), "url_contains")?;
            }
            if *event != WebsiteEvent::TopTimeSpent {
                only(top_percent.is_some(), "top_percent")?;
            }
            if *event != WebsiteEvent::CustomEvent {
                only(event_name.is_some(), "event_name")?;
            }
            let mut rule = Map::new();
            rule.insert("event_sources".to_owned(), source);
            rule.insert("retention_seconds".to_owned(), json!(seconds));
            match event {
                WebsiteEvent::AllVisitors => {
                    rule.insert("filter".to_owned(), url_filter(""));
                }
                WebsiteEvent::UrlContains => {
                    let fragment = bounded_plain(
                        url_contains.as_deref().unwrap_or_default(),
                        "url_contains",
                        MAX_URL_FRAGMENT_CHARS,
                    )?;
                    rule.insert("filter".to_owned(), url_filter(&fragment));
                }
                WebsiteEvent::TopTimeSpent => {
                    let percent = match top_percent {
                        Some(percent @ (5 | 10 | 25)) => *percent,
                        _ => {
                            return Err(invalid(
                                "top_time_spent needs top_percent 5, 10 or 25",
                                "Set top_percent to 5, 10 or 25",
                            ));
                        }
                    };
                    rule.insert("filter".to_owned(), url_filter(""));
                    rule.insert("template".to_owned(), json!("TOP_TIME_SPENDERS"));
                    rule.insert(
                        "aggregation".to_owned(),
                        json!({
                            "type": "time_spent",
                            "method": "percentile",
                            "operator": "in_range",
                            "value": {"from": 100 - percent, "to": 100}
                        }),
                    );
                }
                WebsiteEvent::CustomEvent => {
                    let name = custom_event_name(event_name.as_deref().unwrap_or_default())?;
                    rule.insert("filter".to_owned(), event_filter(&name));
                }
                standard => {
                    let name = standard.standard_name().unwrap_or_default();
                    rule.insert("filter".to_owned(), event_filter(name));
                }
            }
            Ok(Value::Object(rule))
        }
    }
}

fn custom_event_name(raw: &str) -> Result<String, PublicError> {
    let name = raw.trim();
    if name.is_empty()
        || name.chars().count() > MAX_EVENT_NAME_CHARS
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(invalid(
            format!(
                "custom event names are 1 through {MAX_EVENT_NAME_CHARS} letters, digits, _ or -"
            ),
            "Use the exact custom event name sent by the pixel",
        ));
    }
    Ok(name.to_owned())
}

fn rule_group(rules: &[AudienceRule], field: &str) -> Result<Value, PublicError> {
    if rules.is_empty() || rules.len() > MAX_RULES {
        return Err(invalid(
            format!("{field} must contain 1 through {MAX_RULES} rules"),
            format!("Split the audience or remove {field} rules"),
        ));
    }
    let rules = rules
        .iter()
        .map(rule_value)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"operator": "or", "rules": rules}))
}

/// Encode the full Meta `rule` parameter.
fn encode_rule(
    inclusions: &[AudienceRule],
    exclusions: &[AudienceRule],
) -> Result<String, PublicError> {
    let mut rule = Map::new();
    rule.insert(
        "inclusions".to_owned(),
        rule_group(inclusions, "inclusions")?,
    );
    if !exclusions.is_empty() {
        rule.insert(
            "exclusions".to_owned(),
            rule_group(exclusions, "exclusions")?,
        );
    }
    serde_json::to_string(&Value::Object(rule)).map_err(|_| {
        invalid(
            "audience rule could not be encoded",
            "Use the typed rule fields",
        )
    })
}

fn created_id(payload: &Value) -> Result<String, PublicError> {
    if payload.get("error").is_some()
        || payload.get("success").and_then(Value::as_bool) == Some(false)
    {
        return Err(ambiguous_mutation_result(
            "Meta did not confirm the new audience",
            "List the account's audiences before creating it again",
        ));
    }
    payload.get("id").and_then(numeric_value).ok_or_else(|| {
        ambiguous_mutation_result(
            "Meta did not confirm the new audience ID",
            "List the account's audiences before creating it again",
        )
    })
}

fn account(raw: &str) -> Result<String, PublicError> {
    normalize_ad_account_id(raw).ok_or_else(|| {
        invalid(
            "ad_account_id must be a numeric Meta ad-account ID",
            "Use an ID such as `act_123456789` or `123456789`",
        )
    })
}

// ---------------------------------------------------------------------------
// create_engagement_audience
// ---------------------------------------------------------------------------

/// Create one engagement or website audience from typed rules.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateEngagementAudienceInput {
    /// Numeric Meta ad-account ID, with or without `act_`.
    pub ad_account_id: String,
    /// Audience name, 1 through 256 characters.
    pub name: String,
    /// Optional description, up to 900 characters.
    pub description: Option<String>,
    /// 1 through 10 rules; people matching any of them are included.
    #[schemars(length(min = 1, max = 10))]
    pub inclusions: Vec<AudienceRule>,
    /// Optional rules; people matching any of them are left out.
    #[schemars(length(min = 1, max = 10))]
    pub exclusions: Option<Vec<AudienceRule>>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CreatedEngagementAudience {
    pub custom_audience_id: String,
}

fn build_create_form(
    name: &str,
    description: Option<&str>,
    rule: String,
) -> Result<Vec<(String, String)>, PublicError> {
    let mut form = vec![
        (
            "name".to_owned(),
            bounded_plain(name, "name", MAX_NAME_CHARS)?,
        ),
        ("rule".to_owned(), rule),
        ("prefill".to_owned(), "true".to_owned()),
    ];
    if let Some(description) = description {
        let value = description.trim();
        if value.chars().count() > MAX_DESCRIPTION_CHARS
            || value.chars().any(|c| c.is_control() && c != '\n')
        {
            return Err(invalid(
                format!("description must be at most {MAX_DESCRIPTION_CHARS} plain characters"),
                "Shorten the description",
            ));
        }
        if !value.is_empty() {
            form.push(("description".to_owned(), value.to_owned()));
        }
    }
    Ok(form)
}

pub(crate) async fn create_engagement_audience(
    graph: &GraphClient,
    input: CreateEngagementAudienceInput,
) -> ToolResponse<CreatedEngagementAudience> {
    let request = (|| {
        let account_id = account(&input.ad_account_id)?;
        let rule = encode_rule(
            &input.inclusions,
            input.exclusions.as_deref().unwrap_or_default(),
        )?;
        let form = build_create_form(&input.name, input.description.as_deref(), rule)?;
        Ok::<_, PublicError>((format!("{account_id}/customaudiences"), form))
    })();
    let (endpoint, form) = match request {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    match graph.post_form_json(&endpoint, &form).await {
        Ok(payload) => match created_id(&payload) {
            Ok(custom_audience_id) => {
                ToolResponse::success(CreatedEngagementAudience { custom_audience_id })
            }
            Err(error) => ToolResponse::error(error),
        },
        Err(error) => ToolResponse::error(mutation_error_without_blind_retry(
            error,
            "List the account's audiences before creating it again",
        )),
    }
}

// ---------------------------------------------------------------------------
// ensure_audience_set
// ---------------------------------------------------------------------------

/// Built-in audience sets.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AudiencePreset {
    /// About 12 funnel audiences: engaged 30/90/365/730 days, messaged 365,
    /// site visitors 30/180, top 25% time on site, checkout without purchase,
    /// leads, purchasers 730 and one combined hot audience.
    FunnelCore,
    /// Every Instagram, Page and website event for each retention window
    /// (default 3, 7, 14, 30, 60, 90, 180, 365 and 730 days, within each source's limit).
    EngagementMatrix,
}

impl AudiencePreset {
    const fn as_str(self) -> &'static str {
        match self {
            Self::FunnelCore => "funnel_core",
            Self::EngagementMatrix => "engagement_matrix",
        }
    }
}

/// Language of the generated audience names.
#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LabelLanguage {
    #[default]
    En,
    PtBr,
}

/// One audience of a custom set.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudienceSpec {
    /// Stable key within the set: lowercase letters, digits and `_`, up to 64.
    pub key: String,
    /// Audience name (the name prefix is added in front).
    pub name: String,
    #[schemars(length(min = 1, max = 10))]
    pub inclusions: Vec<AudienceRule>,
    #[schemars(length(min = 1, max = 10))]
    pub exclusions: Option<Vec<AudienceRule>>,
}

/// Bring an ad account to a declared set of engagement and website audiences.
/// Without `apply_acknowledgement` this only previews. Existing audiences of the
/// set are skipped, so repeating the call never duplicates.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnsureAudienceSetInput {
    /// Numeric Meta ad-account ID, with or without `act_`.
    pub ad_account_id: String,
    /// Built-in set. Provide exactly one of `preset` or `audiences`.
    pub preset: Option<AudiencePreset>,
    /// Custom set, 1 through 200 audiences.
    #[schemars(length(min = 1, max = 200))]
    pub audiences: Option<Vec<AudienceSpec>>,
    /// Instagram professional-account ID used by preset audiences.
    pub instagram_account_id: Option<String>,
    /// Facebook Page ID used by preset audiences.
    pub page_id: Option<String>,
    /// Pixel (dataset) ID used by preset audiences.
    pub pixel_id: Option<String>,
    /// Name of the set written into each audience's description marker.
    /// Defaults to the preset name, or `custom`.
    pub set_name: Option<String>,
    /// Text placed in front of every audience name, e.g. `[Client] `.
    pub name_prefix: Option<String>,
    /// Language of preset audience names. Defaults to `en`.
    pub label_language: Option<LabelLanguage>,
    /// `engagement_matrix` only: retention windows in days.
    #[schemars(length(min = 1, max = 12), inner(range(min = 1, max = 730)))]
    pub retention_days: Option<Vec<u16>>,
    /// Presets: extra website audiences for URLs containing each fragment (up to 10).
    #[schemars(length(max = 10))]
    pub url_contains: Option<Vec<String>>,
    /// Presets: extra website audiences for each custom pixel event (up to 10).
    #[schemars(length(max = 10))]
    pub custom_events: Option<Vec<String>>,
    /// The `plan_token` returned by the preview. Required to apply.
    pub plan_token: Option<String>,
    /// Pass `APPLY_LIVE_META_ADS_CHANGES` after the operator approved the preview.
    #[schemars(
        length(min = 27, max = 27),
        regex(pattern = "^APPLY_LIVE_META_ADS_CHANGES$")
    )]
    pub apply_acknowledgement: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PlannedAudience {
    pub key: String,
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ExistingSetAudience {
    pub key: String,
    pub name: String,
    pub custom_audience_id: String,
    /// `marker` (description marker of this set) or `name`.
    pub matched_by: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SkippedAudience {
    pub key: String,
    pub reason: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CreatedSetAudience {
    pub key: String,
    pub name: String,
    pub custom_audience_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FailedSetAudience {
    pub key: String,
    pub error: PublicError,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AudienceSetResult {
    pub set_name: String,
    /// True when nothing was created (preview).
    pub dry_run: bool,
    /// Pass back with `apply_acknowledgement` to create `to_create`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_token: Option<String>,
    pub to_create: Vec<PlannedAudience>,
    pub existing: Vec<ExistingSetAudience>,
    pub skipped: Vec<SkippedAudience>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub created: Vec<CreatedSetAudience>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<FailedSetAudience>,
    /// Keys not attempted after a failure; run the call again to continue.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub not_attempted: Vec<String>,
    /// `true`, `false`, or absent when no pixel audience needed the check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pixel_events_checked: Option<bool>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct DesiredAudience {
    key: String,
    name: String,
    description: Option<String>,
    inclusions: Vec<AudienceRule>,
    exclusions: Vec<AudienceRule>,
}

struct Assets<'a> {
    instagram: Option<&'a str>,
    page: Option<&'a str>,
    pixel: Option<&'a str>,
}

fn instagram(id: &str, event: InstagramEngagement, days: u16) -> AudienceRule {
    AudienceRule::Instagram {
        instagram_account_id: id.to_owned(),
        event,
        retention_days: days,
    }
}

fn page(id: &str, event: PageEngagement, days: u16) -> AudienceRule {
    AudienceRule::FacebookPage {
        page_id: id.to_owned(),
        event,
        retention_days: days,
    }
}

fn website(id: &str, event: WebsiteEvent, days: u16) -> AudienceRule {
    AudienceRule::Website {
        pixel_id: id.to_owned(),
        event,
        retention_days: days,
        url_contains: None,
        top_percent: None,
        event_name: None,
    }
}

fn top_time(id: &str, percent: u8, days: u16) -> AudienceRule {
    AudienceRule::Website {
        pixel_id: id.to_owned(),
        event: WebsiteEvent::TopTimeSpent,
        retention_days: days,
        url_contains: None,
        top_percent: Some(percent),
        event_name: None,
    }
}

fn url_rule(id: &str, fragment: &str, days: u16) -> AudienceRule {
    AudienceRule::Website {
        pixel_id: id.to_owned(),
        event: WebsiteEvent::UrlContains,
        retention_days: days,
        url_contains: Some(fragment.to_owned()),
        top_percent: None,
        event_name: None,
    }
}

fn custom_rule(id: &str, name: &str, days: u16) -> AudienceRule {
    AudienceRule::Website {
        pixel_id: id.to_owned(),
        event: WebsiteEvent::CustomEvent,
        retention_days: days,
        url_contains: None,
        top_percent: None,
        event_name: Some(name.to_owned()),
    }
}

fn slug(raw: &str) -> String {
    let mut slug = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('_') {
            slug.push('_');
        }
    }
    let slug = slug.trim_matches('_');
    slug.chars().take(24).collect()
}

struct PresetBuilder<'a> {
    prefix: &'a str,
    language: LabelLanguage,
    items: Vec<DesiredAudience>,
    skipped: Vec<SkippedAudience>,
}

impl PresetBuilder<'_> {
    fn label(&self, en: &str, pt: &str) -> String {
        match self.language {
            LabelLanguage::En => en.to_owned(),
            LabelLanguage::PtBr => pt.to_owned(),
        }
    }

    fn push(
        &mut self,
        key: String,
        label: String,
        days: u16,
        inclusions: Vec<AudienceRule>,
        exclusions: Vec<AudienceRule>,
    ) {
        self.items.push(DesiredAudience {
            name: format!("{}{label} – {days}D", self.prefix),
            key,
            description: None,
            inclusions,
            exclusions,
        });
    }

    fn skip(&mut self, key: impl Into<String>, reason: impl Into<String>) {
        self.skipped.push(SkippedAudience {
            key: key.into(),
            reason: reason.into(),
        });
    }
}

fn sources_label(instagram: bool, page: bool) -> &'static str {
    match (instagram, page) {
        (true, true) => "IG+FB",
        (true, false) => "IG",
        _ => "FB",
    }
}

fn build_funnel_core(
    builder: &mut PresetBuilder<'_>,
    assets: &Assets<'_>,
    urls: &[String],
    customs: &[String],
) {
    let social = assets.instagram.is_some() || assets.page.is_some();
    let both = sources_label(assets.instagram.is_some(), assets.page.is_some());
    for days in [30_u16, 90, 365, 730] {
        let key = format!("eng_{days}");
        if !social {
            builder.skip(key, "needs instagram_account_id or page_id");
            continue;
        }
        let mut rules = Vec::new();
        if let Some(id) = assets.instagram {
            rules.push(instagram(id, InstagramEngagement::All, days));
        }
        if let Some(id) = assets.page {
            rules.push(page(id, PageEngagement::Engaged, days));
        }
        let label = builder.label(&format!("Engaged {both}"), &format!("Engajou {both}"));
        builder.push(key, label, days, rules, Vec::new());
    }
    if social {
        let mut rules = Vec::new();
        if let Some(id) = assets.instagram {
            rules.push(instagram(id, InstagramEngagement::Messaged, 365));
        }
        if let Some(id) = assets.page {
            rules.push(page(id, PageEngagement::Messaged, 365));
        }
        let label = builder.label(&format!("Messaged {both}"), &format!("Mensagem {both}"));
        builder.push("msg_365".to_owned(), label, 365, rules, Vec::new());
    } else {
        builder.skip("msg_365", "needs instagram_account_id or page_id");
    }

    let Some(pixel) = assets.pixel else {
        for key in [
            "site_30",
            "site_180",
            "top25_180",
            "checkout_30_ex_purchase",
            "lead_180",
            "purchase_730",
        ] {
            builder.skip(key, "needs pixel_id");
        }
        if social {
            builder.skip("hot_all_30", "needs pixel_id");
        }
        return;
    };
    for days in [30_u16, 180] {
        let label = builder.label("Website visitors", "Visitantes do site");
        builder.push(
            format!("site_{days}"),
            label,
            days,
            vec![website(pixel, WebsiteEvent::AllVisitors, days)],
            Vec::new(),
        );
    }
    let label = builder.label("Top 25% time on site", "Top 25% tempo no site");
    builder.push(
        "top25_180".to_owned(),
        label,
        180,
        vec![top_time(pixel, 25, 180)],
        Vec::new(),
    );
    let label = builder.label("Checkout without purchase", "Checkout sem compra");
    builder.push(
        "checkout_30_ex_purchase".to_owned(),
        label,
        30,
        vec![website(pixel, WebsiteEvent::InitiateCheckout, 30)],
        vec![website(pixel, WebsiteEvent::Purchase, 30)],
    );
    let label = builder.label("Leads and sign-ups", "Leads e cadastros");
    builder.push(
        "lead_180".to_owned(),
        label,
        180,
        vec![
            website(pixel, WebsiteEvent::Lead, 180),
            website(pixel, WebsiteEvent::CompleteRegistration, 180),
        ],
        Vec::new(),
    );
    let label = builder.label("Purchasers", "Compradores");
    builder.push(
        "purchase_730".to_owned(),
        label,
        730,
        vec![website(pixel, WebsiteEvent::Purchase, 730)],
        Vec::new(),
    );
    if social {
        let mut rules = Vec::new();
        if let Some(id) = assets.instagram {
            rules.push(instagram(id, InstagramEngagement::All, 30));
        }
        if let Some(id) = assets.page {
            rules.push(page(id, PageEngagement::Engaged, 30));
        }
        rules.push(website(pixel, WebsiteEvent::AllVisitors, 30));
        let label = builder.label("Hot: engaged or visited", "Quente: engajou ou visitou");
        builder.push("hot_all_30".to_owned(), label, 30, rules, Vec::new());
    }
    for (index, fragment) in urls.iter().enumerate() {
        for days in [30_u16, 180] {
            let label = format!("URL {fragment}");
            builder.push(
                format!("url_{}_{}_{days}", index + 1, slug(fragment)),
                label,
                days,
                vec![url_rule(pixel, fragment, days)],
                Vec::new(),
            );
        }
    }
    for (index, name) in customs.iter().enumerate() {
        for days in [30_u16, 180] {
            builder.push(
                format!("event_{}_{}_{days}", index + 1, slug(name)),
                name.clone(),
                days,
                vec![custom_rule(pixel, name, days)],
                Vec::new(),
            );
        }
    }
}

fn build_matrix(
    builder: &mut PresetBuilder<'_>,
    assets: &Assets<'_>,
    windows: &[u16],
    urls: &[String],
    customs: &[String],
) {
    const IG: [(InstagramEngagement, &str, &str, &str); 5] = [
        (
            InstagramEngagement::All,
            "all",
            "IG all engagement",
            "IG – [TODOS] Envolvimento",
        ),
        (
            InstagramEngagement::ProfileVisit,
            "visit",
            "IG profile visit",
            "IG – [VISITARAM] Envolvimento",
        ),
        (
            InstagramEngagement::Engaged,
            "engaged",
            "IG engaged",
            "IG – [ENVOLVERAM] Envolvimento",
        ),
        (
            InstagramEngagement::Messaged,
            "messaged",
            "IG messaged",
            "IG – [MENSAGEM] Envolvimento",
        ),
        (
            InstagramEngagement::Saved,
            "saved",
            "IG saved",
            "IG – [SALVARAM] Envolvimento",
        ),
    ];
    const PAGE: [(PageEngagement, &str, &str, &str); 5] = [
        (
            PageEngagement::Engaged,
            "all",
            "FB all engagement",
            "FB – [TODOS] Envolvimento",
        ),
        (
            PageEngagement::Visited,
            "visit",
            "FB Page visit",
            "FB – [VISITARAM] Envolvimento",
        ),
        (
            PageEngagement::Saved,
            "saved",
            "FB saved",
            "FB – [SALVARAM] Envolvimento",
        ),
        (
            PageEngagement::Messaged,
            "messaged",
            "FB messaged",
            "FB – [MENSAGEM] Envolvimento",
        ),
        (
            PageEngagement::PostInteraction,
            "engaged",
            "FB post interaction",
            "FB – [ENVOLVERAM] Envolvimento",
        ),
    ];
    const SITE_EVENTS: [(WebsiteEvent, &str); 5] = [
        (WebsiteEvent::PageView, "pageview"),
        (WebsiteEvent::InitiateCheckout, "initiatecheckout"),
        (WebsiteEvent::Lead, "lead"),
        (WebsiteEvent::CompleteRegistration, "completeregistration"),
        (WebsiteEvent::Purchase, "purchase"),
    ];
    match assets.instagram {
        Some(id) => {
            for (event, key, en, pt) in IG {
                for &days in windows
                    .iter()
                    .filter(|days| **days <= MAX_ENGAGEMENT_RETENTION_DAYS)
                {
                    let label = builder.label(en, pt);
                    builder.push(
                        format!("ig_{key}_{days}"),
                        label,
                        days,
                        vec![instagram(id, event, days)],
                        Vec::new(),
                    );
                }
            }
        }
        None => builder.skip("ig_*", "needs instagram_account_id"),
    }
    match assets.page {
        Some(id) => {
            for (event, key, en, pt) in PAGE {
                for &days in windows
                    .iter()
                    .filter(|days| **days <= MAX_ENGAGEMENT_RETENTION_DAYS)
                {
                    let label = builder.label(en, pt);
                    builder.push(
                        format!("fb_{key}_{days}"),
                        label,
                        days,
                        vec![page(id, event, days)],
                        Vec::new(),
                    );
                }
            }
        }
        None => builder.skip("fb_*", "needs page_id"),
    }
    let Some(pixel) = assets.pixel else {
        builder.skip("site_*", "needs pixel_id");
        return;
    };
    let site_windows = windows
        .iter()
        .copied()
        .filter(|days| *days <= MAX_WEBSITE_RETENTION_DAYS)
        .collect::<Vec<_>>();
    for &days in &site_windows {
        let label = builder.label("Website visitors", "View Page – SITE");
        builder.push(
            format!("site_all_{days}"),
            label,
            days,
            vec![website(pixel, WebsiteEvent::AllVisitors, days)],
            Vec::new(),
        );
        for percent in [5_u8, 10, 25] {
            let label = builder.label(
                &format!("Top {percent}% time on site"),
                &format!("View Page – SITE {percent}%"),
            );
            builder.push(
                format!("site_top{percent}_{days}"),
                label,
                days,
                vec![top_time(pixel, percent, days)],
                Vec::new(),
            );
        }
        for (index, fragment) in urls.iter().enumerate() {
            builder.push(
                format!("url_{}_{}_{days}", index + 1, slug(fragment)),
                format!("URL {fragment}"),
                days,
                vec![url_rule(pixel, fragment, days)],
                Vec::new(),
            );
        }
        for (index, name) in customs.iter().enumerate() {
            builder.push(
                format!("event_{}_{}_{days}", index + 1, slug(name)),
                name.clone(),
                days,
                vec![custom_rule(pixel, name, days)],
                Vec::new(),
            );
        }
    }
    for (event, key) in SITE_EVENTS {
        let name = event.standard_name().unwrap_or_default();
        for &days in windows
            .iter()
            .filter(|days| **days <= event.max_retention_days())
        {
            builder.push(
                format!("px_{key}_{days}"),
                name.to_owned(),
                days,
                vec![website(pixel, event, days)],
                Vec::new(),
            );
        }
    }
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_KEY_CHARS
        && key
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn set_name(input: &EnsureAudienceSetInput) -> Result<String, PublicError> {
    let name = match (&input.set_name, input.preset) {
        (Some(name), _) => name.trim().to_owned(),
        (None, Some(preset)) => preset.as_str().to_owned(),
        (None, None) => "custom".to_owned(),
    };
    if name.is_empty()
        || name.len() > MAX_SET_NAME_CHARS
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(invalid(
            format!("set_name must be 1 through {MAX_SET_NAME_CHARS} letters, digits, _ or -"),
            "Use a short identifier such as funnel_core",
        ));
    }
    Ok(name)
}

fn extra_list(
    values: Option<&[String]>,
    field: &str,
    max_chars: usize,
) -> Result<Vec<String>, PublicError> {
    let values = values.unwrap_or_default();
    if values.len() > MAX_EXTRA_FILTERS {
        return Err(invalid(
            format!("{field} accepts at most {MAX_EXTRA_FILTERS} values"),
            format!("Shorten {field}"),
        ));
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        let value = bounded_plain(value, field, max_chars)?;
        if seen.insert(value.clone()) {
            out.push(value);
        }
    }
    Ok(out)
}

fn optional_id(raw: Option<&str>, field: &str) -> Result<Option<String>, PublicError> {
    raw.map(|raw| numeric_id(raw, field)).transpose()
}

/// Turn the input into the desired audience list (before pixel checks).
fn desired_audiences(
    input: &EnsureAudienceSetInput,
) -> Result<(Vec<DesiredAudience>, Vec<SkippedAudience>), PublicError> {
    let prefix = match input.name_prefix.as_deref() {
        Some(prefix)
            if prefix.chars().count() > MAX_PREFIX_CHARS
                || prefix.chars().any(char::is_control) =>
        {
            return Err(invalid(
                format!("name_prefix must be at most {MAX_PREFIX_CHARS} plain characters"),
                "Shorten name_prefix",
            ));
        }
        Some(prefix) => prefix.to_owned(),
        None => String::new(),
    };
    match (input.preset, &input.audiences) {
        (Some(preset), None) => {
            let instagram = optional_id(
                input.instagram_account_id.as_deref(),
                "instagram_account_id",
            )?;
            let page = optional_id(input.page_id.as_deref(), "page_id")?;
            let pixel = optional_id(input.pixel_id.as_deref(), "pixel_id")?;
            if instagram.is_none() && page.is_none() && pixel.is_none() {
                return Err(invalid(
                    "a preset needs at least one of instagram_account_id, page_id or pixel_id",
                    "Pass the account's Instagram, Page and pixel IDs",
                ));
            }
            let urls = extra_list(
                input.url_contains.as_deref(),
                "url_contains",
                MAX_URL_FRAGMENT_CHARS,
            )?;
            let customs = extra_list(
                input.custom_events.as_deref(),
                "custom_events",
                MAX_EVENT_NAME_CHARS,
            )?
            .into_iter()
            .map(|name| custom_event_name(&name))
            .collect::<Result<Vec<_>, _>>()?;
            let assets = Assets {
                instagram: instagram.as_deref(),
                page: page.as_deref(),
                pixel: pixel.as_deref(),
            };
            let mut builder = PresetBuilder {
                prefix: &prefix,
                language: input.label_language.unwrap_or_default(),
                items: Vec::new(),
                skipped: Vec::new(),
            };
            match preset {
                AudiencePreset::FunnelCore => {
                    if input.retention_days.is_some() {
                        return Err(invalid(
                            "retention_days applies only to engagement_matrix",
                            "Remove retention_days or use preset engagement_matrix",
                        ));
                    }
                    build_funnel_core(&mut builder, &assets, &urls, &customs);
                }
                AudiencePreset::EngagementMatrix => {
                    let windows = match &input.retention_days {
                        Some(days) => {
                            let unique = days.iter().copied().collect::<BTreeSet<_>>();
                            if unique.is_empty()
                                || unique.len() > DEFAULT_MATRIX_DAYS.len() + 3
                                || unique
                                    .iter()
                                    .any(|day| *day == 0 || *day > MAX_ENGAGEMENT_RETENTION_DAYS)
                            {
                                return Err(invalid(
                                    "retention_days must hold 1 through 12 windows of 1 to 730 days",
                                    "Use windows such as [7, 30, 90, 180, 365]",
                                ));
                            }
                            unique.into_iter().collect()
                        }
                        None => DEFAULT_MATRIX_DAYS.to_vec(),
                    };
                    build_matrix(&mut builder, &assets, &windows, &urls, &customs);
                }
            }
            Ok((builder.items, builder.skipped))
        }
        (None, Some(audiences)) => {
            if audiences.is_empty() {
                return Err(invalid(
                    "audiences must not be empty",
                    "Add at least one audience",
                ));
            }
            if input.retention_days.is_some()
                || input.url_contains.is_some()
                || input.custom_events.is_some()
            {
                return Err(invalid(
                    "retention_days, url_contains and custom_events apply only to presets",
                    "Put those conditions in the custom audiences' rules",
                ));
            }
            let mut items = Vec::with_capacity(audiences.len());
            for spec in audiences {
                if !valid_key(&spec.key) {
                    return Err(invalid(
                        format!(
                            "audience key `{}` is invalid",
                            spec.key.chars().take(MAX_KEY_CHARS).collect::<String>()
                        ),
                        "Use lowercase letters, digits and _ (up to 64)",
                    ));
                }
                items.push(DesiredAudience {
                    key: spec.key.clone(),
                    name: format!("{prefix}{}", spec.name.trim()),
                    description: None,
                    inclusions: spec.inclusions.clone(),
                    exclusions: spec.exclusions.clone().unwrap_or_default(),
                });
            }
            Ok((items, Vec::new()))
        }
        _ => Err(invalid(
            "provide exactly one of preset or audiences",
            "Choose a built-in preset or pass your own audiences",
        )),
    }
}

/// Drop website rules for pixel events that never fired. Returns the skipped audiences.
fn apply_pixel_activity(
    items: Vec<DesiredAudience>,
    fired: &BTreeMap<String, u64>,
) -> (Vec<DesiredAudience>, Vec<SkippedAudience>) {
    let silent = |rule: &AudienceRule| {
        rule.pixel_event_name()
            .is_some_and(|name| fired.get(&name).copied().unwrap_or(0) == 0)
    };
    let mut kept = Vec::with_capacity(items.len());
    let mut skipped = Vec::new();
    for mut item in items {
        let silent_events = item
            .inclusions
            .iter()
            .filter(|rule| silent(rule))
            .filter_map(AudienceRule::pixel_event_name)
            .collect::<Vec<_>>();
        item.inclusions.retain(|rule| !silent(rule));
        item.exclusions.retain(|rule| !silent(rule));
        if item.inclusions.is_empty() {
            skipped.push(SkippedAudience {
                key: item.key,
                reason: format!(
                    "pixel event {} did not fire in the last {PIXEL_STATS_DAYS} days",
                    silent_events.join(", ")
                ),
            });
        } else {
            kept.push(item);
        }
    }
    (kept, skipped)
}

fn marker(set: &str, key: &str) -> String {
    format!("{MARKER_PREFIX}{set};key={key}")
}

fn parse_marker(description: &str) -> Option<(&str, &str)> {
    let start = description.find(MARKER_PREFIX)? + MARKER_PREFIX.len();
    let rest = &description[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let (set, key) = rest[..end].split_once(";key=")?;
    Some((set, key))
}

#[derive(Debug, Clone)]
struct ExistingAudience {
    id: String,
    name: String,
    description: String,
}

async fn list_existing(
    graph: &GraphClient,
    account_id: &str,
) -> Result<Vec<ExistingAudience>, PublicError> {
    let endpoint = format!("{account_id}/customaudiences");
    let mut out = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..MAX_LIST_PAGES {
        let mut query = vec![
            ("fields".to_owned(), LIST_FIELDS.to_owned()),
            ("limit".to_owned(), "200".to_owned()),
        ];
        if let Some(cursor) = &after {
            query.push(("after".to_owned(), cursor.clone()));
        }
        let payload = graph
            .get_json(&endpoint, &query)
            .await
            .map_err(PublicError::from)?;
        let data = payload
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid_provider("Meta returned an unexpected audience list"))?;
        for row in data {
            let Some(id) = row.get("id").and_then(numeric_value) else {
                continue;
            };
            out.push(ExistingAudience {
                id,
                name: row
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                description: row
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            });
        }
        let next = payload.pointer("/paging/next").is_some();
        after = payload
            .pointer("/paging/cursors/after")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if !next || after.is_none() {
            return Ok(out);
        }
    }
    Err(invalid_provider(
        "The account has more audiences than this tool reads in one call",
    ))
}

fn invalid_provider(message: &str) -> PublicError {
    PublicError {
        code: "UNEXPECTED_RESPONSE".to_owned(),
        message: message.to_owned(),
        retryable: false,
        action: Some("Check the account's audiences in Meta Ads Manager".to_owned()),
    }
}

/// Sum pixel event counts over the last 30 days. `None` when Meta does not answer.
async fn pixel_event_counts(
    graph: &GraphClient,
    pixel_id: &str,
    now: u64,
) -> Option<BTreeMap<String, u64>> {
    let endpoint = format!("{pixel_id}/stats");
    let start = now.saturating_sub(PIXEL_STATS_DAYS * SECONDS_PER_DAY);
    let mut counts = BTreeMap::new();
    let mut after: Option<String> = None;
    for _ in 0..MAX_STATS_PAGES {
        let mut query = vec![
            ("aggregation".to_owned(), "event".to_owned()),
            ("start_time".to_owned(), start.to_string()),
            ("limit".to_owned(), "1000".to_owned()),
        ];
        if let Some(cursor) = &after {
            query.push(("after".to_owned(), cursor.clone()));
        }
        let payload = graph.get_json(&endpoint, &query).await.ok()?;
        for bucket in payload.get("data")?.as_array()? {
            for entry in bucket
                .get("data")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let (Some(event), Some(count)) = (
                    entry.get("value").and_then(Value::as_str),
                    entry.get("count").and_then(Value::as_u64),
                ) else {
                    continue;
                };
                *counts.entry(event.to_owned()).or_insert(0) += count;
            }
        }
        let next = payload.pointer("/paging/next").is_some();
        after = payload
            .pointer("/paging/cursors/after")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if !next || after.is_none() {
            return Some(counts);
        }
    }
    // Partial data could hide events that did fire; treat as unknown.
    None
}

struct Plan {
    to_create: Vec<(DesiredAudience, String)>,
    existing: Vec<ExistingSetAudience>,
    token: String,
}

fn plan_audiences(
    account_id: &str,
    set: &str,
    items: Vec<DesiredAudience>,
    existing: &[ExistingAudience],
) -> Result<Plan, PublicError> {
    let mut by_marker = BTreeMap::new();
    let mut by_name = BTreeMap::new();
    for audience in existing {
        if let Some((audience_set, key)) = parse_marker(&audience.description)
            && audience_set == set
        {
            by_marker.entry(key.to_owned()).or_insert(audience);
        }
        by_name.entry(audience.name.clone()).or_insert(audience);
    }
    let mut seen = BTreeSet::new();
    let mut to_create = Vec::new();
    let mut matched = Vec::new();
    for item in items {
        if !seen.insert(item.key.clone()) {
            return Err(invalid(
                format!("audience key `{}` appears twice", item.key),
                "Give every audience in the set a unique key",
            ));
        }
        bounded_plain(&item.name, "audience name", MAX_NAME_CHARS)?;
        if let Some(found) = by_marker.get(&item.key) {
            matched.push(ExistingSetAudience {
                key: item.key,
                name: found.name.clone(),
                custom_audience_id: found.id.clone(),
                matched_by: "marker".to_owned(),
            });
        } else if let Some(found) = by_name.get(&item.name) {
            matched.push(ExistingSetAudience {
                key: item.key,
                name: found.name.clone(),
                custom_audience_id: found.id.clone(),
                matched_by: "name".to_owned(),
            });
        } else {
            let rule = encode_rule(&item.inclusions, &item.exclusions)?;
            to_create.push((item, rule));
        }
    }
    let mut hasher = Sha256::new();
    hasher.update(account_id.as_bytes());
    hasher.update(b"\n");
    hasher.update(set.as_bytes());
    for (item, rule) in &to_create {
        hasher.update(b"\n");
        hasher.update(item.key.as_bytes());
        hasher.update(b"\t");
        hasher.update(item.name.as_bytes());
        hasher.update(b"\t");
        hasher.update(rule.as_bytes());
    }
    let mut token = String::with_capacity(PLAN_TOKEN_CHARS);
    for byte in hasher.finalize().iter().take(PLAN_TOKEN_CHARS / 2) {
        let _ = write!(token, "{byte:02x}");
    }
    Ok(Plan {
        to_create,
        existing: matched,
        token,
    })
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

pub(crate) async fn ensure_audience_set(
    graph: &GraphClient,
    input: EnsureAudienceSetInput,
) -> ToolResponse<AudienceSetResult> {
    match ensure_audience_set_inner(graph, input, now_seconds(), CREATE_SPACING).await {
        Ok(result) => ToolResponse::success(result),
        Err(error) => ToolResponse::error(error),
    }
}

async fn ensure_audience_set_inner(
    graph: &GraphClient,
    input: EnsureAudienceSetInput,
    now: u64,
    spacing: Duration,
) -> Result<AudienceSetResult, PublicError> {
    let account_id = account(&input.ad_account_id)?;
    let set = set_name(&input)?;
    let applying = match input.apply_acknowledgement.as_deref() {
        None => false,
        Some(APPLY_ACKNOWLEDGEMENT) => true,
        Some(_) => {
            return Err(invalid(
                "apply_acknowledgement is invalid",
                "After the operator approves the preview, pass APPLY_LIVE_META_ADS_CHANGES exactly",
            ));
        }
    };
    if applying && input.plan_token.is_none() {
        return Err(invalid(
            "applying needs the plan_token from the preview",
            "Run the call without apply_acknowledgement first and pass its plan_token",
        ));
    }

    let (items, mut skipped) = desired_audiences(&input)?;
    let mut warnings = Vec::new();
    let needs_pixel_check = items
        .iter()
        .flat_map(|item| item.inclusions.iter().chain(&item.exclusions))
        .any(|rule| rule.pixel_event_name().is_some());
    let mut pixel_events_checked = None;
    let items = if needs_pixel_check {
        let pixel = items
            .iter()
            .flat_map(|item| item.inclusions.iter().chain(&item.exclusions))
            .find_map(|rule| match rule {
                AudienceRule::Website { pixel_id, .. } => normalize_numeric_id(pixel_id),
                _ => None,
            });
        match pixel {
            Some(pixel) => match pixel_event_counts(graph, &pixel, now).await {
                Some(counts) => {
                    pixel_events_checked = Some(true);
                    let (kept, silent) = apply_pixel_activity(items, &counts);
                    skipped.extend(silent);
                    kept
                }
                None => {
                    pixel_events_checked = Some(false);
                    warnings.push(
                        "Pixel event activity could not be read; event audiences are planned without that check".to_owned(),
                    );
                    items
                }
            },
            None => items,
        }
    } else {
        items
    };
    if items.len() > MAX_SET_ITEMS {
        return Err(invalid(
            format!(
                "the set has {} audiences; one call handles at most {MAX_SET_ITEMS}",
                items.len()
            ),
            "Use fewer retention windows or split the set",
        ));
    }

    let existing = list_existing(graph, &account_id).await?;
    let plan = plan_audiences(&account_id, &set, items, &existing)?;
    if existing.len() + plan.to_create.len() > ACCOUNT_AUDIENCE_SOFT_LIMIT {
        warnings.push(format!(
            "The account would hold {} custom audiences; Meta limits an ad account to about {ACCOUNT_AUDIENCE_SOFT_LIMIT}",
            existing.len() + plan.to_create.len()
        ));
    }
    let planned = plan
        .to_create
        .iter()
        .map(|(item, _)| PlannedAudience {
            key: item.key.clone(),
            name: item.name.clone(),
        })
        .collect::<Vec<_>>();

    if !applying {
        return Ok(AudienceSetResult {
            set_name: set,
            dry_run: true,
            plan_token: (!planned.is_empty()).then(|| plan.token.clone()),
            to_create: planned,
            existing: plan.existing,
            skipped,
            created: Vec::new(),
            failed: None,
            not_attempted: Vec::new(),
            pixel_events_checked,
            warnings,
        });
    }
    if input.plan_token.as_deref() != Some(plan.token.as_str()) {
        return Err(PublicError {
            code: "PLAN_CHANGED".to_owned(),
            message: "The audience plan changed since the preview".to_owned(),
            retryable: false,
            action: Some(
                "Run the preview again, review it, and apply its new plan_token".to_owned(),
            ),
        });
    }

    let endpoint = format!("{account_id}/customaudiences");
    let mut created = Vec::new();
    let mut failed = None;
    let mut not_attempted = Vec::new();
    let mut iter = plan.to_create.into_iter().peekable();
    let mut first = true;
    while let Some((item, rule)) = iter.next() {
        if !first && !spacing.is_zero() {
            tokio::time::sleep(spacing).await;
        }
        first = false;
        let mut description = item.description.clone().unwrap_or_default();
        if !description.is_empty() {
            description.push('\n');
        }
        description.push_str(&marker(&set, &item.key));
        let outcome = match build_create_form(&item.name, Some(&description), rule) {
            Ok(form) => match graph.post_form_json(&endpoint, &form).await {
                Ok(payload) => created_id(&payload),
                Err(error) => Err(mutation_error_without_blind_retry(
                    error,
                    "Run ensure_audience_set again; existing audiences of the set are skipped",
                )),
            },
            Err(error) => Err(error),
        };
        match outcome {
            Ok(custom_audience_id) => created.push(CreatedSetAudience {
                key: item.key,
                name: item.name,
                custom_audience_id,
            }),
            Err(error) => {
                failed = Some(FailedSetAudience {
                    key: item.key,
                    error,
                });
                not_attempted = iter.map(|(item, _)| item.key).collect();
                break;
            }
        }
    }
    Ok(AudienceSetResult {
        set_name: set,
        dry_run: false,
        plan_token: None,
        to_create: planned,
        existing: plan.existing,
        skipped,
        created,
        failed,
        not_attempted,
        pixel_events_checked,
        warnings,
    })
}

// ---------------------------------------------------------------------------
// audit_audiences
// ---------------------------------------------------------------------------

/// Read the size and health of an ad account's custom audiences.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditAudiencesInput {
    /// Numeric Meta ad-account ID, with or without `act_`.
    pub ad_account_id: String,
    /// Only audiences carrying this set's marker.
    pub set_name: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AuditedAudience {
    pub custom_audience_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtype: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub set_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub set_key: Option<String>,
    /// Meta's lower size bound; negative while Meta is still sizing it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_lower_bound: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_upper_bound: Option<i64>,
    /// Meta's operation status (200 = normal).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_status: Option<AudienceStatus>,
    /// Meta's delivery status (200 = ready).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_status: Option<AudienceStatus>,
    /// `too_small_to_deliver` (delivery status 300), `delivery_issue` (other delivery
    /// status), `too_small_for_lookalike_seed` (<100 people), `size_pending`,
    /// `populating` (operation status 441: usable while it fills) and
    /// `needs_attention` (any other operation status).
    pub flags: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AudienceStatus {
    pub code: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

fn status(row: &Value, field: &str) -> Option<AudienceStatus> {
    let value = row.get(field)?;
    Some(AudienceStatus {
        code: value.get("code")?.as_i64()?,
        description: value
            .get("description")
            .and_then(Value::as_str)
            .map(|text| text.chars().take(300).collect()),
    })
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AudienceAudit {
    pub total: usize,
    pub flagged: usize,
    pub audiences: Vec<AuditedAudience>,
    /// Groups of audience IDs that share the same rule.
    pub duplicate_rule_groups: Vec<Vec<String>>,
}

fn audit_row(row: &Value) -> Option<(AuditedAudience, Option<String>)> {
    let id = row.get("id").and_then(numeric_value)?;
    let lower = row
        .get("approximate_count_lower_bound")
        .and_then(Value::as_i64);
    let upper = row
        .get("approximate_count_upper_bound")
        .and_then(Value::as_i64);
    let mut flags = Vec::new();
    let operation_status = status(row, "operation_status");
    let delivery_status = status(row, "delivery_status");
    match delivery_status.as_ref().map(|s| s.code) {
        Some(DELIVERY_READY) | None => {}
        Some(DELIVERY_TOO_SMALL) => flags.push("too_small_to_deliver".to_owned()),
        Some(_) => flags.push("delivery_issue".to_owned()),
    }
    match upper {
        Some(size) if size < 0 => flags.push("size_pending".to_owned()),
        Some(size) if size < SMALL_FOR_SEED => {
            flags.push("too_small_for_lookalike_seed".to_owned());
        }
        _ => {}
    }
    match operation_status.as_ref().map(|s| s.code) {
        Some(OPERATION_NORMAL) | None => {}
        // Meta is still filling the audience; it can already deliver.
        Some(OPERATION_POPULATING) => flags.push("populating".to_owned()),
        Some(_) => flags.push("needs_attention".to_owned()),
    }
    let description = row
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let (set_name, set_key) = parse_marker(description)
        .map(|(set, key)| (Some(set.to_owned()), Some(key.to_owned())))
        .unwrap_or_default();
    let rule_hash = row.get("rule").map(|rule| {
        let text = match rule {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        let digest = Sha256::digest(text.as_bytes());
        digest.iter().take(12).fold(String::new(), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
    });
    Some((
        AuditedAudience {
            custom_audience_id: id,
            name: row
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .chars()
                .take(MAX_NAME_CHARS)
                .collect(),
            subtype: row
                .get("subtype")
                .and_then(Value::as_str)
                .map(str::to_owned),
            set_name,
            set_key,
            size_lower_bound: lower,
            size_upper_bound: upper,
            operation_status,
            delivery_status,
            flags,
        },
        rule_hash,
    ))
}

pub(crate) async fn audit_audiences(
    graph: &GraphClient,
    input: AuditAudiencesInput,
) -> ToolResponse<AudienceAudit> {
    let account_id = match account(&input.ad_account_id) {
        Ok(account_id) => account_id,
        Err(error) => return ToolResponse::error(error),
    };
    let endpoint = format!("{account_id}/customaudiences");
    let mut rows = Vec::new();
    let mut after: Option<String> = None;
    let mut complete = false;
    for _ in 0..MAX_LIST_PAGES {
        let mut query = vec![
            ("fields".to_owned(), AUDIT_FIELDS.to_owned()),
            ("limit".to_owned(), "100".to_owned()),
        ];
        if let Some(cursor) = &after {
            query.push(("after".to_owned(), cursor.clone()));
        }
        let payload = match graph.get_json(&endpoint, &query).await {
            Ok(payload) => payload,
            Err(error) => return ToolResponse::error(error),
        };
        let Some(data) = payload.get("data").and_then(Value::as_array) else {
            return ToolResponse::error(invalid_provider(
                "Meta returned an unexpected audience list",
            ));
        };
        rows.extend(data.iter().filter_map(audit_row));
        let next = payload.pointer("/paging/next").is_some();
        after = payload
            .pointer("/paging/cursors/after")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if !next || after.is_none() {
            complete = true;
            break;
        }
    }
    if !complete {
        return ToolResponse::error(invalid_provider(
            "The account has more audiences than this tool reads in one call",
        ));
    }
    if let Some(set) = input.set_name.as_deref() {
        rows.retain(|(audience, _)| audience.set_name.as_deref() == Some(set));
    }
    let mut by_rule: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (audience, hash) in &rows {
        if let Some(hash) = hash {
            by_rule
                .entry(hash.clone())
                .or_default()
                .push(audience.custom_audience_id.clone());
        }
    }
    let duplicate_rule_groups = by_rule.into_values().filter(|ids| ids.len() > 1).collect();
    let audiences = rows
        .into_iter()
        .map(|(audience, _)| audience)
        .collect::<Vec<_>>();
    ToolResponse::success(AudienceAudit {
        total: audiences.len(),
        flagged: audiences
            .iter()
            .filter(|audience| !audience.flags.is_empty())
            .count(),
        audiences,
        duplicate_rule_groups,
    })
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

#[tool_router(router = engagement_audiences_router, vis = "pub(crate)")]
impl MetaAdsServer {
    #[tool(
        name = "create_engagement_audience",
        description = "Create one engagement or website custom audience from typed rules: Instagram or Facebook Page engagement (up to 730 days) or pixel activity (up to 180 days; Purchase up to 730). Inclusions are OR-ed; exclusions leave people out. Repeated calls can create duplicates; prefer ensure_audience_set for sets.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_engagement_audience(
        &self,
        Parameters(input): Parameters<CreateEngagementAudienceInput>,
    ) -> Result<
        Json<ToolResponse<CreatedEngagementAudience>>,
        Json<ToolResponse<CreatedEngagementAudience>>,
    > {
        create_engagement_audience(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "ensure_audience_set",
        description = "Preview, then create, a set of engagement and website audiences (preset funnel_core or engagement_matrix, or your own). Skips audiences of the set that already exist and pixel events that never fired, so repeating it never duplicates. Apply with the preview's plan_token and APPLY_LIVE_META_ADS_CHANGES. Never deletes.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn ensure_audience_set(
        &self,
        Parameters(input): Parameters<EnsureAudienceSetInput>,
    ) -> Result<Json<ToolResponse<AudienceSetResult>>, Json<ToolResponse<AudienceSetResult>>> {
        ensure_audience_set(&self.graph, input)
            .await
            .into_mcp_result()
    }

    #[tool(
        name = "audit_audiences",
        description = "Read the size and health of an ad account's custom audiences using Meta's status codes: flags audiences too small to deliver or to seed lookalikes, still populating, needing attention, or sharing the same rule.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn audit_audiences(
        &self,
        Parameters(input): Parameters<AuditAudiencesInput>,
    ) -> Result<Json<ToolResponse<AudienceAudit>>, Json<ToolResponse<AudienceAudit>>> {
        audit_audiences(&self.graph, input).await.into_mcp_result()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::{Value, json};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    use super::*;
    use crate::config::MetaConfig;

    const NOW: u64 = 1_800_000_000;

    fn rule(value: Value) -> AudienceRule {
        serde_json::from_value(value).unwrap()
    }

    fn rule_json(rules: &[AudienceRule], exclusions: &[AudienceRule]) -> Value {
        serde_json::from_str(&encode_rule(rules, exclusions).unwrap()).unwrap()
    }

    #[test]
    fn builds_exact_instagram_page_and_website_rules() {
        let encoded = rule_json(
            &[
                rule(
                    json!({"source":"instagram","instagram_account_id":"111","event":"messaged","retention_days":365}),
                ),
                rule(
                    json!({"source":"facebook_page","page_id":"222","event":"saved","retention_days":730}),
                ),
            ],
            &[],
        );
        assert_eq!(
            encoded,
            json!({"inclusions":{"operator":"or","rules":[
                {"event_sources":[{"id":"111","type":"ig_business"}],"retention_seconds":31_536_000,
                 "filter":{"operator":"and","filters":[{"field":"event","operator":"eq","value":"ig_business_profile_user_messaged"}]}},
                {"event_sources":[{"id":"222","type":"page"}],"retention_seconds":63_072_000,
                 "filter":{"operator":"and","filters":[{"field":"event","operator":"eq","value":"page_or_post_save"}]}}
            ]}})
        );

        let website = rule_json(
            &[rule(
                json!({"source":"website","pixel_id":"333","event":"top_time_spent","top_percent":10,"retention_days":180}),
            )],
            &[rule(
                json!({"source":"website","pixel_id":"333","event":"purchase","retention_days":730}),
            )],
        );
        assert_eq!(
            website["inclusions"]["rules"][0],
            json!({"event_sources":[{"id":"333","type":"pixel"}],"retention_seconds":15_552_000,
                   "filter":{"operator":"and","filters":[{"field":"url","operator":"i_contains","value":""}]},
                   "template":"TOP_TIME_SPENDERS",
                   "aggregation":{"type":"time_spent","method":"percentile","operator":"in_range","value":{"from":90,"to":100}}})
        );
        assert_eq!(
            website["exclusions"]["rules"][0]["filter"]["filters"][0]["value"],
            "Purchase"
        );

        let url = rule_json(
            &[rule(
                json!({"source":"website","pixel_id":"333","event":"url_contains","url_contains":"/obrigado","retention_days":30}),
            )],
            &[],
        );
        assert_eq!(
            url["inclusions"]["rules"][0]["filter"]["filters"][0]["value"],
            "/obrigado"
        );
    }

    #[test]
    fn enforces_retention_caps_per_source() {
        let ok = [
            json!({"source":"instagram","instagram_account_id":"1","event":"all","retention_days":730}),
            json!({"source":"facebook_page","page_id":"1","event":"engaged","retention_days":730}),
            json!({"source":"website","pixel_id":"1","event":"purchase","retention_days":730}),
            json!({"source":"website","pixel_id":"1","event":"lead","retention_days":180}),
        ];
        for value in ok {
            assert!(encode_rule(&[rule(value.clone())], &[]).is_ok(), "{value}");
        }
        let rejected = [
            json!({"source":"instagram","instagram_account_id":"1","event":"all","retention_days":731}),
            json!({"source":"facebook_page","page_id":"1","event":"engaged","retention_days":0}),
            json!({"source":"website","pixel_id":"1","event":"lead","retention_days":181}),
            json!({"source":"website","pixel_id":"1","event":"all_visitors","retention_days":365}),
            json!({"source":"website","pixel_id":"1","event":"purchase","retention_days":731}),
            json!({"source":"website","pixel_id":"1","event":"url_contains","retention_days":30}),
            json!({"source":"website","pixel_id":"1","event":"top_time_spent","top_percent":50,"retention_days":30}),
            json!({"source":"website","pixel_id":"1","event":"lead","event_name":"X","retention_days":30}),
            json!({"source":"website","pixel_id":"1","event":"custom_event","event_name":"bad name","retention_days":30}),
            json!({"source":"website","pixel_id":"abc","event":"lead","retention_days":30}),
        ];
        for value in rejected {
            assert!(encode_rule(&[rule(value.clone())], &[]).is_err(), "{value}");
        }
        assert!(
            serde_json::from_value::<AudienceRule>(
                json!({"source":"instagram","instagram_account_id":"1","event":"all","retention_days":30,"extra":1})
            )
            .is_err()
        );
        assert!(encode_rule(&[], &[]).is_err());
    }

    fn preset_input(preset: &str) -> EnsureAudienceSetInput {
        serde_json::from_value(json!({
            "ad_account_id": "act_9",
            "preset": preset,
            "instagram_account_id": "111",
            "page_id": "222",
            "pixel_id": "333",
            "name_prefix": "[Client] "
        }))
        .unwrap()
    }

    #[test]
    fn funnel_core_builds_twelve_audiences_and_skips_missing_assets() {
        let (items, skipped) = desired_audiences(&preset_input("funnel_core")).unwrap();
        let keys = items
            .iter()
            .map(|item| item.key.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            [
                "eng_30",
                "eng_90",
                "eng_365",
                "eng_730",
                "msg_365",
                "site_30",
                "site_180",
                "top25_180",
                "checkout_30_ex_purchase",
                "lead_180",
                "purchase_730",
                "hot_all_30"
            ]
        );
        assert!(skipped.is_empty());
        assert_eq!(items[0].name, "[Client] Engaged IG+FB – 30D");
        assert_eq!(items[0].inclusions.len(), 2);
        assert_eq!(items[8].exclusions.len(), 1);
        assert_eq!(items[11].inclusions.len(), 3);
        for item in &items {
            encode_rule(&item.inclusions, &item.exclusions).unwrap();
        }

        let mut only_ig = preset_input("funnel_core");
        only_ig.page_id = None;
        only_ig.pixel_id = None;
        only_ig.label_language = Some(LabelLanguage::PtBr);
        let (items, skipped) = desired_audiences(&only_ig).unwrap();
        assert_eq!(items.len(), 5);
        assert_eq!(items[0].name, "[Client] Engajou IG – 30D");
        assert!(skipped.iter().any(|skip| skip.key == "purchase_730"));
    }

    #[test]
    fn engagement_matrix_respects_each_source_limit() {
        let (items, _) = desired_audiences(&preset_input("engagement_matrix")).unwrap();
        // IG 5x9 + FB 5x9 + site (all + 3 top-time) x7 + 4 events x7 + Purchase x9
        assert_eq!(items.len(), 45 + 45 + 28 + 28 + 9);
        assert!(items.iter().any(|item| item.key == "px_purchase_730"));
        assert!(!items.iter().any(|item| item.key == "px_lead_365"));
        assert!(!items.iter().any(|item| item.key == "site_all_365"));
        for item in &items {
            encode_rule(&item.inclusions, &item.exclusions).unwrap();
        }

        let mut custom_windows = preset_input("engagement_matrix");
        custom_windows.retention_days = Some(vec![30, 365, 30]);
        let (items, _) = desired_audiences(&custom_windows).unwrap();
        assert_eq!(items.len(), 10 + 10 + 4 + 4 + 2);
    }

    #[test]
    fn silent_pixel_events_drop_rules_or_skip_audiences() {
        let (items, _) = desired_audiences(&preset_input("funnel_core")).unwrap();
        let fired = BTreeMap::from([("Lead".to_owned(), 4_u64), ("PageView".to_owned(), 90)]);
        let (kept, skipped) = apply_pixel_activity(items, &fired);
        let lead = kept.iter().find(|item| item.key == "lead_180").unwrap();
        assert_eq!(lead.inclusions.len(), 1);
        assert!(
            skipped
                .iter()
                .any(|skip| skip.key == "checkout_30_ex_purchase")
        );
        assert!(skipped.iter().any(|skip| skip.key == "purchase_730"));
        // Visitors, time-on-site and engagement audiences never depend on events.
        assert!(kept.iter().any(|item| item.key == "site_30"));
        assert!(kept.iter().any(|item| item.key == "top25_180"));
    }

    fn existing(id: &str, name: &str, description: &str) -> ExistingAudience {
        ExistingAudience {
            id: id.to_owned(),
            name: name.to_owned(),
            description: description.to_owned(),
        }
    }

    #[test]
    fn plan_skips_marked_or_same_name_audiences_and_tokens_track_changes() {
        let (items, _) = desired_audiences(&preset_input("funnel_core")).unwrap();
        let current = [
            existing(
                "1",
                "Renamed by hand",
                "notes\narmavita:set=funnel_core;key=eng_30",
            ),
            existing("2", "[Client] Website visitors – 30D", ""),
            existing("3", "Other set", "armavita:set=other;key=eng_90"),
        ];
        let plan = plan_audiences("act_9", "funnel_core", items.clone(), &current).unwrap();
        assert_eq!(plan.existing.len(), 2);
        assert_eq!(plan.existing[0].matched_by, "marker");
        assert_eq!(plan.existing[1].matched_by, "name");
        assert_eq!(plan.to_create.len(), 10);
        assert_eq!(plan.token.len(), PLAN_TOKEN_CHARS);

        let again = plan_audiences("act_9", "funnel_core", items.clone(), &current).unwrap();
        assert_eq!(plan.token, again.token);
        let fewer = plan_audiences("act_9", "funnel_core", items.clone(), &current[..1]).unwrap();
        assert_ne!(plan.token, fewer.token);

        let mut duplicated = items;
        duplicated.push(duplicated[0].clone());
        assert!(plan_audiences("act_9", "funnel_core", duplicated, &[]).is_err());
    }

    #[test]
    fn markers_round_trip() {
        let text = format!("hello\n{}", marker("funnel_core", "eng_30"));
        assert_eq!(parse_marker(&text), Some(("funnel_core", "eng_30")));
        assert_eq!(parse_marker("no marker"), None);
    }

    #[test]
    fn audit_flags_follow_meta_status_codes() {
        let (small, _) = audit_row(&json!({
            "id":"1","name":"Small","approximate_count_lower_bound":10,"approximate_count_upper_bound":50,
            "operation_status":{"code":200},"delivery_status":{"code":300,"description":"too small"},
            "description":"armavita:set=funnel_core;key=eng_30"
        }))
        .unwrap();
        assert_eq!(
            small.flags,
            ["too_small_to_deliver", "too_small_for_lookalike_seed"]
        );
        assert_eq!(small.set_key.as_deref(), Some("eng_30"));
        assert_eq!(small.delivery_status.as_ref().unwrap().code, 300);

        // 441 = still filling but already usable: informational, not an error.
        let (filling, _) = audit_row(&json!({
            "id":"2","name":"Filling","approximate_count_upper_bound":1_000,
            "operation_status":{"code":441},"delivery_status":{"code":200}
        }))
        .unwrap();
        assert_eq!(filling.flags, ["populating"]);

        let (odd, _) = audit_row(&json!({
            "id":"3","name":"New","approximate_count_upper_bound":-1,
            "operation_status":{"code":422},"delivery_status":{"code":400}
        }))
        .unwrap();
        assert_eq!(
            odd.flags,
            ["delivery_issue", "size_pending", "needs_attention"]
        );

        let (healthy, _) = audit_row(&json!({
            "id":"4","name":"Big","approximate_count_upper_bound":800,
            "operation_status":{"code":200},"delivery_status":{"code":200}
        }))
        .unwrap();
        assert!(healthy.flags.is_empty());
    }

    // ---- HTTP flow ----------------------------------------------------------

    async fn scripted_graph(
        responses: Vec<(u16, &'static str)>,
    ) -> (GraphClient, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut requests = Vec::with_capacity(responses.len());
            for (status, body) in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                requests.push(read_request(&mut socket).await);
                let reason = if status == 200 { "OK" } else { "Bad Request" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(200), listener.accept())
                    .await
                    .is_err(),
                "unexpected extra request"
            );
            requests
        });
        let graph = GraphClient::new(&MetaConfig::for_test(
            format!("http://{address}"),
            Some("test-access-token-1234567890"),
        ))
        .unwrap();
        (graph, server)
    }

    async fn read_request(socket: &mut TcpStream) -> String {
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = socket.read(&mut buffer).await.unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
            let length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if request.len() >= end + 4 + length {
                break;
            }
        }
        String::from_utf8(request).unwrap()
    }

    fn custom_set(extra: Value) -> EnsureAudienceSetInput {
        let mut value = json!({
            "ad_account_id": "9",
            "set_name": "client_a",
            "audiences": [
                {"key":"ig_30","name":"IG 30","inclusions":[{"source":"instagram","instagram_account_id":"111","event":"all","retention_days":30}]},
                {"key":"fb_30","name":"FB 30","inclusions":[{"source":"facebook_page","page_id":"222","event":"engaged","retention_days":30}]},
                {"key":"ig_90","name":"IG 90","inclusions":[{"source":"instagram","instagram_account_id":"111","event":"all","retention_days":90}]}
            ]
        });
        for (key, field) in extra.as_object().unwrap() {
            value[key] = field.clone();
        }
        serde_json::from_value(value).unwrap()
    }

    const LIST_WITH_IG_30: &str = r#"{"data":[{"id":"501","name":"IG 30 renamed","description":"armavita:set=client_a;key=ig_30"}]}"#;

    #[tokio::test]
    async fn previews_then_applies_only_missing_audiences() {
        let (graph, server) = scripted_graph(vec![(200, LIST_WITH_IG_30)]).await;
        let preview = ensure_audience_set_inner(&graph, custom_set(json!({})), NOW, Duration::ZERO)
            .await
            .unwrap();
        assert!(preview.dry_run);
        assert_eq!(preview.existing.len(), 1);
        assert_eq!(
            preview
                .to_create
                .iter()
                .map(|item| item.key.as_str())
                .collect::<Vec<_>>(),
            ["fb_30", "ig_90"]
        );
        let token = preview.plan_token.clone().unwrap();
        let requests = server.await.unwrap();
        assert!(
            requests[0].starts_with(
                "GET /act_9/customaudiences?fields=id%2Cname%2Cdescription&limit=200 "
            )
        );

        let (graph, server) = scripted_graph(vec![
            (200, LIST_WITH_IG_30),
            (200, r#"{"id":"601"}"#),
            (200, r#"{"id":"602"}"#),
        ])
        .await;
        let applied = ensure_audience_set_inner(
            &graph,
            custom_set(
                json!({"plan_token": token, "apply_acknowledgement": "APPLY_LIVE_META_ADS_CHANGES"}),
            ),
            NOW,
            Duration::ZERO,
        )
        .await
        .unwrap();
        assert!(!applied.dry_run);
        assert_eq!(
            applied
                .created
                .iter()
                .map(|item| item.custom_audience_id.as_str())
                .collect::<Vec<_>>(),
            ["601", "602"]
        );
        assert!(applied.failed.is_none());
        let requests = server.await.unwrap();
        assert!(requests[1].starts_with("POST /act_9/customaudiences "));
        assert!(requests[1].contains("name=FB+30"));
        assert!(requests[1].contains("prefill=true"));
        assert!(requests[1].contains("armavita%3Aset%3Dclient_a%3Bkey%3Dfb_30"));
        assert!(requests[1].contains("page_engaged"));
        let body = requests[1].split("\r\n\r\n").nth(1).unwrap();
        assert!(!body.contains("test-access-token"));
    }

    #[tokio::test]
    async fn refuses_to_apply_a_changed_plan_or_without_acknowledgement() {
        let (graph, server) = scripted_graph(vec![(200, r#"{"data":[]}"#)]).await;
        let error = ensure_audience_set_inner(
            &graph,
            custom_set(
                json!({"plan_token":"0000","apply_acknowledgement":"APPLY_LIVE_META_ADS_CHANGES"}),
            ),
            NOW,
            Duration::ZERO,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "PLAN_CHANGED");
        assert_eq!(server.await.unwrap().len(), 1);

        let (graph, server) = scripted_graph(vec![]).await;
        assert!(
            ensure_audience_set_inner(
                &graph,
                custom_set(json!({"apply_acknowledgement":"APPLY_LIVE_META_ADS_CHANGES"})),
                NOW,
                Duration::ZERO,
            )
            .await
            .is_err()
        );
        assert!(
            ensure_audience_set_inner(
                &graph,
                custom_set(json!({"plan_token":"x","apply_acknowledgement":"yes"})),
                NOW,
                Duration::ZERO,
            )
            .await
            .is_err()
        );
        assert!(server.await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn stops_at_the_first_failed_create_and_reports_the_rest() {
        let empty = r#"{"data":[]}"#;
        let (graph, server) = scripted_graph(vec![(200, empty)]).await;
        let token = ensure_audience_set_inner(&graph, custom_set(json!({})), NOW, Duration::ZERO)
            .await
            .unwrap()
            .plan_token
            .unwrap();
        server.await.unwrap();

        let (graph, server) = scripted_graph(vec![
            (200, empty),
            (200, r#"{"id":"601"}"#),
            (
                400,
                r#"{"error":{"code":100,"message":"Invalid parameter"}}"#,
            ),
        ])
        .await;
        let result = ensure_audience_set_inner(
            &graph,
            custom_set(
                json!({"plan_token": token, "apply_acknowledgement": "APPLY_LIVE_META_ADS_CHANGES"}),
            ),
            NOW,
            Duration::ZERO,
        )
        .await
        .unwrap();
        assert_eq!(result.created.len(), 1);
        assert_eq!(result.failed.as_ref().unwrap().key, "fb_30");
        assert_eq!(result.not_attempted, ["ig_90"]);
        assert_eq!(server.await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn preset_preview_reads_pixel_activity_and_skips_silent_events() {
        let stats = r#"{"data":[{"aggregation":"event","data":[{"value":"PageView","count":50},{"value":"Lead","count":3}]}]}"#;
        let (graph, server) = scripted_graph(vec![(200, stats), (200, r#"{"data":[]}"#)]).await;
        let result =
            ensure_audience_set_inner(&graph, preset_input("funnel_core"), NOW, Duration::ZERO)
                .await
                .unwrap();
        assert_eq!(result.pixel_events_checked, Some(true));
        assert!(result.skipped.iter().any(|skip| skip.key == "purchase_730"));
        assert!(result.to_create.iter().any(|item| item.key == "lead_180"));
        let requests = server.await.unwrap();
        assert!(requests[0].starts_with(&format!(
            "GET /333/stats?aggregation=event&start_time={}&limit=1000 ",
            NOW - 30 * 86_400
        )));
    }

    #[tokio::test]
    async fn unreadable_pixel_stats_only_warn() {
        let (graph, server) = scripted_graph(vec![
            (
                400,
                r#"{"error":{"code":200,"message":"Permissions error"}}"#,
            ),
            (200, r#"{"data":[]}"#),
        ])
        .await;
        let result =
            ensure_audience_set_inner(&graph, preset_input("funnel_core"), NOW, Duration::ZERO)
                .await
                .unwrap();
        assert_eq!(result.pixel_events_checked, Some(false));
        assert_eq!(result.to_create.len(), 12);
        assert_eq!(result.warnings.len(), 1);
        server.await.unwrap();
    }
}
