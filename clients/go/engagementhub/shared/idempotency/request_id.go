package idempotency

import "github.com/google/uuid"

// NamespaceOutbound is the fixed UUIDv5 namespace for outbound dispatcher idempotency keys.
//
// Rust mirror:
//
//	use uuid::{uuid, Uuid};
//	const NAMESPACE_OUTBOUND: Uuid = uuid!("ba40c89b-d320-47cd-aa7c-c05c3b24dd6a");
var NamespaceOutbound = uuid.MustParse("ba40c89b-d320-47cd-aa7c-c05c3b24dd6a")

// DeriveRequestID returns a deterministic UUIDv5 request_id for the given outbound
// campaign attempt. Derivation: UUIDv5(NamespaceOutbound, batchID+":"+contactNumber+":"+attemptNumber).
// attemptNumber must be the plain decimal string of the attempt integer ("1", "2", ...).
// Inputs must not contain ":". Panics if any argument is empty.
func DeriveRequestID(batchID, contactNumber, attemptNumber string) string {
	if batchID == "" || contactNumber == "" || attemptNumber == "" {
		panic("idempotency.DeriveRequestID: all arguments must be non-empty")
	}
	return uuid.NewSHA1(NamespaceOutbound, []byte(batchID+":"+contactNumber+":"+attemptNumber)).String()
}
