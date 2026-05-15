# T3-01: OTEL exporters — three-toggle plumbing + local file exporter + bin/trace-query

**Issue:** #32 | **Branch:** feat/32-otel-exporters | **Date:** 2026-05-15

## Brainstorm

### Problem

The legacy `OTEL_TYPE` single-selector forced an either/or choice between Grafana/Tempo and Langfuse, and coupled exporter health directly to request latency — if the active exporter stalled, it could block the RPC path. Developers had no zero-infra way to inspect traces locally without standing up a collector. This story replaces that design with three independently-toggled exporters (Grafana, Langfuse, local JSONL file) so one exporter failing or being disabled never starves the others, and adds `bin/trace-query` so engineers and Claude can query local traces with nothing but jq.

### Options considered

- **`BatchSpanProcessor` wrapping vs. custom `CountingSpanProcessor`:** Wrapping `BatchSpanProcessor` was simpler but drops spans silently on queue overflow with no observable callback, making `otel_exporter_dropped_spans_total` impossible to maintain accurately from outside. A custom implementation was chosen: `CountingSpanProcessor<E>` owns a bounded `tokio::sync::mpsc::channel(2048)`, batches spans in a spawned background task, and calls `E::export()` with a 5 s timeout — giving full control over drop counting.
- **OTLP JSON via prost vs. mirror structs:** Pulling in `prost` (used by the gRPC exporter) for the local file format would add build complexity. Serde-annotated mirror structs that emit the OTLP JSON schema directly were simpler and self-contained.
- **`trace-query` in Rust vs. Bash+jq:** A Rust binary would typecheck the OTLP schema but adds a compile step. Bash+jq is zero-build, readable, and sufficient for flat span queries over local JSONL files.

### Decision

Three-toggle design: `OTEL_EXPORT_GRAFANA`, `OTEL_EXPORT_LANGFUSE`, `OTEL_EXPORT_LOCAL` each control an independent `CountingSpanProcessor` inside a single `TracerProvider`. Independence is guaranteed because each processor owns its own channel and background task — a panic or timeout in one does not propagate to the others. Legacy `OTEL_TYPE` is translated at parse time with a deprecation warning and then discarded.

## Implementation plan

### Design decisions locked in

- Bounded queue: 2048 spans, max export batch 512, 1 s schedule delay, 5 s export timeout
- Shutdown protocol: oneshot flush signal → drain one more batch → `E::shutdown()` → join with 5 s timeout; uses `block_in_place` so it works on a multi-thread Tokio runtime
- OTLP JSON mirror structs: no `prost` dependency; status code serialized as string enum name per OTLP JSON spec (`STATUS_CODE_ERROR`, `STATUS_CODE_OK`, `STATUS_CODE_UNSET`)
- Local file path: `.traces/engagement-hub-<YYYY-MM-DD>.jsonl` relative to CWD; daily rotation, 100 MB size cap, 7-day retention cleaned up at `init_telemetry` time
- `BoxedProcessor` newtype required because `opentelemetry_sdk 0.27` does not blanket-impl `SpanProcessor` for `Box<dyn SpanProcessor>`
- `tracing-opentelemetry` pinned to 0.28 (not 0.27) to avoid trait mismatches against the rest of the 0.27 SDK stack
- Semantic convention constants (`SERVICE_NAMESPACE`, `DEPLOYMENT_ENVIRONMENT`) replaced with inline string literals to avoid the `semconv_experimental` feature flag
- OTLP exporter builder API: `opentelemetry_otlp::SpanExporter::builder().with_tonic().build()` / `.with_http().build()` (plan's `new_exporter().tonic()` pattern was outdated)
- Test helpers (`StatusForTest`, `fake_span_for_test`) marked `#[doc(hidden)]` rather than `#[cfg(test)]` so example binaries compile; safe because `publish = false` in workspace config

### Tasks

1. `chore(t3-01): add opentelemetry workspace dependencies` — add `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp`, `tracing-opentelemetry`, `opentelemetry-semantic-conventions` to workspace `Cargo.toml`; pin `tracing-opentelemetry` to 0.28 to avoid 0.27 trait mismatches
2. `feat(t3-01): add otel_exporter_dropped_spans_total counter to Metrics` — extend `metrics.rs` with a `dropped_spans_total` counter labelled by exporter name
3. `feat(t3-01): add OTEL config fields and legacy OTEL_TYPE translation` — add `otel_export_grafana`, `otel_export_langfuse`, `otel_export_local`, endpoint, and key fields to `Config`; post-parse fixup sets `otel_export_local=true` when `EH_ENV=dev`; `OTEL_TYPE` env var translated with `warn!`
4. `feat(t3-01): OTLP JSON mirror structs and span serialization` — `telemetry/otlp_json.rs`: serde structs that serialize `SpanData` to the OTLP JSON envelope format
5. `feat(t3-01): JsonlFileExporter with write, rotation, size cap, and 7-day retention` — `telemetry/local_exporter.rs`: implements `SpanExporter`; daily rotation, 100 MB cap, retention cleanup at init
6. `feat(t3-01): CountingSpanProcessor with bounded queue and drop counting` — `telemetry/processor.rs`: custom `SpanProcessor` wrapping an exporter in a background Tokio task with bounded channel, batch export, and drop counter
7. `feat(t3-01): build_provider, combination matrix and independence tests` — `telemetry/mod.rs`: `build_provider` assembles one `TracerProvider` from the enabled toggles using `BoxedProcessor` newtype; unit tests for all 8 flag combinations and exporter independence
8. `feat(t3-01): init_telemetry, shutdown_telemetry, wire into main.rs` — public `init_telemetry`/`shutdown_telemetry` API; replace `init_tracing` call in `main.rs`; add 5 s flush on graceful shutdown
9. `feat(t3-01): bootstrap revolab-observability/tools/trace-query + bin symlink` — Bash+jq script with `engagement`, `trace`, `slow`, `errors` subcommands; `bin/trace-query` symlink; fixes for `jq -sc`→`-cn` (first-file skip bug) and extra parens around division for jq 1.7
10. `test(t3-01): trace-query subcommand tests with JSONL fixture` — `tests/trace_query.rs` using `assert_cmd`; fixture timestamps corrected from plan (plan had 50,000 ms not 50 ms)
11. `fix(t3-01): jq -sc → -cn (first file skipped) + extra parens for jq 1.7 division parse` — two jq bugs fixed post-initial-commit during review
12. `test(t3-01): strengthen trace-query test assertions and guards` — add explicit `assert!(output.status.success())` guards and tighter JSON assertions
13. `fix(t3-01): expose test helpers outside #[cfg(test)] so example compiles` — move `StatusForTest`/`fake_span_for_test` from `#[cfg(test)]` to `#[doc(hidden)]`

### Deferred

- Langfuse Basic-auth header injection in the HTTP exporter (fields are plumbed; export call uses unauthenticated builder pending Langfuse staging credentials)
- Cross-service engagement trace linking (multi-service integration test from the DoD — deferred to T3-02 when the second service exists)
- Extracting `revolab-observability` to a standalone repo (flagged in the directory comment; deferred until more tools accumulate)
