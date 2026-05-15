//! `CountingSpanProcessor` – a bounded-channel span processor that
//! batches spans and exports them via a generic `SpanExporter`.
//!
//! * Bounded `tokio::sync::mpsc` channel (default 2 048 slots).
//! * Background task: batches up to 512 spans, flushes every 1 s or when the
//!   batch is full, with a 5 s per-export timeout.
//! * `on_end` uses `try_send`; on failure it increments a prometheus counter.
//! * `shutdown` drains remaining spans, calls `E::shutdown()`, then waits up
//!   to 5 s for the task to exit (blocking the calling thread via
//!   `tokio::task::block_in_place`).

use std::sync::Mutex;
use std::time::Duration;

use opentelemetry::Context;
use opentelemetry::trace::TraceResult;
use opentelemetry_sdk::export::trace::{SpanData, SpanExporter};
use opentelemetry_sdk::trace::Span;

const QUEUE_CAPACITY: usize = 2048;
const MAX_BATCH: usize = 512;
const SCHEDULE_DELAY: Duration = Duration::from_secs(1);
const EXPORT_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

// ──────────────────────────────────────────────────────────────────────────────
// Public struct
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct CountingSpanProcessor {
    tx: tokio::sync::mpsc::Sender<SpanData>,
    shutdown_tx: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    task_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    dropped: prometheus::IntCounter,
}

impl CountingSpanProcessor {
    /// Create a processor with the default queue capacity (2 048).
    pub fn new<E: SpanExporter + 'static>(
        name: &'static str,
        exporter: E,
        dropped: prometheus::IntCounter,
    ) -> Self {
        Self::new_with_capacity(name, exporter, dropped, QUEUE_CAPACITY)
    }

    /// Create a processor with an explicit queue capacity (useful for tests).
    pub fn new_with_capacity<E: SpanExporter + 'static>(
        name: &'static str,
        exporter: E,
        dropped: prometheus::IntCounter,
        capacity: usize,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(capacity);
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::runtime::Handle::current().spawn(run_background(
            name,
            rx,
            shutdown_rx,
            exporter,
            dropped.clone(),
        ));
        Self {
            tx,
            shutdown_tx: Mutex::new(Some(shutdown_tx)),
            task_handle: Mutex::new(Some(handle)),
            dropped,
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// SpanProcessor impl
// ──────────────────────────────────────────────────────────────────────────────

impl opentelemetry_sdk::trace::SpanProcessor for CountingSpanProcessor {
    fn on_start(&self, _span: &mut Span, _cx: &Context) {
        // nothing to do on start
    }

    fn on_end(&self, span: SpanData) {
        if self.tx.try_send(span).is_err() {
            self.dropped.inc();
        }
    }

    fn force_flush(&self) -> TraceResult<()> {
        // Not implemented: buffered spans in the background task's channel are not drained
        // synchronously. Spans will be exported on the next 1s tick or at shutdown.
        // Use `shutdown_telemetry()` (i.e. `SpanProcessor::shutdown`) for a guaranteed flush.
        Ok(())
    }

    fn shutdown(&self) -> TraceResult<()> {
        // Requires a multi-thread Tokio runtime; will panic on the `current_thread` flavor
        // because `tokio::task::block_in_place` is not supported there.
        // Signal the background task to drain and stop.
        if let Ok(mut guard) = self.shutdown_tx.lock()
            && let Some(tx) = guard.take()
        {
            let _ = tx.send(());
        }

        // Block the calling thread until the background task finishes (or times out).
        if let Ok(mut guard) = self.task_handle.lock()
            && let Some(handle) = guard.take()
        {
            let rt = tokio::runtime::Handle::try_current().expect(
                "CountingSpanProcessor::shutdown must be called from within a multi-thread \
                 Tokio runtime",
            );
            tokio::task::block_in_place(|| {
                let _ = rt.block_on(tokio::time::timeout(SHUTDOWN_TIMEOUT, handle));
            });
        }

        Ok(())
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Background task
// ──────────────────────────────────────────────────────────────────────────────

async fn run_background<E: SpanExporter>(
    name: &'static str,
    mut rx: tokio::sync::mpsc::Receiver<SpanData>,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    mut exporter: E,
    dropped: prometheus::IntCounter,
) {
    let mut batch: Vec<SpanData> = Vec::with_capacity(MAX_BATCH);
    let mut interval = tokio::time::interval(SCHEDULE_DELAY);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if !batch.is_empty() {
                    let to_send = std::mem::take(&mut batch);
                    let batch_len = to_send.len();
                    match tokio::time::timeout(EXPORT_TIMEOUT, exporter.export(to_send)).await {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => {
                            eprintln!("[otel:{name}] export failed: {e}; dropping {batch_len} spans");
                            dropped.inc_by(batch_len as u64);
                        }
                        Err(_) => {
                            eprintln!("[otel:{name}] export timed out; dropping {batch_len} spans");
                            dropped.inc_by(batch_len as u64);
                        }
                    }
                }
            }
            Some(span) = rx.recv() => {
                batch.push(span);
                if batch.len() >= MAX_BATCH {
                    let to_send = std::mem::take(&mut batch);
                    let batch_len = to_send.len();
                    match tokio::time::timeout(EXPORT_TIMEOUT, exporter.export(to_send)).await {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => {
                            eprintln!("[otel:{name}] export failed: {e}; dropping {batch_len} spans");
                            dropped.inc_by(batch_len as u64);
                        }
                        Err(_) => {
                            eprintln!("[otel:{name}] export timed out; dropping {batch_len} spans");
                            dropped.inc_by(batch_len as u64);
                        }
                    }
                }
            }
            _ = &mut shutdown_rx => {
                // Close the channel so no new spans are accepted, then drain.
                rx.close();
                while let Ok(span) = rx.try_recv() {
                    batch.push(span);
                    if batch.len() >= MAX_BATCH {
                        let to_send = std::mem::take(&mut batch);
                        let batch_len = to_send.len();
                        match tokio::time::timeout(EXPORT_TIMEOUT, exporter.export(to_send)).await {
                            Ok(Ok(())) => {}
                            Ok(Err(e)) => {
                                eprintln!("[otel:{name}] export failed during drain: {e}; dropping {batch_len} spans");
                                dropped.inc_by(batch_len as u64);
                            }
                            Err(_) => {
                                eprintln!("[otel:{name}] export timed out during drain; dropping {batch_len} spans");
                                dropped.inc_by(batch_len as u64);
                            }
                        }
                    }
                }
                if !batch.is_empty() {
                    let batch_len = batch.len();
                    let final_batch = std::mem::take(&mut batch);
                    match tokio::time::timeout(EXPORT_TIMEOUT, exporter.export(final_batch)).await {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => {
                            eprintln!("[otel:{name}] export failed during shutdown: {e}; dropping {batch_len} spans");
                            dropped.inc_by(batch_len as u64);
                        }
                        Err(_) => {
                            eprintln!("[otel:{name}] export timed out during shutdown; dropping {batch_len} spans");
                            dropped.inc_by(batch_len as u64);
                        }
                    }
                }
                exporter.shutdown();
                return;
            }
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::otlp_json::{StatusForTest, fake_span_for_test};
    use std::sync::{Arc, Mutex};

    #[derive(Debug)]
    struct RecordingExporter(Arc<Mutex<Vec<opentelemetry_sdk::export::trace::SpanData>>>);

    impl RecordingExporter {
        fn new() -> (
            Self,
            Arc<Mutex<Vec<opentelemetry_sdk::export::trace::SpanData>>>,
        ) {
            let store = Arc::new(Mutex::new(vec![]));
            (Self(store.clone()), store)
        }
    }

    impl opentelemetry_sdk::export::trace::SpanExporter for RecordingExporter {
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
            self.0.lock().unwrap().extend(batch);
            Box::pin(std::future::ready(Ok(())))
        }
    }

    #[tokio::test]
    async fn processor_delivers_spans_to_exporter() {
        let (exporter, store) = RecordingExporter::new();
        let counter = prometheus::IntCounter::new("test_drop_deliver", "t").unwrap();
        let processor = CountingSpanProcessor::new("test", exporter, counter);

        let span = fake_span_for_test("op", 10, StatusForTest::Ok);
        opentelemetry_sdk::trace::SpanProcessor::on_end(&processor, span);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if !store.lock().unwrap().is_empty() {
                break;
            }
            if std::time::Instant::now() > deadline {
                panic!("span never delivered to exporter");
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(store.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn processor_increments_dropped_counter_when_queue_full() {
        let (exporter, _) = RecordingExporter::new();
        let counter = prometheus::IntCounter::new("test_drop_full", "t").unwrap();
        let processor =
            CountingSpanProcessor::new_with_capacity("test", exporter, counter.clone(), 1);

        for _ in 0..100 {
            opentelemetry_sdk::trace::SpanProcessor::on_end(
                &processor,
                fake_span_for_test("op", 1, StatusForTest::Ok),
            );
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(counter.get() > 0, "expected dropped counter > 0");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn processor_shuts_down_cleanly() {
        let (exporter, store) = RecordingExporter::new();
        let counter = prometheus::IntCounter::new("test_shutdown", "t").unwrap();
        let processor = CountingSpanProcessor::new("test", exporter, counter);

        opentelemetry_sdk::trace::SpanProcessor::on_end(
            &processor,
            fake_span_for_test("op", 5, StatusForTest::Ok),
        );
        // use SpanProcessor::shutdown
        opentelemetry_sdk::trace::SpanProcessor::shutdown(&processor).unwrap();

        assert!(
            !store.lock().unwrap().is_empty(),
            "span not flushed on shutdown"
        );
    }
}
