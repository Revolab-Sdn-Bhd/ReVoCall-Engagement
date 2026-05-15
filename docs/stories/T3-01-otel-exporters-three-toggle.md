# T3-01: OTEL exporters — three-toggle plumbing + local file exporter + bin/trace-query

## Context

Stands up three independent OTEL span exporters — local JSONL file (dev only), Grafana/Tempo (ops), and Langfuse (prompt/LLM review) — each behind its own toggle so one exporter failing or being disabled never starves the others or blocks the RPC path. The local file exporter plus `bin/trace-query` gives Claude and engineers a zero-infra way to read traces during local development without standing up a collector. Replaces the legacy `OTEL_TYPE` single-selector design, which forced an either/or choice and coupled exporter health to request latency.

## Story details

- **Track:** T3 — Observability, dashboard, runbooks
- **Owner:** EH team
- **PRD refs:** §10.1, §10.2, §10.3
- **Depends on:** none

## Acceptance criteria

- `OTEL_EXPORT_GRAFANA`, `OTEL_EXPORT_LANGFUSE`, `OTEL_EXPORT_LOCAL` each control an independent BatchSpanProcessor
- Processors parallel; one failure doesn't block RPC path or other exporters
- Bounded queue (2048 spans), max export batch (512), 1s schedule delay, 5s timeout
- Queue full → oldest spans drop; `engagementhub_otel_exporter_dropped_spans_total{exporter}` increments
- In-flight RPC never blocked by export
- Replaces legacy `OTEL_TYPE=collector|langfuse|both` selector with deprecation warning
- OTLP JSON spans → JSONL at `$REPO_ROOT/.traces/<service>-<YYYY-MM-DD>.jsonl`
- `OTEL_EXPORT_LOCAL=true` in dev; false in staging/prod
- Daily rotation, 100MB max per file, 7-day retention
- `.traces/` covered by `.gitignore`
- `bin/trace-query` (Bash + jq) symlinked from `revolab-observability/tools/`
- Subcommands: `engagement <id>`, `trace <trace_id>`, `slow [<threshold_ms>]`, `errors`
- Results formatted JSON (trace_id, span_id, operation, duration_ms, status)

## Definition of done

- Independence test: each exporter can fail/drop without affecting others
- Combination matrix tested
- Local exporter file creation + rotation verified
- `bin/trace-query` subcommands validated on sample JSONL
- Cross-service test: multi-service engagement → traces linked by engagement_id
- 7-day retention verified (mock time forward)

## Design

_Approved 2026-05-15. Brainstormed in Session 3._

### Module structure

```
ReVoCall-Engagement/
├── revolab-observability/          ← scaffolded here; extract to standalone repo later
│   └── tools/
│       └── trace-query             ← Bash+jq script (chmod +x)
├── bin/
│   └── trace-query -> ../revolab-observability/tools/trace-query
└── crates/engagement-hub/src/
    ├── main.rs                     ← calls telemetry::init / ::shutdown
    ├── config.rs                   ← adds otel toggle fields
    ├── metrics.rs                  ← adds dropped_spans counter
    └── telemetry/
        ├── mod.rs                  ← init_telemetry(), shutdown_telemetry()
        └── local_exporter.rs       ← JsonlFileExporter implementing SpanExporter
```

The existing `init_tracing` in `main.rs` is replaced by `telemetry::init_telemetry(&config)`. Shutdown (5s flush) added alongside `pool.close().await`.

### Config additions (config.rs)

New fields on existing `Config` struct using clap `#[arg(env = "...")]` pattern. No `EH_` prefix — these are platform-wide vars per PRD §10.

```rust
// Toggles
#[arg(long, env = "OTEL_EXPORT_GRAFANA", default_value_t = true)]
pub otel_export_grafana: bool,

#[arg(long, env = "OTEL_EXPORT_LANGFUSE", default_value_t = false)]  // false for EH
pub otel_export_langfuse: bool,

#[arg(long, env = "OTEL_EXPORT_LOCAL", default_value_t = false)]  // post-parse: true when EH_ENV=dev
pub otel_export_local: bool,

// Endpoints
#[arg(long, env = "OTEL_GRAFANA_ENDPOINT", default_value = "http://localhost:4317")]
pub otel_grafana_endpoint: String,

#[arg(long, env = "OTEL_LANGFUSE_ENDPOINT", default_value = "https://cloud.langfuse.com/api/public/otel")]
pub otel_langfuse_endpoint: String,

#[arg(long, env = "LANGFUSE_PUBLIC_KEY")]
pub langfuse_public_key: Option<String>,

#[arg(long, env = "LANGFUSE_SECRET_KEY")]
pub langfuse_secret_key: Option<String>,
```

`OTEL_EXPORT_LOCAL` defaults to `true` when `EH_ENV=dev`, `false` otherwise — post-parse fixup after clap resolves `EH_ENV`.

**Legacy deprecation**: if `OTEL_TYPE` env var is present, log `warn!` and translate `collector→grafana`, `langfuse→langfuse`, `both→grafana+langfuse`.

### Telemetry init / shutdown (telemetry/mod.rs)

`init_telemetry(&config)` builds one `TracerProvider` with one `BatchSpanProcessor` per enabled toggle, all running in parallel:

```
TracerProvider
  ├── CountingSpanProcessor("grafana", OtlpGrpcExporter)   (OTEL_EXPORT_GRAFANA)
  ├── CountingSpanProcessor("langfuse", OtlpHttpExporter)  (OTEL_EXPORT_LANGFUSE, Basic auth header set at build time)
  └── CountingSpanProcessor("local", JsonlFileExporter)    (OTEL_EXPORT_LOCAL)
```

`CountingSpanProcessor<E>` implements `SpanProcessor` directly. It owns a bounded `tokio::sync::mpsc::channel(2048)` and spawns a background Tokio task that has exclusive `&mut E` ownership. The task batches spans (max 512, 1s schedule delay) and calls `E::export()` with a 5s timeout. `on_end` uses `try_send`; on failure it increments `engagementhub_otel_exporter_dropped_spans_total{exporter=<name>}`. This is intentional re-implementation (not wrapping `BatchSpanProcessor`) because `BatchSpanProcessor` drops silently with no callback, making drop counting impossible from outside.

**Shutdown protocol** (prevents deadlock): `shutdown()` sends a oneshot flush signal to the background task; the task finishes its current batch, drains any remaining spans (up to one more batch), calls `E::shutdown()`, then exits. The calling thread `join`s the task handle with a 5s `tokio::time::timeout`. If the timeout fires, the task is abandoned (spans already in the OS write buffer are lost — acceptable for dev/ops telemetry).

OTEL Resource (built once, attached to provider):

- `service.name = "engagement-hub"`, `service.version = env!("CARGO_PKG_VERSION")`
- `service.namespace = "revocall"`, `deployment.environment = config.env.as_str()`

`tracing_subscriber` registry extended with `tracing_opentelemetry::layer()` so all `tracing::` spans flow into the OTEL pipeline.

Dropped-spans counter added to `metrics.rs`.

### Local file exporter (telemetry/local_exporter.rs)

Implements `opentelemetry_sdk::export::trace::SpanExporter`.

On each `export(batch)`:

1. Check current date — if changed since last write, close old handle, open new `engagement-hub-<YYYY-MM-DD>.jsonl` under `.traces/`
2. Check file size — if ≥ 100MB, skip write, increment `dropped_spans{exporter="local"}` (no overflow file)
3. Convert `Vec<SpanData>` → OTLP JSON envelope via serde-annotated mirror structs (no prost dependency): `{"resourceSpans":[{"resource":{...},"scopeSpans":[{"spans":[...]}]}]}`
4. Append one JSON object (the full batch envelope) as a single line

**7-day retention**: on `init_telemetry()`, scan `.traces/`, delete files whose date suffix is > 7 days old. Runs once at startup (dev-only exporter; no background task needed).

**File path**: `.traces/engagement-hub-<YYYY-MM-DD>.jsonl` relative to CWD (always repo root in dev workflow).

### bin/trace-query (revolab-observability/tools/trace-query)

Bash+jq script. All subcommands read from `.traces/*.jsonl`.

Shared jq pipeline extracts a flat span record from the OTLP envelope:

```
.resourceSpans[].scopeSpans[].spans[]
| {
    trace_id:    .traceId,
    span_id:     .spanId,
    operation:   .name,
    duration_ms: ((.endTimeUnixNano|tonumber) - (.startTimeUnixNano|tonumber)) / 1e6,
    status:      .status.code,
    attrs:       ([.attributes[]? | {(.key): .value.stringValue}] | add // {})
  }
```

| Subcommand | Filter |
| --- | --- |
| `engagement <id>` | `attrs["revolab.engagement_id"] == $id` |
| `trace <trace_id>` | `.trace_id == $trace_id` |
| `slow [<ms>]` | `.duration_ms >= ($threshold\|tonumber)` (default 1000) |
| `errors` | `.status == "STATUS_CODE_ERROR"` |

Note: the shared pipeline extracts `.status.code` into the flat record's `status` field. The `JsonlFileExporter` mirror structs serialize status code as its string enum name (`STATUS_CODE_ERROR`, `STATUS_CODE_OK`, `STATUS_CODE_UNSET`) per OTLP JSON spec — so the string comparison is correct.

Output: JSON array of flat span records (stdout).

`bin/trace-query` is a relative symlink: `bin/trace-query -> ../revolab-observability/tools/trace-query`.

### New workspace dependencies

```toml
opentelemetry          = "0.27"
opentelemetry_sdk      = { version = "0.27", features = ["rt-tokio", "trace"] }
opentelemetry-otlp     = { version = "0.27", features = ["grpc-tonic", "http-proto"] }  # http-proto (not http-json) — Langfuse documents proto encoding only
tracing-opentelemetry  = "0.27"
opentelemetry-semantic-conventions = "0.27"
```

Exact patch versions pinned at `cargo add` time during implementation.

### Tests

All in `crates/engagement-hub/src/telemetry/` via `#[cfg(test)]`.

| Test | Verifies |
| --- | --- |
| `exporter_independence` | One exporter panics; other two still receive spans |
| `local_exporter_creates_file` | `export(batch)` → valid JSONL file in `.traces/` |
| `local_exporter_daily_rotation` | Mock clock past midnight → new file opened |
| `local_exporter_size_cap` | File ≥ 100MB → writes stop, `dropped_spans{local}` increments |
| `local_exporter_retention` | Files 1–9 days old seeded → init deletes > 7-day files |
| `combination_matrix` | All 8 toggle combinations build without panic |
| `legacy_otel_type_deprecation` | `OTEL_TYPE=both` → warn! logged, grafana+langfuse exporters active |
| `trace_query_engagement` | Sample JSONL fixture → `engagement <id>` returns correct spans |
| `trace_query_slow` | Mixed durations → `slow 500` returns only ≥ 500ms spans |
| `trace_query_errors` | Mixed statuses → `errors` returns only ERROR spans |

`trace-query` tests use `assert_cmd` (already a dev-dep).

## Implementation plan

Plan at `docs/superpowers/plans/2026-05-15-t3-01-otel-exporters.md`.

## Implementation notes

Deviations from the original design discovered during implementation:

- **`tracing-opentelemetry` upgraded to 0.28** (design said 0.27). Version 0.27 of `tracing-opentelemetry` internally depends on `opentelemetry` 0.26 / `opentelemetry_sdk` 0.26, causing trait mismatches against the rest of the 0.27 SDK stack.

- **`BoxedProcessor` newtype required.** `opentelemetry_sdk 0.27` does not blanket-implement `SpanProcessor` for `Box<dyn SpanProcessor>`, so the provider builder cannot accept `Vec<Box<dyn SpanProcessor>>`. A `BoxedProcessor(Box<dyn SpanProcessor>)` newtype that forwards all trait methods was added in `telemetry/mod.rs`.

- **`SERVICE_NAMESPACE` / `DEPLOYMENT_ENVIRONMENT` semantic convention constants removed.** These are behind the `semconv_experimental` feature in `opentelemetry-semantic-conventions 0.27`. Replaced with inline string literals (`"service.namespace"`, `"deployment.environment"`) to avoid adding a feature flag.

- **OTLP exporter builder API changed.** The plan's `new_exporter().tonic()` pattern is outdated. Actual API: `opentelemetry_otlp::SpanExporter::builder().with_tonic().build()` / `.with_http().build()`.

- **`futures` crate is dev-dep only.** `SpanExporter::export` return type uses `std::pin::Pin<Box<dyn std::future::Future<Output = ExportResult> + Send + 'static>>` instead of `futures::future::BoxFuture`.

- **`CountingSpanProcessor` shutdown uses `block_in_place`.** Requires multi-thread Tokio runtime; the shutdown test uses `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]`.

- **E2E: Legacy `OTEL_TYPE` path bypassed Langfuse credential validation.** `validate()` ran before `apply_otel_type_legacy`, so `OTEL_TYPE=langfuse` with no keys passed validation then enabled the exporter post-check. Fixed: `validate()` is called a second time after the legacy translation in `main.rs`.

- **E2E: `opentelemetry-otlp` missing `reqwest-client` feature.** The `http-proto` feature enables the protobuf codec but not the HTTP client; Langfuse exporter failed at runtime with `no http client, you must select one from features`. Fixed: added `"reqwest-client"` to `opentelemetry-otlp` workspace features.

- **`StatusForTest` / `fake_span_for_test` moved from `#[cfg(test)]` to `#[doc(hidden)]` (Task 11).** Pre-existing `examples/test_otlp.rs` referenced these symbols. `#[cfg(test)]` prevented compilation of the example binary. Changed to `#[doc(hidden)]` so they compile in all build modes while remaining hidden from rustdoc. Safe because `publish = false` in workspace config prevents external access.

- **Fixture timestamps in plan were wrong (Task 10).** The plan specified `endTimeUnixNano="1050000000000"` (start + 50,000,000,000 ns = 50,000 ms), not 50 ms. Correct timestamps: `endTimeUnixNano="1000050000000"` (start + 50,000,000 ns = 50 ms), `endTimeUnixNano="1001500000000"` (1500 ms), `endTimeUnixNano="1000200000000"` (200 ms). Corrected during implementation.

- **`trace-query` jq fixes (Task 9).** Two bugs found during code review:
  1. `jq -sc` + `[inputs | ...]` silently drops the first file (first file consumed by slurp, `inputs` only gets the rest). Fixed: `jq -sc` → `jq -cn` across all four subcommands.
  2. `/ 1000000` after a closing `)` triggers a jq 1.7 (macOS) parse error (misread as regex). Fixed: extra outer parens around the full division expression — `((sub) / 1000000)`.
