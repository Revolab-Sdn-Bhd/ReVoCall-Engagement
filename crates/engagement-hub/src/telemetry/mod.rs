pub mod otlp_json;
pub mod local_exporter;
pub mod processor;

use opentelemetry::trace::TraceResult;
use opentelemetry::Context;
use opentelemetry_sdk::trace::{Span, SpanProcessor, TracerProvider};
use opentelemetry_sdk::export::trace::SpanData;
use opentelemetry_sdk::Resource;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Env, RegistryAdapter};
    use crate::metrics::Metrics;
    use crate::telemetry::otlp_json::{fake_span_for_test, StatusForTest};
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
        fn new() -> (Self, Arc<Mutex<Vec<opentelemetry_sdk::export::trace::SpanData>>>) {
            let store = Arc::new(Mutex::new(vec![]));
            (Self { store: store.clone(), panic_on_export: false }, store)
        }
        fn panicking() -> Self {
            Self { store: Arc::new(Mutex::new(vec![])), panic_on_export: true }
        }
    }

    impl opentelemetry_sdk::export::trace::SpanExporter for FakeExporter {
        fn export(
            &mut self,
            batch: Vec<opentelemetry_sdk::export::trace::SpanData>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = opentelemetry_sdk::export::trace::ExportResult> + Send + 'static>> {
            if self.panic_on_export { panic!("intentional panic"); }
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
                            "grafana", e,
                            metrics.otel_exporter_dropped_spans.with_label_values(&["grafana"]),
                        )));
                    }
                    if langfuse {
                        let (e, _) = FakeExporter::new();
                        procs.push(Box::new(processor::CountingSpanProcessor::new(
                            "langfuse", e,
                            metrics.otel_exporter_dropped_spans.with_label_values(&["langfuse"]),
                        )));
                    }
                    if local {
                        let (e, _) = FakeExporter::new();
                        procs.push(Box::new(processor::CountingSpanProcessor::new(
                            "local", e,
                            metrics.otel_exporter_dropped_spans.with_label_values(&["local"]),
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
            "grafana", good_exp,
            metrics.otel_exporter_dropped_spans.with_label_values(&["grafana"]),
        );
        let bad_proc = processor::CountingSpanProcessor::new(
            "langfuse", bad_exp,
            metrics.otel_exporter_dropped_spans.with_label_values(&["langfuse"]),
        );

        let span = fake_span_for_test("op", 5, StatusForTest::Ok);
        bad_proc.on_end(span.clone());
        good_proc.on_end(span);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if !good_store.lock().unwrap().is_empty() { break; }
            if std::time::Instant::now() > deadline {
                panic!("good exporter never received span");
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(good_store.lock().unwrap().len(), 1);
    }
}
