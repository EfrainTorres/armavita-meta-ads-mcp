use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    insights::{
        ActionMetric, InsightDatePreset, InsightLevel, InsightRow, InsightTimeRange,
        ListInsightsInput, list_insights,
    },
};

const DEFAULT_REPORT_NAME: &str = "Meta Ads performance report";
const MAX_REPORT_NAME_CHARS: usize = 120;
const MAX_TOP_ACTIONS: usize = 8;
const REPORT_FIELDS: [&str; 12] = [
    "date_start",
    "date_stop",
    "impressions",
    "clicks",
    "spend",
    "cpc",
    "cpm",
    "ctr",
    "reach",
    "frequency",
    "unique_clicks",
    "actions",
];

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReportScope {
    Account,
    Campaign,
    AdSet,
    Ad,
}

impl ReportScope {
    const fn insight_level(self) -> InsightLevel {
        match self {
            Self::Account => InsightLevel::Account,
            Self::Campaign => InsightLevel::Campaign,
            Self::AdSet => InsightLevel::AdSet,
            Self::Ad => InsightLevel::Ad,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportPeriod {
    /// Meta-defined relative date window. Defaults to `last_30d` for the current period.
    pub date_preset: Option<InsightDatePreset>,
    /// Inclusive calendar dates; mutually exclusive with `date_preset`.
    pub time_range: Option<InsightTimeRange>,
}

impl ReportPeriod {
    fn last_30_days() -> Self {
        Self {
            date_preset: Some(InsightDatePreset::Last30d),
            time_range: None,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateReportInput {
    /// Numeric Meta account, campaign, ad-set, or ad ID. Account IDs may use `act_`.
    pub object_id: String,
    /// Aggregation scope matching `object_id`.
    pub scope: ReportScope,
    /// Short label returned with the report; maximum 120 characters.
    pub report_name: Option<String>,
    /// Current reporting period. Defaults to `last_30d`.
    pub current_period: Option<ReportPeriod>,
    /// Optional second period returned separately for direct comparison.
    pub comparison_period: Option<ReportPeriod>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PerformanceReport {
    pub name: String,
    pub scope: ReportScope,
    pub current: ReportSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparison: Option<ReportSnapshot>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ReportSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_stop: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impressions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clicks: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unique_clicks: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ctr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpm: Option<String>,
    /// Meta's one-row reach value. This server never sums reach across rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reach: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ReportAction>,
    #[serde(skip_serializing_if = "is_zero")]
    pub actions_omitted: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ReportAction {
    pub action_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

pub(crate) async fn create_report(
    graph: &GraphClient,
    input: CreateReportInput,
) -> ToolResponse<PerformanceReport> {
    let name = match normalize_name(input.report_name.as_deref()) {
        Ok(name) => name,
        Err(error) => return ToolResponse::error(error),
    };
    let current = match fetch_snapshot(
        graph,
        &input.object_id,
        input.scope,
        input
            .current_period
            .unwrap_or_else(ReportPeriod::last_30_days),
    )
    .await
    {
        Ok(snapshot) => snapshot,
        Err(error) => return ToolResponse::Error { error },
    };
    let comparison = if let Some(period) = input.comparison_period {
        match fetch_snapshot(graph, &input.object_id, input.scope, period).await {
            Ok(snapshot) => Some(snapshot),
            Err(error) => return ToolResponse::Error { error },
        }
    } else {
        None
    };

    ToolResponse::success(PerformanceReport {
        name,
        scope: input.scope,
        current,
        comparison,
    })
}

async fn fetch_snapshot(
    graph: &GraphClient,
    object_id: &str,
    scope: ReportScope,
    period: ReportPeriod,
) -> Result<ReportSnapshot, PublicError> {
    let response = list_insights(
        graph,
        ListInsightsInput {
            object_id: object_id.to_owned(),
            level: Some(scope.insight_level()),
            date_preset: period.date_preset,
            time_range: period.time_range,
            time_increment: None,
            fields: Some(
                REPORT_FIELDS
                    .iter()
                    .map(|field| (*field).to_owned())
                    .collect(),
            ),
            breakdowns: None,
            action_breakdowns: None,
            summary_action_breakdowns: None,
            action_attribution_windows: None,
            page_size: Some(1),
            page_cursor: None,
        },
    )
    .await;

    let page = match response {
        ToolResponse::Success { data } => data,
        ToolResponse::Error { error } => return Err(error),
    };
    let mut rows = page.rows.into_iter();
    let row = rows.next().ok_or_else(|| {
        PublicError::invalid_upstream("Meta returned no aggregate row for the report period")
    })?;
    if rows.next().is_some() || page.next_cursor.is_some() {
        return Err(PublicError::invalid_upstream(
            "Meta returned multiple rows for an unbroken aggregate report",
        ));
    }
    Ok(snapshot_from_row(row))
}

fn normalize_name(name: Option<&str>) -> Result<String, PublicError> {
    let name = name.unwrap_or(DEFAULT_REPORT_NAME).trim();
    if name.is_empty()
        || name.chars().count() > MAX_REPORT_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(PublicError::invalid_input(
            "report_name must contain 1 to 120 printable characters",
            "Use a short plain-text report label",
        ));
    }
    Ok(name.to_owned())
}

fn snapshot_from_row(row: InsightRow) -> ReportSnapshot {
    let actions = row.actions.unwrap_or_default();
    let actions_omitted = actions.len().saturating_sub(MAX_TOP_ACTIONS);
    let actions = actions
        .into_iter()
        .take(MAX_TOP_ACTIONS)
        .map(action_for_report)
        .collect();
    ReportSnapshot {
        date_start: row.date_start,
        date_stop: row.date_stop,
        spend: row.spend,
        impressions: row.impressions,
        clicks: row.clicks,
        unique_clicks: row.unique_clicks,
        ctr: row.ctr,
        cpc: row.cpc,
        cpm: row.cpm,
        reach: row.reach,
        frequency: row.frequency,
        actions,
        actions_omitted,
    }
}

fn action_for_report(action: ActionMetric) -> ReportAction {
    ReportAction {
        action_type: action.action_type,
        value: action.value,
    }
}

const fn is_zero(value: &usize) -> bool {
    *value == 0
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{MAX_TOP_ACTIONS, normalize_name, snapshot_from_row};
    use crate::insights::{ActionMetric, InsightRow};

    fn row_with_actions(count: usize) -> InsightRow {
        InsightRow {
            account_id: None,
            account_name: None,
            campaign_id: None,
            campaign_name: None,
            adset_id: None,
            adset_name: None,
            ad_id: None,
            ad_name: None,
            date_start: Some("2026-08-01".to_owned()),
            date_stop: Some("2026-08-19".to_owned()),
            impressions: Some("100".to_owned()),
            clicks: Some("10".to_owned()),
            spend: Some("12.34".to_owned()),
            cpc: Some("1.234".to_owned()),
            cpm: Some("123.4".to_owned()),
            ctr: Some("10".to_owned()),
            reach: Some("80".to_owned()),
            frequency: Some("1.25".to_owned()),
            unique_clicks: Some("9".to_owned()),
            actions: Some(
                (0..count)
                    .map(|index| ActionMetric {
                        action_type: format!("action_{index}"),
                        value: Some(index.to_string()),
                        detail: BTreeMap::new(),
                    })
                    .collect(),
            ),
            action_values: None,
            conversions: None,
            cost_per_action_type: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn preserves_one_row_metrics_and_bounds_actions() {
        let snapshot = snapshot_from_row(row_with_actions(MAX_TOP_ACTIONS + 3));
        assert_eq!(snapshot.reach.as_deref(), Some("80"));
        assert_eq!(snapshot.actions.len(), MAX_TOP_ACTIONS);
        assert_eq!(snapshot.actions_omitted, 3);
    }

    #[test]
    fn report_names_are_small_plain_text() {
        assert_eq!(
            normalize_name(Some("  Weekly paid media  ")).unwrap(),
            "Weekly paid media"
        );
        assert!(normalize_name(Some("")).is_err());
        assert!(normalize_name(Some("bad\nname")).is_err());
        assert!(normalize_name(Some(&"x".repeat(121))).is_err());
    }
}
