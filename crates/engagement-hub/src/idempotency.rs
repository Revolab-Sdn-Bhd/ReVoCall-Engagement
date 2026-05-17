//! Idempotency primitives (PRD §9, AIP-155).
//!
//! ## Canonical hash
//!
//! Each mutating RPC defines an `idempotency_fields` allow-list of request fields
//! that determine uniqueness. The hash is:
//!
//! ```text
//! sha256(canonical_json(idempotency_fields))
//! ```
//!
//! where `canonical_json` sorts object keys lexicographically and emits no
//! insignificant whitespace — equivalent to RFC 8785 for the scalar types
//! (string, UUID, bool, integer) present in the StartEngagement allow-list.
//!
//! ## Flow (AIP-155)
//!
//! 1. SELECT engagement WHERE organization\_id = O AND request\_id = R.
//! 2. Row found: compare stored `payload_hash` with computed hash.
//!    - Match → return current engagement state (success / replay).
//!    - Mismatch → `REQUEST_ID_CONFLICT`.
//! 3. Row not found: compute hash H, INSERT … ON CONFLICT DO NOTHING RETURNING …
//!    If no row returned (concurrent duplicate) → go to step 1.
//!
//! ## StartEngagement allow-list
//!
//! `org_id`, `channel`, `mode`, `journey_version`, `contact`, `batch_id`,
//! `voice`, `test_mode`, `language`.
//!
//! `metadata` and `contact.display_name` are explicitly excluded.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The 32-byte SHA-256 digest of the canonical JSON for an idempotency key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadHash([u8; 32]);

impl PayloadHash {
    /// Borrow as a byte slice — used for `BYTEA` binding.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Build from a raw 32-byte array.
    pub fn from_bytes(b: [u8; 32]) -> Self {
        Self(b)
    }

    /// Try to convert a `Vec<u8>` (sqlx BYTEA decode) into a hash.
    ///
    /// Returns `None` if the length is not exactly 32.
    pub fn from_vec(v: Vec<u8>) -> Option<Self> {
        let arr: [u8; 32] = v.try_into().ok()?;
        Some(Self(arr))
    }

    /// Convert to a hex string (useful for logging / debugging).
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl std::fmt::Display for PayloadHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

/// Outcome of an idempotency check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdempotencyResult {
    /// No prior row found — caller should proceed with the new request.
    New,
    /// Row found and hash matches — return the existing engagement ID.
    Replay {
        engagement_id: Uuid,
        /// True if the engagement is in a terminal state (success / cancelled / failed).
        is_terminal: bool,
    },
    /// Row found but hash differs — the caller must return `REQUEST_ID_CONFLICT`.
    Conflict,
}

/// Lightweight container for the fields that participate in the canonical hash
/// for `StartEngagement`.
///
/// Fields correspond to the PRD §9 allow-list. All values are `Option<String>`
/// to accommodate optional proto fields; absent fields are omitted from the JSON
/// object entirely (not set to `null`) — equivalent to RFC 8785 member absence.
///
/// **Excluded from hash:** `metadata`, `contact.display_name`.
///
/// # idempotency_fields allow-list
///
/// StartEngagement: org_id, channel, mode, journey_version, contact (kind +
/// id/phone), batch_id, voice (profile_id), test_mode, language.
#[derive(Debug, Clone)]
pub struct StartEngagementFields {
    // idempotency_fields:
    /// `organization_id`
    pub org_id: String,
    /// `channel` (proto enum ordinal, as string)
    pub channel: String,
    /// `mode` (proto enum ordinal, as string)
    pub mode: String,
    /// `journey_version`
    pub journey_version: String,
    /// `contact.kind` (serialised as string)
    pub contact_kind: String,
    /// `contact.id` — absent for phone contacts
    pub contact_id: Option<String>,
    /// `contact.phone_e164` — absent for UUID contacts
    pub contact_phone: Option<String>,
    /// `batch_id` — absent if not provided
    pub batch_id: Option<String>,
    /// `voice.profile_id` — absent if not provided
    pub voice_profile_id: Option<String>,
    /// `test_mode`
    pub test_mode: bool,
    /// `language` (BCP-47 tag) — absent if not provided
    pub language: Option<String>,
}

// ---------------------------------------------------------------------------
// Canonical hash
// ---------------------------------------------------------------------------

/// Compute the idempotency hash for a `StartEngagement` request.
///
/// The hash is `sha256(canonical_json(fields))` where canonical JSON
/// sorts object keys lexicographically and emits no whitespace.
pub fn canonical_hash_start_engagement(fields: &StartEngagementFields) -> PayloadHash {
    let json = canonical_json_start_engagement(fields);
    let digest = Sha256::digest(json.as_bytes());
    PayloadHash(digest.into())
}

/// Produce the canonical JSON string for the given fields.
///
/// All optional fields absent from the struct are **omitted** from the object
/// (not serialised as `null`). Present fields are serialised as JSON strings
/// or booleans. Keys are sorted lexicographically via [`BTreeMap`].
pub fn canonical_json_start_engagement(fields: &StartEngagementFields) -> String {
    let mut map: BTreeMap<&str, serde_json::Value> = BTreeMap::new();

    map.insert("channel", serde_json::Value::String(fields.channel.clone()));
    map.insert(
        "contact_kind",
        serde_json::Value::String(fields.contact_kind.clone()),
    );

    if let Some(ref id) = fields.contact_id {
        map.insert("contact_id", serde_json::Value::String(id.clone()));
    }
    if let Some(ref phone) = fields.contact_phone {
        map.insert("contact_phone", serde_json::Value::String(phone.clone()));
    }
    if let Some(ref bid) = fields.batch_id {
        map.insert("batch_id", serde_json::Value::String(bid.clone()));
    }
    if let Some(ref lang) = fields.language {
        map.insert("language", serde_json::Value::String(lang.clone()));
    }

    map.insert(
        "journey_version",
        serde_json::Value::String(fields.journey_version.clone()),
    );
    map.insert("mode", serde_json::Value::String(fields.mode.clone()));
    map.insert("org_id", serde_json::Value::String(fields.org_id.clone()));
    map.insert("test_mode", serde_json::Value::Bool(fields.test_mode));

    if let Some(ref vp) = fields.voice_profile_id {
        map.insert("voice_profile_id", serde_json::Value::String(vp.clone()));
    }

    // serde_json serialises BTreeMap with keys in sorted order.
    serde_json::to_string(&map).expect("BTreeMap<&str, Value> serialisation is infallible")
}

// ---------------------------------------------------------------------------
// IdempotencyChecker
// ---------------------------------------------------------------------------

/// Checks and records idempotency for mutating RPCs.
///
/// This is a thin service-layer wrapper around DB reads; it does not hold
/// any mutable state itself.
pub struct IdempotencyChecker;

impl IdempotencyChecker {
    pub fn new() -> Self {
        Self
    }

    /// Check whether `(org_id, request_id)` already has an engagement row.
    ///
    /// Returns:
    /// - [`IdempotencyResult::New`] if no row exists.
    /// - [`IdempotencyResult::Replay`] if the row exists and the hash matches.
    /// - [`IdempotencyResult::Conflict`] if the row exists but the hash differs.
    pub async fn check(
        &self,
        pool: &sqlx::PgPool,
        org_id: Uuid,
        request_id: Uuid,
        computed_hash: &PayloadHash,
    ) -> Result<IdempotencyResult, sqlx::Error> {
        // Statuses considered terminal: 4 = SUCCESS, 5 = CANCELLED, 6 = FAILED
        // (These match the `status` SMALLINT on the engagements table.)
        const TERMINAL_STATUSES: &[i16] = &[4, 5, 6];

        let row: Option<(Vec<u8>, i16, Uuid)> = sqlx::query_as(
            r#"
            SELECT payload_hash, status, engagement_id
            FROM   engagements
            WHERE  organization_id = $1
              AND  request_id      = $2
            LIMIT 1
            "#,
        )
        .bind(org_id)
        .bind(request_id)
        .fetch_optional(pool)
        .await?;

        match row {
            None => Ok(IdempotencyResult::New),
            Some((stored_hash_bytes, status, engagement_id)) => {
                let matches = stored_hash_bytes == computed_hash.as_bytes();
                if matches {
                    Ok(IdempotencyResult::Replay {
                        engagement_id,
                        is_terminal: TERMINAL_STATUSES.contains(&status),
                    })
                } else {
                    Ok(IdempotencyResult::Conflict)
                }
            }
        }
    }
}

impl Default for IdempotencyChecker {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_fields() -> StartEngagementFields {
        StartEngagementFields {
            org_id: "00000000-0000-0000-0000-000000000001".into(),
            channel: "1".into(),
            mode: "1".into(),
            journey_version: "v1.0.0".into(),
            contact_kind: "2".into(),
            contact_id: None,
            contact_phone: Some("+60126013446".into()),
            batch_id: None,
            voice_profile_id: None,
            test_mode: false,
            language: Some("en-MY".into()),
        }
    }

    // --- canonical JSON correctness ---

    #[test]
    fn canonical_json_keys_are_sorted() {
        let fields = sample_fields();
        let json = canonical_json_start_engagement(&fields);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let obj = parsed.as_object().unwrap();
        let keys: Vec<&str> = obj.keys().map(|s| s.as_str()).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(
            keys, sorted,
            "keys must be lexicographically sorted in canonical JSON"
        );
    }

    #[test]
    fn canonical_json_omits_absent_optional_fields() {
        let fields = sample_fields(); // batch_id, voice_profile_id = None
        let json = canonical_json_start_engagement(&fields);
        assert!(
            !json.contains("batch_id"),
            "absent batch_id must not appear in canonical JSON"
        );
        assert!(
            !json.contains("voice_profile_id"),
            "absent voice_profile_id must not appear in canonical JSON"
        );
    }

    #[test]
    fn canonical_json_includes_present_optional_fields() {
        let fields = sample_fields(); // contact_phone and language are Some
        let json = canonical_json_start_engagement(&fields);
        assert!(
            json.contains("contact_phone"),
            "present contact_phone must appear in canonical JSON"
        );
        assert!(
            json.contains("en-MY"),
            "language value must appear in canonical JSON"
        );
    }

    #[test]
    fn canonical_json_no_insignificant_whitespace() {
        let fields = sample_fields();
        let json = canonical_json_start_engagement(&fields);
        assert!(
            !json.contains(": "),
            "canonical JSON must not have space after colon"
        );
        assert!(
            !json.contains(", "),
            "canonical JSON must not have space after comma"
        );
    }

    // --- hash stability (same input → same hash) ---

    #[test]
    fn hash_is_deterministic() {
        let f1 = sample_fields();
        let f2 = sample_fields();
        assert_eq!(
            canonical_hash_start_engagement(&f1),
            canonical_hash_start_engagement(&f2)
        );
    }

    #[test]
    fn hash_changes_when_field_changes() {
        let f1 = sample_fields();
        let mut f2 = sample_fields();
        f2.channel = "2".into();
        assert_ne!(
            canonical_hash_start_engagement(&f1),
            canonical_hash_start_engagement(&f2)
        );
    }

    #[test]
    fn hash_changes_when_optional_field_added() {
        let f1 = sample_fields(); // batch_id = None
        let mut f2 = sample_fields();
        f2.batch_id = Some(Uuid::new_v4().to_string()); // batch_id = Some
        assert_ne!(
            canonical_hash_start_engagement(&f1),
            canonical_hash_start_engagement(&f2)
        );
    }

    #[test]
    fn hash_excludes_metadata_and_display_name() {
        let fields = sample_fields();
        let json = canonical_json_start_engagement(&fields);
        assert!(
            !json.contains("metadata"),
            "metadata must be excluded from canonical JSON"
        );
        assert!(
            !json.contains("display_name"),
            "display_name must be excluded from canonical JSON"
        );
    }

    // --- PayloadHash helpers ---

    #[test]
    fn payload_hash_roundtrip_via_vec() {
        let fields = sample_fields();
        let hash = canonical_hash_start_engagement(&fields);
        let bytes = hash.as_bytes().to_vec();
        let recovered = PayloadHash::from_vec(bytes).unwrap();
        assert_eq!(hash, recovered);
    }

    #[test]
    fn payload_hash_from_vec_rejects_wrong_length() {
        let bad: Vec<u8> = vec![0u8; 16]; // 16 bytes, not 32
        assert!(PayloadHash::from_vec(bad).is_none());
    }

    #[test]
    fn payload_hash_hex_is_64_chars() {
        let fields = sample_fields();
        let hash = canonical_hash_start_engagement(&fields);
        let hex = hash.to_hex();
        assert_eq!(hex.len(), 64, "SHA-256 hex must be 64 chars");
        assert!(
            hex.chars().all(|c| c.is_ascii_hexdigit()),
            "hex must only contain hex chars"
        );
    }

    // --- IdempotencyResult equality ---

    #[test]
    fn idempotency_result_variants_are_distinguishable() {
        let eid = Uuid::new_v4();
        let r1 = IdempotencyResult::New;
        let r2 = IdempotencyResult::Replay {
            engagement_id: eid,
            is_terminal: false,
        };
        let r3 = IdempotencyResult::Conflict;
        assert_ne!(r1, r2);
        assert_ne!(r1, r3);
        assert_ne!(r2, r3);
    }
}
