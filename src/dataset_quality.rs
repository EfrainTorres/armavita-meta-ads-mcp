use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::{PublicError, ToolResponse},
    graph::GraphClient,
    meta_ids::{numeric_owned as normalize_numeric_id, numeric_value as normalize_numeric_value},
};

// Verified against Meta's generated v26 Business `ads_dataset` edge and AdsDataset fields.
// Provider configuration, event_stats, ownership objects, PII flags, and paging stay private.
const DATASET_QUALITY_FIELDS: &str = "id,name,collection_rate,match_rate_approx,matched_entries,valid_entries,duplicate_entries,upload_rate,event_time_min,event_time_max,last_fired_time,last_upload_time,is_unavailable";
const MAX_NAME_CHARS: usize = 256;
const MAX_TIME_CHARS: usize = 64;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadDatasetQualityInput {
    /// Numeric Meta Business ID that owns the dataset.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub business_id: String,
    /// Numeric Ads Dataset ID returned by `list_business_datasets`.
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[0-9]{1,64}$"))]
    pub dataset_id: String,
}

/// Compact Ads Dataset ingestion and matching-health summary.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DatasetQuality {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_rate_approx: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_entries: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_entries: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_entries: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_time_min: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_time_max: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fired_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_upload_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_unavailable: Option<bool>,
}

#[derive(Debug)]
struct DatasetQualityRequest {
    endpoint: String,
    query: Vec<(String, String)>,
    dataset_id: String,
}

#[derive(Debug, Deserialize)]
struct RawDatasetPage {
    data: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct RawDatasetQuality {
    id: Option<Value>,
    name: Option<String>,
    collection_rate: Option<Value>,
    match_rate_approx: Option<Value>,
    matched_entries: Option<Value>,
    valid_entries: Option<Value>,
    duplicate_entries: Option<Value>,
    upload_rate: Option<Value>,
    event_time_min: Option<Value>,
    event_time_max: Option<Value>,
    last_fired_time: Option<String>,
    last_upload_time: Option<Value>,
    is_unavailable: Option<bool>,
}

pub(crate) async fn read_dataset_quality(
    graph: &GraphClient,
    input: ReadDatasetQualityInput,
) -> ToolResponse<DatasetQuality> {
    let request = match build_request(input) {
        Ok(request) => request,
        Err(error) => return ToolResponse::error(error),
    };
    let payload = match graph.get_json(&request.endpoint, &request.query).await {
        Ok(payload) => payload,
        Err(error) => return ToolResponse::error(error),
    };
    match parse_quality(payload, &request.dataset_id) {
        Ok(quality) => ToolResponse::success(quality),
        Err(error) => ToolResponse::error(error),
    }
}

fn build_request(input: ReadDatasetQualityInput) -> Result<DatasetQualityRequest, PublicError> {
    let business_id = normalize_numeric_id(&input.business_id).ok_or_else(|| {
        PublicError::invalid_input(
            "business_id must be a numeric Meta Business ID",
            "Use a Business ID without URL or path characters",
        )
    })?;
    let dataset_id = normalize_numeric_id(&input.dataset_id).ok_or_else(|| {
        PublicError::invalid_input(
            "dataset_id must be a numeric Ads Dataset ID",
            "Use an ID returned by list_business_datasets",
        )
    })?;

    Ok(DatasetQualityRequest {
        endpoint: format!("{business_id}/ads_dataset"),
        query: vec![
            ("fields".to_owned(), DATASET_QUALITY_FIELDS.to_owned()),
            ("id_filter".to_owned(), dataset_id.clone()),
            ("limit".to_owned(), "1".to_owned()),
        ],
        dataset_id,
    })
}

fn parse_quality(payload: Value, expected_id: &str) -> Result<DatasetQuality, PublicError> {
    let mut page = serde_json::from_value::<RawDatasetPage>(payload).map_err(|_| {
        PublicError::invalid_upstream("Meta returned an unexpected Ads Dataset response")
    })?;
    match page.data.len() {
        0 => return Err(dataset_not_found()),
        1 => {}
        _ => {
            return Err(exact_filter_error(
                "Meta returned multiple datasets for one exact ID filter",
            ));
        }
    }

    let item = page.data.pop().ok_or_else(|| {
        PublicError::invalid_upstream("Meta omitted Ads Dataset quality metadata")
    })?;
    let raw = serde_json::from_value::<RawDatasetQuality>(item).map_err(|_| {
        PublicError::invalid_upstream("Meta returned malformed Ads Dataset quality metadata")
    })?;
    normalize_quality(raw, expected_id)
}

fn normalize_quality(
    raw: RawDatasetQuality,
    expected_id: &str,
) -> Result<DatasetQuality, PublicError> {
    let id = raw
        .id
        .as_ref()
        .and_then(normalize_numeric_value)
        .ok_or_else(|| PublicError::invalid_upstream("Meta omitted a valid Ads Dataset ID"))?;
    if id != expected_id {
        return Err(exact_filter_error(
            "Meta returned a dataset that did not match dataset_id",
        ));
    }

    let event_time_min = optional_integer(raw.event_time_min, "event_time_min")?;
    let event_time_max = optional_integer(raw.event_time_max, "event_time_max")?;

    Ok(DatasetQuality {
        id,
        name: optional_text(raw.name, MAX_NAME_CHARS, "name")?,
        collection_rate: optional_finite_float(raw.collection_rate, "collection_rate")?,
        match_rate_approx: optional_integer(raw.match_rate_approx, "match_rate_approx")?,
        matched_entries: optional_integer(raw.matched_entries, "matched_entries")?,
        valid_entries: optional_integer(raw.valid_entries, "valid_entries")?,
        duplicate_entries: optional_integer(raw.duplicate_entries, "duplicate_entries")?,
        upload_rate: optional_finite_float(raw.upload_rate, "upload_rate")?,
        event_time_min,
        event_time_max,
        last_fired_time: optional_text(raw.last_fired_time, MAX_TIME_CHARS, "last_fired_time")?,
        last_upload_time: optional_integer(raw.last_upload_time, "last_upload_time")?,
        is_unavailable: raw.is_unavailable,
    })
}

fn optional_integer(raw: Option<Value>, field: &str) -> Result<Option<i64>, PublicError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let parsed = match raw {
        Value::Number(value) => value.as_i64(),
        Value::String(value)
            if !value.is_empty()
                && value.len() <= 20
                && value
                    .strip_prefix('-')
                    .unwrap_or(&value)
                    .bytes()
                    .all(|byte| byte.is_ascii_digit())
                && value != "-" =>
        {
            value.parse().ok()
        }
        _ => None,
    };
    parsed.map(Some).ok_or_else(|| malformed_field(field))
}

fn optional_finite_float(raw: Option<Value>, field: &str) -> Result<Option<f64>, PublicError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let parsed = match raw {
        Value::Number(value) => value.as_f64(),
        Value::String(value) if !value.is_empty() && value.len() <= 64 => value.parse().ok(),
        _ => None,
    }
    .filter(|value: &f64| value.is_finite());
    parsed.map(Some).ok_or_else(|| malformed_field(field))
}

fn optional_text(
    raw: Option<String>,
    max_chars: usize,
    field: &str,
) -> Result<Option<String>, PublicError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let value = raw.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.chars().count() > max_chars || value.chars().any(char::is_control) {
        return Err(malformed_field(field));
    }
    Ok(Some(value.to_owned()))
}

fn malformed_field(field: &str) -> PublicError {
    PublicError::invalid_upstream(format!(
        "Meta returned an invalid Ads Dataset {field} value"
    ))
}

fn dataset_not_found() -> PublicError {
    PublicError {
        code: "DATASET_NOT_FOUND".to_owned(),
        message: "The dataset was not returned for this business".to_owned(),
        retryable: false,
        action: Some("Verify both IDs and Business access with list_business_datasets".to_owned()),
    }
}

fn exact_filter_error(message: &str) -> PublicError {
    PublicError {
        code: "INVALID_UPSTREAM_RESPONSE".to_owned(),
        message: message.to_owned(),
        retryable: false,
        action: Some(
            "Verify dataset ownership; report persistent Meta API contract drift".to_owned(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use rmcp::schemars::schema_for;
    use serde_json::{Value, json};

    use super::{
        DATASET_QUALITY_FIELDS, RawDatasetQuality, ReadDatasetQualityInput, build_request,
        normalize_quality, parse_quality,
    };

    #[test]
    fn builds_exact_business_dataset_filter_request() {
        let request = build_request(ReadDatasetQualityInput {
            business_id: " 123 ".to_owned(),
            dataset_id: " 456 ".to_owned(),
        })
        .unwrap();

        assert_eq!(request.endpoint, "123/ads_dataset");
        assert_eq!(
            request.query,
            vec![
                ("fields".to_owned(), DATASET_QUALITY_FIELDS.to_owned()),
                ("id_filter".to_owned(), "456".to_owned()),
                ("limit".to_owned(), "1".to_owned()),
            ]
        );
        assert!(!request.endpoint.contains("dataset_quality"));
        assert!(!request.query.iter().any(|(key, _)| key == "after"));

        for (business_id, dataset_id) in [
            ("../123", "456"),
            ("123", "456/path"),
            ("act_123", "456"),
            ("123", ""),
        ] {
            assert!(
                build_request(ReadDatasetQualityInput {
                    business_id: business_id.to_owned(),
                    dataset_id: dataset_id.to_owned(),
                })
                .is_err()
            );
        }
    }

    #[test]
    fn normalizes_only_compact_quality_fields() {
        let raw: RawDatasetQuality = serde_json::from_value(json!({
            "id": 456,
            "name": "  Web dataset  ",
            "collection_rate": "0.95",
            "match_rate_approx": "-1",
            "matched_entries": 900,
            "valid_entries": "1000",
            "duplicate_entries": 12,
            "upload_rate": -1.0,
            "event_time_min": "-1",
            "event_time_max": "1700003600",
            "last_fired_time": " 2026-08-19T12:00:00+0000 ",
            "last_upload_time": 1_700_003_600_u64,
            "is_unavailable": false,
            "event_stats": "provider blob",
            "config": "private config",
            "has_sent_pii": true
        }))
        .unwrap();
        let quality = normalize_quality(raw, "456").unwrap();

        assert_eq!(quality.id, "456");
        assert_eq!(quality.name.as_deref(), Some("Web dataset"));
        assert_eq!(quality.collection_rate, Some(0.95));
        assert_eq!(quality.upload_rate, Some(-1.0));
        assert_eq!(quality.match_rate_approx, Some(-1));
        assert_eq!(quality.valid_entries, Some(1_000));
        assert_eq!(quality.event_time_min, Some(-1));
        assert_eq!(quality.event_time_max, Some(1_700_003_600));
        let encoded = serde_json::to_string(&quality).unwrap();
        for private_field in [
            "event_stats",
            "provider blob",
            "private config",
            "has_sent_pii",
        ] {
            assert!(!encoded.contains(private_field));
        }
    }

    #[test]
    fn rejects_empty_mismatched_multiple_and_malformed_results() {
        let empty = parse_quality(json!({"data": []}), "456").unwrap_err();
        assert_eq!(empty.code, "DATASET_NOT_FOUND");
        assert!(!empty.retryable);
        assert!(empty.action.unwrap().contains("list_business_datasets"));

        let mismatch = parse_quality(json!({"data": [{"id": "999"}]}), "456").unwrap_err();
        assert_eq!(mismatch.code, "INVALID_UPSTREAM_RESPONSE");
        assert!(mismatch.message.contains("did not match"));
        assert!(!mismatch.retryable);

        let multiple =
            parse_quality(json!({"data": [{"id": "456"}, {"id": "456"}]}), "456").unwrap_err();
        assert!(multiple.message.contains("multiple"));

        for payload in [
            json!({}),
            json!({"data": [{"id": "456", "matched_entries": "not-an-int"}]}),
            json!({"data": [{"id": "456", "collection_rate": "NaN"}]}),
            json!({"data": [{"id": "456", "event_time_min": 1.5}]}),
        ] {
            assert_eq!(
                parse_quality(payload, "456").unwrap_err().code,
                "INVALID_UPSTREAM_RESPONSE"
            );
        }
    }

    #[test]
    fn input_schema_is_closed_numeric_and_small() {
        let schema = serde_json::to_value(schema_for!(ReadDatasetQualityInput)).unwrap();
        assert_eq!(
            schema.pointer("/additionalProperties"),
            Some(&Value::Bool(false))
        );
        let properties = schema
            .pointer("/properties")
            .and_then(Value::as_object)
            .unwrap();
        assert_eq!(properties.len(), 2);
        assert!(properties.contains_key("business_id"));
        assert!(properties.contains_key("dataset_id"));
        for field in ["business_id", "dataset_id"] {
            assert_eq!(
                schema.pointer(&format!("/properties/{field}/maxLength")),
                Some(&json!(64))
            );
            assert_eq!(
                schema.pointer(&format!("/properties/{field}/pattern")),
                Some(&json!("^[0-9]{1,64}$"))
            );
        }
        let serialized = schema.to_string();
        assert!(
            serialized.len() < 1_200,
            "schema is {} bytes",
            serialized.len()
        );
        assert!(!serialized.contains("access_token"));
        assert!(
            serde_json::from_value::<ReadDatasetQualityInput>(json!({
                "business_id": "123",
                "dataset_id": "456",
                "page_cursor": "must fail"
            }))
            .is_err()
        );
    }
}
