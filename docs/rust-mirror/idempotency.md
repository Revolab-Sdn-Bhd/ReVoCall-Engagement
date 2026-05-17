# Rust mirror: idempotency key derivation

Go canonical: `clients/go/engagementhub/shared/idempotency`
Rust implementation scope: T6-04 (#56)

## Namespace constant

```rust
use uuid::{uuid, Uuid};

const NAMESPACE_OUTBOUND: Uuid = uuid!("ba40c89b-d320-47cd-aa7c-c05c3b24dd6a");
```

## Function signature

```rust
/// Derives a deterministic UUIDv5 request_id for the given outbound campaign attempt.
///
/// `attempt_number` must be the plain decimal string of the attempt integer ("1", "2", ...).
/// Panics if any argument is empty.
pub fn derive_request_id(batch_id: &str, contact_number: &str, attempt_number: &str) -> String {
    assert!(
        !batch_id.is_empty() && !contact_number.is_empty() && !attempt_number.is_empty(),
        "derive_request_id: all arguments must be non-empty"
    );
    let data = format!("{}:{}:{}", batch_id, contact_number, attempt_number);
    Uuid::new_v5(&NAMESPACE_OUTBOUND, data.as_bytes()).to_string()
}
```

Required: `uuid = { version = "1", features = ["v5"] }`

## Known-value pins

The Rust implementation must produce identical UUIDs for identical inputs:

| batch_id | contact_number | attempt_number | Expected UUID |
| ---------- | ---------------- | ---------------- | ------------- |
| `batch-abc` | `+60126013446` | `1` | `03518426-c533-5d8f-bbb9-f8ad0c139ffb` |
| `batch-abc` | `+60126013446` | `2` | `092e314e-4c2b-59d8-9991-1c438df81e2e` |
| `batch-abc` | `+60126013447` | `1` | `49443967-f52d-512f-9934-03269b7e401c` |
