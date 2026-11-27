// SPDX-License-Identifier: AGPL-3.0-only

//! Optional tool groups selected with `META_ADS_TOOLSETS`, so clients that need
//! only part of the server load a smaller tool catalog.

use std::collections::BTreeSet;

pub(crate) const TOOLSETS_ENV: &str = "META_ADS_TOOLSETS";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Toolset {
    /// Accounts, campaigns, ad sets, ads, creatives, media, Pages, targeting
    /// search, comments, extensions, labels, budgets and mutation plans.
    Core,
    Insights,
    Audiences,
    Catalogs,
    Measurement,
    Leads,
    Business,
    Automation,
    Planning,
}

impl Toolset {
    pub(crate) const ALL: [Self; 9] = [
        Self::Core,
        Self::Insights,
        Self::Audiences,
        Self::Catalogs,
        Self::Measurement,
        Self::Leads,
        Self::Business,
        Self::Automation,
        Self::Planning,
    ];

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Insights => "insights",
            Self::Audiences => "audiences",
            Self::Catalogs => "catalogs",
            Self::Measurement => "measurement",
            Self::Leads => "leads",
            Self::Business => "business",
            Self::Automation => "automation",
            Self::Planning => "planning",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|set| set.name() == raw)
    }
}

/// The group a tool belongs to. Every registered tool must be listed here; a
/// test enforces it, so a new tool cannot silently escape the filter.
pub(crate) fn toolset_of(tool: &str) -> Option<Toolset> {
    use Toolset::*;
    Some(match tool {
        "list_ad_accounts"
        | "read_ad_account"
        | "get_account_controls"
        | "update_account_controls"
        | "list_campaigns"
        | "read_campaign"
        | "create_campaign"
        | "update_campaign"
        | "clone_campaign"
        | "list_ad_sets"
        | "read_ad_set"
        | "create_ad_set"
        | "update_ad_set"
        | "clone_ad_set"
        | "list_ads"
        | "read_ad"
        | "create_ad"
        | "update_ad"
        | "clone_ad"
        | "list_ad_creatives"
        | "read_ad_creative"
        | "create_ad_creative"
        | "update_ad_creative"
        | "list_ad_previews"
        | "generate_creative_previews"
        | "list_ad_images"
        | "list_ad_videos"
        | "read_ad_image"
        | "upload_ad_image_asset"
        | "upload_ad_video_asset"
        | "upload_page_ad_media"
        | "delete_ad_media"
        | "list_pages"
        | "read_ad_assets"
        | "inspect_ad_object"
        | "read_ad_inventory"
        | "list_recommendations"
        | "manage_ad_label"
        | "change_ad_labels"
        | "build_mutation_plan"
        | "get_mutation_plan"
        | "apply_mutation_plan"
        | "discard_mutation_plan"
        | "create_campaign_budget_schedule"
        | "read_budget_schedules"
        | "manage_budget_schedules"
        | "read_ad_comments"
        | "manage_ad_comment"
        | "read_ad_extensions"
        | "manage_ad_extensions"
        | "read_instant_experiences"
        | "manage_instant_experiences"
        | "search_interests"
        | "suggest_interests"
        | "search_behaviors"
        | "search_demographics"
        | "search_geo_locations"
        | "estimate_audience_size"
        | "query_targeting"
        | "inspect_access_token" => Core,
        "list_insights"
        | "create_insights_job"
        | "read_insights_job"
        | "read_insights_job_results"
        | "create_report"
        | "list_ad_custom_derived_metrics" => Insights,
        "list_custom_audiences"
        | "read_custom_audience"
        | "create_custom_audience"
        | "update_custom_audience"
        | "delete_custom_audience"
        | "manage_custom_audience_users"
        | "create_lookalike_audience"
        | "share_custom_audience"
        | "create_engagement_audience"
        | "ensure_audience_set"
        | "audit_audiences" => Audiences,
        "list_product_catalogs"
        | "list_products"
        | "list_product_sets"
        | "upsert_product"
        | "batch_products"
        | "read_catalog_object"
        | "list_catalog_resources"
        | "write_catalog_object"
        | "delete_catalog_object"
        | "manage_catalog_connection"
        | "upload_catalog_feed"
        | "batch_catalog_items"
        | "read_catalog_batch_status"
        | "configure_catalog_advertising"
        | "send_marketplace_signal"
        | "create_feed_error_report" => Catalogs,
        "send_capi_events"
        | "list_business_datasets"
        | "read_dataset_quality"
        | "list_custom_conversions"
        | "read_custom_conversion"
        | "create_custom_conversion"
        | "update_custom_conversion"
        | "delete_custom_conversion" => Measurement,
        "read_lead_data"
        | "create_lead_form"
        | "set_lead_form_status"
        | "create_test_lead"
        | "delete_test_lead"
        | "manage_lead_subscription" => Leads,
        "read_business_asset"
        | "list_business_assets"
        | "create_business_asset"
        | "update_business_asset"
        | "manage_business_asset_access"
        | "delete_business_asset"
        | "list_branded_content_ad_permissions"
        | "grant_branded_content_ad_permission"
        | "revoke_branded_content_ad_permission"
        | "get_threads_account"
        | "create_threads_account"
        | "create_page_backed_instagram_account" => Business,
        "read_ad_rules" | "manage_ad_rules" | "read_ad_studies" | "manage_ad_studies"
        | "read_value_rules" | "manage_value_rules" => Automation,
        "read_reach_frequency_prediction"
        | "list_reach_frequency_predictions"
        | "create_reach_frequency_prediction"
        | "search_ads_archive" => Planning,
        _ => return None,
    })
}

/// Parse `META_ADS_TOOLSETS`. `None` (unset, empty or `all`) keeps every tool.
/// `core` is always included. Unknown names are an error naming the bad entry.
pub(crate) fn parse_toolsets(raw: Option<&str>) -> Result<Option<BTreeSet<Toolset>>, String> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(None);
    };
    let mut selected = BTreeSet::from([Toolset::Core]);
    for entry in raw
        .split(',')
        .map(|entry| entry.trim().to_ascii_lowercase())
    {
        if entry.is_empty() {
            continue;
        }
        if entry == "all" {
            return Ok(None);
        }
        let set =
            Toolset::parse(&entry).ok_or_else(|| entry.chars().take(40).collect::<String>())?;
        selected.insert(set);
    }
    Ok(Some(selected))
}

pub(crate) fn toolset_names() -> String {
    Toolset::ALL.map(Toolset::name).join(", ")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{Toolset, parse_toolsets, toolset_of};

    #[test]
    fn parses_lists_all_and_always_keeps_core() {
        assert_eq!(parse_toolsets(None), Ok(None));
        assert_eq!(parse_toolsets(Some("  ")), Ok(None));
        assert_eq!(parse_toolsets(Some("insights,ALL")), Ok(None));
        assert_eq!(
            parse_toolsets(Some(" Insights , audiences,")),
            Ok(Some(BTreeSet::from([
                Toolset::Core,
                Toolset::Insights,
                Toolset::Audiences
            ])))
        );
        assert_eq!(
            parse_toolsets(Some("core")),
            Ok(Some(BTreeSet::from([Toolset::Core])))
        );
        assert_eq!(
            parse_toolsets(Some("insights,whatsapp")),
            Err("whatsapp".into())
        );
    }

    #[test]
    fn unknown_tools_have_no_toolset() {
        assert_eq!(toolset_of("list_campaigns"), Some(Toolset::Core));
        assert_eq!(toolset_of("send_capi_events"), Some(Toolset::Measurement));
        assert_eq!(toolset_of("not_a_tool"), None);
    }
}
