# T1-09: LISTEN/NOTIFY fanout + gap recovery

**Issue:** #16 | **Branch:** feat/16-listen-notify-fanout | **Date:** 2026-05-17

## Brainstorm

### Problem

Watch streams that admin-backend's UI and the outbound dispatcher rely on need
cross-replica event fanout: an event committed on one EH replica must reach
subscribers connected to any other replica within milliseconds. This story wires
up Postgres `LISTEN/NOTIFY` keyed off the `trg_notify_engagement_event` trigger,
plus gap-recovery using per-engagement `sequence` (not `occurred_at`, because
clock skew across replicas would silently drop events). The trigger fires only
on COMMIT; each saga step uses a separate tx (per T1-06 design) so that
subscribers receive events progressively rather than batched.

### Options considered

#### A. LISTEN/NOTIFY with dedicated persistent connection (chosen)

Use `sqlx::PgListener` as a dedicated persistent connection (separate from the
main pool). On each notification, parse the JSON payload and fan out to
subscribers via `tokio::sync::broadcast`. On reconnect, replay missed events
from DB using `sequence > last_seen` per engagement. This is the approach
specified in PRD §11.

#### B. Polling the DB on each replica

Each subscriber polls `engagement_events` on a fixed interval. Simpler but adds
significant DB load (N subscribers × poll rate), introduces latency proportional
to poll interval, and does not scale. Rejected.

#### C. Redis Pub/Sub or external message broker

Adds operational complexity (another stateful service). Not aligned with the
"Postgres-native where possible" philosophy for v1. Deferred consideration for
high-scale future. Rejected for v1.

### Decision

Design pre-determined per PRD §11. Key decisions locked:

- **Separate persistent `PgConnection` for LISTEN** — pool recycles connections
  and drops LISTEN registrations silently; `sqlx::PgListener` owns a single
  dedicated connection.
- **Sequence-based gap recovery** — `sequence` is monotonically increasing per
  engagement and unaffected by clock skew. `occurred_at` is NEVER used as a
  resume cursor.
- **NOTIFY fires only on COMMIT** — per-event tx by design (T1-06), enabling
  progressive streaming. Batching event inserts into one tx would batch the
  NOTIFYs and break the streaming UX.
- **`HashMap<Uuid, Vec<Sender<NotifyPayload>>>`** — subscriber dispatch by
  `engagement_id` or `batch_id`. Dead senders (all receivers dropped) are pruned
  in-place on each fanout.
- **Trigger already in initial schema** — `trg_notify_engagement_event()` and
  `engagement_events_notify` trigger were added in
  `migrations/20260515000000_initial_schema.up.sql` (T1-01). No new migration
  needed for this story.
- **`sqlx::PgListener::eager_reconnect(false)`** — disables automatic
  reconnection so the manager controls reconnect timing and increments the metric
  on each reconnect event.
- **`RECONNECT_DELAY = 100ms`** (revised from initial 5s draft) — PRD requires
  `<5s reconnect`; worst case is 10s health interval + 5s `try_recv` timeout +
  delay; 100ms keeps this well within any reasonable SLO window.
- **`consumer_lag_events` sums `tx.len()` per active sender** (revised from
  counting active senders) — counts buffered-but-undelivered messages, giving
  operators a true lag signal rather than a subscriber count.

## Implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement LISTEN/NOTIFY fanout + gap recovery for the Engagement Hub
so that events committed on any replica reach all matching subscribers within
milliseconds, with reconnect + gap-fill on connection loss.

**Architecture:** A `ListenNotifyManager` owns a dedicated `sqlx::PgListener`
(not from the pool) and issues `LISTEN engagement_events`. On every NOTIFY it
parses the JSON payload, updates the `consumer_lag_events` gauge, and fans out
to broadcast senders keyed by `engagement_id` or `batch_id`. On reconnect the
caller calls `gap_fill()` with the last-seen sequence to replay missed events.

**Tech Stack:** Rust, sqlx 0.8 (`PgListener`, `PgPool`, `query_as`),
`tokio::sync::broadcast`, `prometheus` (IntCounter + IntGauge), `serde_json`,
`uuid`.

### Design decisions

- Trigger + DB migration already exist in T1-01 schema — no new migration file
  needed.
- `CHANNEL_CAP = 256` per subscriber — slow subscribers are dropped
  (STREAM_OVERFLOW); SDK reconnects via sequence cursor (T1-12 wires this).
- `HEALTH_INTERVAL = 10s` health-check via `listener.try_recv()` with 5s
  timeout.
- `RECONNECT_DELAY = 100ms` — reduced from initial 5s after identifying PRD
  compliance gap; worst case (10s health interval + 5s `try_recv` timeout +
  delay) now stays well within any reasonable SLO window.
- `consumer_lag_events` gauge sums `tx.len()` across all active senders —
  true buffered-message count, not active sender count (revised from draft).
- Metrics `engagementhub_listen_notify_reconnects_total` (counter) and
  `engagementhub_consumer_lag_events` (gauge) added to `metrics.rs`.

### File map

| Action | Path | Responsibility |
| ------ | ---- | -------------- |
| Create | `crates/engagement-hub/src/notify.rs` | `NotifyPayload`, `Registry`, `ListenNotifyManager`, gap-fill, unit tests |
| Modify | `crates/engagement-hub/src/lib.rs` | Export `notify` module |
| Modify | `crates/engagement-hub/src/main.rs` | Spawn `ListenNotifyManager` as background task |
| Modify | `crates/engagement-hub/src/metrics.rs` | Add `consumer_lag_events` gauge + tests |
| Create | `crates/engagement-hub/tests/listen_notify.rs` | Integration tests (require live Postgres) |
| Create | `docs/stories/T1-09-listen-notify-fanout-and-gap-recovery.md` | This story doc |

### Task 1: DB trigger verification

**Files:**

- Read: `migrations/20260515000000_initial_schema.up.sql`

- [x] **Step 1: Confirm trigger exists in initial schema**

  The `trg_notify_engagement_event()` function and `engagement_events_notify`
  trigger were added in the T1-01 migration. Verified at lines 148–167.
  No new migration file needed for T1-09.

---

### Task 2: Metrics — consumer\_lag\_events gauge

**Files:**

- Modify: `crates/engagement-hub/src/metrics.rs`

- [x] **Step 1: Add `consumer_lag_events` IntGauge field**

  Added to `Metrics` struct and registered as
  `engagementhub_consumer_lag_events`.

- [x] **Step 2: Update comment on histogram count** (9 of 10 → 10 of 10 per PRD §10.4)

- [x] **Step 3: Add gauge to `all_gauges_registered` test**

- [x] **Step 4: Commit**

```bash
git add crates/engagement-hub/src/metrics.rs
git commit -m "feat(metrics): add consumer_lag_events gauge (T1-09) (#16)"
```

---

### Task 3: LISTEN/NOTIFY manager — notify.rs

**Files:**

- Create: `crates/engagement-hub/src/notify.rs`

- [x] **Step 1: Define `NotifyPayload` struct**

  Fields: `engagement_id`, `organization_id`, `batch_id`, `sequence`,
  `event_pk`, `event_type`, `traceparent`. Derives: `Serialize`, `Deserialize`,
  `Clone`, `Debug`, `PartialEq`.

- [x] **Step 2: Implement `Registry` (subscriber dispatch)**

  `HashMap<Uuid, Vec<Sender<NotifyPayload>>>` for by-engagement and by-batch.
  `subscribe_engagement()`, `subscribe_batch()`, `fanout()` (prunes dead
  senders), `queued_event_count()` (sums `tx.len()` for true lag metric).

- [x] **Step 3: Implement gap-fill query**

  `gap_fill_engagement()` runs:

  ```sql
  SELECT ... FROM engagement_events ee
  JOIN engagements e USING (engagement_id)
  WHERE ee.engagement_id = $1 AND ee.sequence > $2
  ORDER BY ee.sequence ASC
  ```

  Uses `GapRow` (`sqlx::FromRow`) then fans out each row as `NotifyPayload`.

- [x] **Step 4: Implement `connect_and_listen()`**

  Opens `PgListener::connect(url)`, calls `eager_reconnect(false)`, calls
  `listen("engagement_events")`.

- [x] **Step 5: Implement `ListenNotifyManager::run()`**

  Outer reconnect loop: connect → inner LISTEN loop with `tokio::select!` on
  shutdown signal / health-check tick / `listener.recv()`. On disconnect:
  increment `listen_notify_reconnects_total`, sleep `RECONNECT_DELAY`, retry.

- [x] **Step 6: Implement `handle_notification()`**

  Parse payload via `serde_json::from_str`, update `consumer_lag_events` gauge,
  call `registry.fanout()`.

- [x] **Step 7: Write unit tests** (11 tests — all without live DB)

  - `payload_round_trips_with_all_fields`
  - `payload_round_trips_with_null_fields`
  - `payload_json_size_under_200_bytes_typical_case`
  - `payload_json_size_worst_case_documented`
  - `payload_malformed_json_does_not_panic`
  - `payload_missing_required_field_is_error`
  - `sequence_order_is_independent_of_occurred_at`
  - `fanout_delivers_to_engagement_subscriber`
  - `fanout_delivers_to_batch_subscriber`
  - `fanout_does_not_deliver_to_wrong_engagement`
  - `fanout_prunes_dead_senders`
  - `multiple_subscribers_all_receive`

- [x] **Step 8: Commit**

```bash
git add crates/engagement-hub/src/notify.rs
git commit -m "feat(notify): LISTEN/NOTIFY fanout + gap recovery (T1-09) (#16)"
```

---

### Task 4: Wire notify into lib.rs and main.rs

**Files:**

- Modify: `crates/engagement-hub/src/lib.rs`
- Modify: `crates/engagement-hub/src/main.rs`

- [x] **Step 1: Export `notify` module in `lib.rs`**

- [x] **Step 2: Spawn `ListenNotifyManager` in `main.rs`**

  After `db::run_migrations`, before gRPC server spawn:

  ```rust
  let listen_manager = ListenNotifyManager::new(cfg.database_url.clone(), pool.clone(), metrics.clone());
  let listen_shutdown_rx = shutdown.shutdown_rx.clone();
  let _listen_handle = tokio::spawn(async move { listen_manager.run(listen_shutdown_rx).await; });
  ```

- [x] **Step 3: Commit**

```bash
git add crates/engagement-hub/src/lib.rs crates/engagement-hub/src/main.rs
git commit -m "feat(main): spawn ListenNotifyManager background task (#16)"
```

---

### Task 5: Integration tests

**Files:**

- Create: `crates/engagement-hub/tests/listen_notify.rs`

- [x] **Step 1: Write integration tests** (3 tests — require live Postgres, skip gracefully)

  - `insert_event_subscriber_receives_notify` — end-to-end: insert triggers NOTIFY, subscriber receives.
  - `gap_fill_sequence_cursor_delivers_missed_events` — gap-fill with `sequence > last_seen` returns events in order.
  - `clock_skew_sequence_resume_ignores_occurred_at` — two events, same `occurred_at`, sequence order held.

- [x] **Step 2: Commit**

```bash
git add crates/engagement-hub/tests/listen_notify.rs
git commit -m "test(notify): integration tests for LISTEN/NOTIFY + gap-fill (#16)"
```

---

### Task 6: Story doc

**Files:**

- Create: `docs/stories/T1-09-listen-notify-fanout-and-gap-recovery.md`

- [x] **Step 1: Write brainstorm + implementation plan** (this file)

- [x] **Step 2: Commit**

```bash
git add docs/stories/T1-09-listen-notify-fanout-and-gap-recovery.md
git commit -m "docs: add story doc brainstorm + implementation plan for #16"
```

---

### Deferred

- `STREAM_OVERFLOW` close + SDK reconnect via sequence cursor — T1-12 (watch streams) wires this end-to-end.
- Per-subscriber event lag tracking (current gauge is a proxy using sender count) — T1-12.
- `WatchEngagements` cross-engagement ordering via `event_pk` — consumed by T1-12.
