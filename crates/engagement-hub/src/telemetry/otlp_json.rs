//! OTLP JSON mirror structs for serialising `SpanData` into the OTLP/JSON
//! trace wire format.  The structs use camelCase keys to match the OTLP spec.

use opentelemetry::Value as OtelValue;
use opentelemetry::trace::{SpanId, SpanKind, Status};
use opentelemetry_sdk::export::trace::SpanData;
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

// ──────────────────────────────────────────────────────────────────────────────
// Top-level envelope
// ──────────────────────────────────────────────────────────────────────────────

/// Root object: `{"resourceSpans": [...]}`
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TracesData {
    pub resource_spans: Vec<ResourceSpans>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSpans {
    pub resource: OtlpResource,
    pub scope_spans: Vec<ScopeSpans>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtlpResource {
    pub attributes: Vec<KvAttribute>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeSpans {
    pub scope: OtlpScope,
    pub spans: Vec<OtlpSpan>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtlpScope {
    pub name: String,
}

// ──────────────────────────────────────────────────────────────────────────────
// Span
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtlpSpan {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: String,
    pub name: String,
    /// SpanKind integer: Internal=1, Server=2, Client=3, Producer=4, Consumer=5
    pub kind: i32,
    /// Nanoseconds since Unix epoch, encoded as a decimal string per OTLP spec.
    pub start_time_unix_nano: String,
    pub end_time_unix_nano: String,
    pub attributes: Vec<KvAttribute>,
    pub status: OtlpStatus,
}

// ──────────────────────────────────────────────────────────────────────────────
// Attributes
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct KvAttribute {
    pub key: String,
    pub value: KvValue,
}

/// OTLP attribute value — only the variant field is serialised.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KvValue {
    StringValue(String),
    BoolValue(bool),
    /// OTLP encodes int64 as a JSON string to avoid precision loss.
    IntValue(String),
    DoubleValue(f64),
}

// ──────────────────────────────────────────────────────────────────────────────
// Status
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct OtlpStatus {
    pub code: &'static str,
}

const STATUS_UNSET: &str = "STATUS_CODE_UNSET";
const STATUS_OK: &str = "STATUS_CODE_OK";
const STATUS_ERROR: &str = "STATUS_CODE_ERROR";

// ──────────────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────────────

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn system_time_to_nano_str(t: SystemTime) -> String {
    t.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string()
}

fn span_kind_to_int(kind: &SpanKind) -> i32 {
    match kind {
        SpanKind::Internal => 1,
        SpanKind::Server => 2,
        SpanKind::Client => 3,
        SpanKind::Producer => 4,
        SpanKind::Consumer => 5,
    }
}

fn otel_value_to_kv(value: &OtelValue) -> KvValue {
    match value {
        OtelValue::Bool(b) => KvValue::BoolValue(*b),
        OtelValue::I64(i) => KvValue::IntValue(i.to_string()),
        OtelValue::F64(f) => KvValue::DoubleValue(*f),
        OtelValue::String(s) => KvValue::StringValue(s.to_string()),
        // Arrays are serialised as their debug representation (best-effort).
        OtelValue::Array(a) => KvValue::StringValue(format!("{a:?}")),
        _ => KvValue::StringValue(format!("{value:?}")),
    }
}

fn status_to_otlp(status: &Status) -> OtlpStatus {
    let code = match status {
        Status::Unset => STATUS_UNSET,
        Status::Ok => STATUS_OK,
        Status::Error { .. } => STATUS_ERROR,
    };
    OtlpStatus { code }
}

// ──────────────────────────────────────────────────────────────────────────────
// Public conversion function
// ──────────────────────────────────────────────────────────────────────────────

/// Convert a batch of `SpanData` into an OTLP `TracesData` envelope ready for
/// JSON serialisation.
///
/// * `resource_attrs` – resource-level key-value pairs (e.g. `service.name`).
/// * `scope_name`     – instrumentation scope name placed in `scope.name`.
pub fn spans_to_traces_data(
    batch: &[SpanData],
    resource_attrs: &[KvAttribute],
    scope_name: &str,
) -> TracesData {
    let spans: Vec<OtlpSpan> = batch
        .iter()
        .map(|span| {
            let trace_id = bytes_to_hex(&span.span_context.trace_id().to_bytes());
            let span_id = bytes_to_hex(&span.span_context.span_id().to_bytes());
            let parent_span_id = if span.parent_span_id == SpanId::INVALID {
                String::new()
            } else {
                bytes_to_hex(&span.parent_span_id.to_bytes())
            };

            let attributes: Vec<KvAttribute> = span
                .attributes
                .iter()
                .map(|kv| KvAttribute {
                    key: kv.key.to_string(),
                    value: otel_value_to_kv(&kv.value),
                })
                .collect();

            OtlpSpan {
                trace_id,
                span_id,
                parent_span_id,
                name: span.name.to_string(),
                kind: span_kind_to_int(&span.span_kind),
                start_time_unix_nano: system_time_to_nano_str(span.start_time),
                end_time_unix_nano: system_time_to_nano_str(span.end_time),
                attributes,
                status: status_to_otlp(&span.status),
            }
        })
        .collect();

    // Build resource attributes (cloning strings from the slice provided).
    let resource_attributes: Vec<KvAttribute> = resource_attrs
        .iter()
        .map(|kv| KvAttribute {
            key: kv.key.clone(),
            value: match &kv.value {
                KvValue::StringValue(s) => KvValue::StringValue(s.clone()),
                KvValue::BoolValue(b) => KvValue::BoolValue(*b),
                KvValue::IntValue(i) => KvValue::IntValue(i.clone()),
                KvValue::DoubleValue(f) => KvValue::DoubleValue(*f),
            },
        })
        .collect();

    TracesData {
        resource_spans: vec![ResourceSpans {
            resource: OtlpResource {
                attributes: resource_attributes,
            },
            scope_spans: vec![ScopeSpans {
                scope: OtlpScope {
                    name: scope_name.to_string(),
                },
                spans,
            }],
        }],
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Test helpers (only compiled under `cfg(test)`; not part of the public API)
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(dead_code)]
pub(crate) enum StatusForTest {
    Ok,
    Error,
    Unset,
}

#[cfg(test)]
pub(crate) fn fake_span_for_test(
    name: &'static str,
    duration_ms: u64,
    status: StatusForTest,
) -> SpanData {
    use opentelemetry::InstrumentationScope;
    use opentelemetry::trace::{SpanContext, TraceFlags, TraceId, TraceState};
    use std::borrow::Cow;
    use std::time::Duration;

    let trace_id = TraceId::from_bytes([1u8; 16]);
    let span_id = SpanId::from_bytes([2u8; 8]);
    let span_context = SpanContext::new(
        trace_id,
        span_id,
        TraceFlags::SAMPLED,
        false,
        TraceState::NONE,
    );

    let start_time = UNIX_EPOCH + Duration::from_secs(1_000_000);
    let end_time = start_time + Duration::from_millis(duration_ms);

    let otel_status = match status {
        StatusForTest::Ok => Status::Ok,
        StatusForTest::Error => Status::Error {
            description: Cow::Borrowed("test error"),
        },
        StatusForTest::Unset => Status::Unset,
    };

    SpanData {
        span_context,
        parent_span_id: SpanId::INVALID,
        span_kind: SpanKind::Internal,
        name: Cow::Borrowed(name),
        start_time,
        end_time,
        attributes: vec![],
        dropped_attributes_count: 0,
        events: opentelemetry_sdk::trace::SpanEvents::default(),
        links: opentelemetry_sdk::trace::SpanLinks::default(),
        status: otel_status,
        instrumentation_scope: InstrumentationScope::builder("test").build(),
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_serializes_to_otlp_json_envelope() {
        let span = fake_span_for_test("my-op", 42, StatusForTest::Ok);
        let data = spans_to_traces_data(&[span], &[], "test-scope");
        let line = serde_json::to_string(&data).unwrap();
        assert!(
            line.contains("resourceSpans"),
            "missing resourceSpans:\n{line}"
        );
        assert!(line.contains("my-op"), "missing operation name:\n{line}");
        assert!(line.contains("traceId"), "missing traceId:\n{line}");
        assert!(line.contains("spanId"), "missing spanId:\n{line}");
        assert!(line.contains("startTimeUnixNano"), "missing start:\n{line}");
        assert!(line.contains("STATUS_CODE_OK"), "missing status:\n{line}");
    }

    #[test]
    fn error_span_has_error_status_code() {
        let span = fake_span_for_test("fail-op", 10, StatusForTest::Error);
        let data = spans_to_traces_data(&[span], &[], "test");
        let line = serde_json::to_string(&data).unwrap();
        assert!(line.contains("STATUS_CODE_ERROR"), "{line}");
    }
}
