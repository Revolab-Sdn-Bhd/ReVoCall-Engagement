pub mod local_exporter;
pub mod otlp_json;
pub mod processor;

use std::collections::HashMap;

use opentelemetry::trace::TraceResult;
use opentelemetry::trace::TracerProvider as TracerProviderTrait;
use opentelemetry::{Context, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_otlp::WithHttpConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::export::trace::SpanData;
use opentelemetry_sdk::trace::{Span, SpanProcessor, TracerProvider};
use opentelemetry_semantic_conventions::attribute::{SERVICE_NAME, SERVICE_VERSION};
// SERVICE_NAMESPACE and DEPLOYMENT_ENVIRONMENT are behind the `semconv_experimental` feature
// in 0.27; use the string literals directly.
const SERVICE_NAMESPACE: &str = "service.namespace";
const DEPLOYMENT_ENVIRONMENT: &str = "deployment.environment";
use tracing_subscriber::{EnvFilter, prelude::*};

use crate::config::{Config, LogFormat};
use crate::metrics::Metrics;

/// A newtype wrapper so that `Box<dyn SpanProcessor>` can be passed to
/// `TracerProvider::builder().with_span_processor(...)`, which requires
/// `T: SpanProcessor + 'static` (the SDK does not provide a blanket impl
/// for `Box<dyn SpanProcessor>`).
struct BoxedProcessor(Box<dyn SpanProcessor>);

impl std::fmt::Debug for BoxedProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoxedProcessor").finish()
    }
}

impl SpanProcessor for BoxedProcessor {
    fn on_start(&self, span: &mut Span, cx: &Context) {
        self.0.on_start(span, cx);
    }
    fn on_end(&self, span: SpanData) {
        self.0.on_end(span);
    }
    fn force_flush(&self) -> TraceResult<()> {
        self.0.force_flush()
    }
    fn shutdown(&self) -> TraceResult<()> {
        self.0.shutdown()
    }
}

pub fn build_provider(
    resource: Resource,
    processors: Vec<Box<dyn SpanProcessor>>,
) -> TracerProvider {
    let mut builder = TracerProvider::builder().with_resource(resource);
    for p in processors {
        builder = builder.with_span_processor(BoxedProcessor(p));
    }
    builder.build()
}

pub fn init_telemetry(config: &Config, metrics: &Metrics) {
    let resource = Resource::new(vec![
        KeyValue::new(SERVICE_NAME, "engagement-hub"),
        KeyValue::new(SERVICE_VERSION, env!("CARGO_PKG_VERSION")),
        KeyValue::new(SERVICE_NAMESPACE, "revocall"),
        KeyValue::new(DEPLOYMENT_ENVIRONMENT, config.env.as_metric_label()),
    ]);

    let mut processors: Vec<Box<dyn SpanProcessor>> = Vec::new();

    if config.otel_export_grafana {
        match opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(&config.otel_grafana_endpoint)
            .build()
        {
            Ok(exporter) => {
                processors.push(Box::new(processor::CountingSpanProcessor::new(
                    "grafana",
                    exporter,
                    metrics
                        .otel_exporter_dropped_spans
                        .with_label_values(&["grafana"]),
                )));
            }
            Err(e) => eprintln!("[otel] failed to build Grafana exporter: {e}"),
        }
    }

    if config.otel_export_langfuse {
        let auth = base64_encode(&format!(
            "{}:{}",
            config.langfuse_public_key.as_deref().unwrap_or(""),
            config.langfuse_secret_key.as_deref().unwrap_or(""),
        ));
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), format!("Basic {auth}"));

        match opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint(&config.otel_langfuse_endpoint)
            .with_headers(headers)
            .build()
        {
            Ok(exporter) => {
                processors.push(Box::new(processor::CountingSpanProcessor::new(
                    "langfuse",
                    exporter,
                    metrics
                        .otel_exporter_dropped_spans
                        .with_label_values(&["langfuse"]),
                )));
            }
            Err(e) => eprintln!("[otel] failed to build Langfuse exporter: {e}"),
        }
    }

    if config.otel_export_local {
        local_exporter::JsonlFileExporter::purge_old_files(
            std::path::Path::new(".traces"),
            chrono::Local::now().date_naive(),
            7,
        );
        processors.push(Box::new(processor::CountingSpanProcessor::new(
            "local",
            local_exporter::JsonlFileExporter::new_with_counter(
                "engagement-hub",
                build_resource_attrs(config),
                metrics
                    .otel_exporter_dropped_spans
                    .with_label_values(&["local"]),
            ),
            metrics
                .otel_exporter_dropped_spans
                .with_label_values(&["local"]),
        )));
    }

    let provider = build_provider(resource, processors);
    let tracer = provider.tracer("engagement-hub");
    opentelemetry::global::set_tracer_provider(provider);

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,engagement_hub=debug,sqlx::query=warn"));
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(otel_layer);

    match config.log_format {
        LogFormat::Pretty => registry
            .with(tracing_subscriber::fmt::layer().pretty())
            .init(),
        LogFormat::Json => registry
            .with(tracing_subscriber::fmt::layer().json())
            .init(),
    }
}

pub fn shutdown_telemetry() {
    opentelemetry::global::shutdown_tracer_provider();
}

fn build_resource_attrs(config: &Config) -> Vec<otlp_json::KvAttribute> {
    use otlp_json::{KvAttribute, KvValue};
    vec![
        KvAttribute {
            key: "service.name".into(),
            value: KvValue::StringValue("engagement-hub".into()),
        },
        KvAttribute {
            key: "service.version".into(),
            value: KvValue::StringValue(env!("CARGO_PKG_VERSION").into()),
        },
        KvAttribute {
            key: "deployment.environment".into(),
            value: KvValue::StringValue(config.env.as_metric_label().into()),
        },
    ]
}

fn base64_encode(input: &str) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 {
            chunk[1] as usize
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            chunk[2] as usize
        } else {
            0
        };
        out.push(TABLE[b0 >> 2] as char);
        out.push(TABLE[((b0 & 3) << 4) | (b1 >> 4)] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((b1 & 0xf) << 2) | (b2 >> 6)] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[b2 & 0x3f] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Env, RegistryAdapter};
    use crate::metrics::Metrics;
    use crate::telemetry::otlp_json::{StatusForTest, fake_span_for_test};
    use opentelemetry_sdk::trace::SpanProcessor as SpanProcessorTrait;
    use std::sync::{Arc, Mutex};

    fn test_metrics() -> Metrics {
        Metrics::new(RegistryAdapter::Stub, Env::Dev, false).unwrap()
    }

    #[derive(Debug)]
    struct FakeExporter {
        store: Arc<Mutex<Vec<opentelemetry_sdk::export::trace::SpanData>>>,
        panic_on_export: bool,
    }

    impl FakeExporter {
        fn new() -> (
            Self,
            Arc<Mutex<Vec<opentelemetry_sdk::export::trace::SpanData>>>,
        ) {
            let store = Arc::new(Mutex::new(vec![]));
            (
                Self {
                    store: store.clone(),
                    panic_on_export: false,
                },
                store,
            )
        }
        fn panicking() -> Self {
            Self {
                store: Arc::new(Mutex::new(vec![])),
                panic_on_export: true,
            }
        }
    }

    impl opentelemetry_sdk::export::trace::SpanExporter for FakeExporter {
        fn export(
            &mut self,
            batch: Vec<opentelemetry_sdk::export::trace::SpanData>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = opentelemetry_sdk::export::trace::ExportResult>
                    + Send
                    + 'static,
            >,
        > {
            if self.panic_on_export {
                panic!("intentional panic");
            }
            self.store.lock().unwrap().extend(batch);
            Box::pin(std::future::ready(Ok(())))
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn combination_matrix_builds_without_panic() {
        for grafana in [false, true] {
            for langfuse in [false, true] {
                for local in [false, true] {
                    let metrics = test_metrics();
                    let mut procs: Vec<Box<dyn SpanProcessor>> = vec![];
                    if grafana {
                        let (e, _) = FakeExporter::new();
                        procs.push(Box::new(processor::CountingSpanProcessor::new(
                            "grafana",
                            e,
                            metrics
                                .otel_exporter_dropped_spans
                                .with_label_values(&["grafana"]),
                        )));
                    }
                    if langfuse {
                        let (e, _) = FakeExporter::new();
                        procs.push(Box::new(processor::CountingSpanProcessor::new(
                            "langfuse",
                            e,
                            metrics
                                .otel_exporter_dropped_spans
                                .with_label_values(&["langfuse"]),
                        )));
                    }
                    if local {
                        let (e, _) = FakeExporter::new();
                        procs.push(Box::new(processor::CountingSpanProcessor::new(
                            "local",
                            e,
                            metrics
                                .otel_exporter_dropped_spans
                                .with_label_values(&["local"]),
                        )));
                    }
                    let _provider = build_provider(Resource::empty(), procs);
                }
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn exporter_independence_one_panic_does_not_block_others() {
        let metrics = test_metrics();
        let (good_exp, good_store) = FakeExporter::new();
        let bad_exp = FakeExporter::panicking();

        let good_proc = processor::CountingSpanProcessor::new(
            "grafana",
            good_exp,
            metrics
                .otel_exporter_dropped_spans
                .with_label_values(&["grafana"]),
        );
        let bad_proc = processor::CountingSpanProcessor::new(
            "langfuse",
            bad_exp,
            metrics
                .otel_exporter_dropped_spans
                .with_label_values(&["langfuse"]),
        );

        let span = fake_span_for_test("op", 5, StatusForTest::Ok);
        bad_proc.on_end(span.clone());
        good_proc.on_end(span);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if !good_store.lock().unwrap().is_empty() {
                break;
            }
            if std::time::Instant::now() > deadline {
                panic!("good exporter never received span");
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(good_store.lock().unwrap().len(), 1);
    }
}
