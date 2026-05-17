package idempotency_test

import (
	"regexp"
	"testing"

	"github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/clients/go/engagementhub/shared/idempotency"
)

var uuidRE = regexp.MustCompile(`^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`)

func TestDeriveRequestID_Determinism(t *testing.T) {
	first := idempotency.DeriveRequestID("batch-123", "+60126013446", "1")
	for i := 0; i < 1000; i++ {
		got := idempotency.DeriveRequestID("batch-123", "+60126013446", "1")
		if got != first {
			t.Fatalf("non-deterministic: iteration %d got %s, want %s", i, got, first)
		}
	}
}

func TestDeriveRequestID_CrossAttemptCollision(t *testing.T) {
	a1 := idempotency.DeriveRequestID("batch-123", "+60126013446", "1")
	a2 := idempotency.DeriveRequestID("batch-123", "+60126013446", "2")
	if a1 == a2 {
		t.Fatalf("attempt 1 and attempt 2 produced the same UUID: %s", a1)
	}
}

func TestDeriveRequestID_CrossContactCollision(t *testing.T) {
	c1 := idempotency.DeriveRequestID("batch-123", "+60126013446", "1")
	c2 := idempotency.DeriveRequestID("batch-123", "+60126013447", "1")
	if c1 == c2 {
		t.Fatalf("different contacts produced the same UUID: %s", c1)
	}
}

func TestDeriveRequestID_RFC4122Format(t *testing.T) {
	got := idempotency.DeriveRequestID("batch-abc", "+60126013446", "1")
	if !uuidRE.MatchString(got) {
		t.Fatalf("output %q is not lowercase hyphenated RFC 4122 UUID", got)
	}
}

func TestDeriveRequestID_KnownValue(t *testing.T) {
	// Pin against a computed-once value. If this fails after a dep upgrade,
	// the derivation contract has changed — investigate before proceeding.
	const want = "03518426-c533-5d8f-bbb9-f8ad0c139ffb"
	got := idempotency.DeriveRequestID("batch-abc", "+60126013446", "1")
	if got != want {
		t.Fatalf("known-value regression: got %s, want %s", got, want)
	}
}

func TestDeriveRequestID_PanicsOnEmptyBatchID(t *testing.T) {
	defer func() {
		if r := recover(); r == nil {
			t.Fatal("expected panic for empty batchID, got none")
		}
	}()
	idempotency.DeriveRequestID("", "+60126013446", "1")
}

func TestDeriveRequestID_PanicsOnEmptyContactNumber(t *testing.T) {
	defer func() {
		if r := recover(); r == nil {
			t.Fatal("expected panic for empty contactNumber, got none")
		}
	}()
	idempotency.DeriveRequestID("batch-123", "", "1")
}

func TestDeriveRequestID_PanicsOnEmptyAttemptNumber(t *testing.T) {
	defer func() {
		if r := recover(); r == nil {
			t.Fatal("expected panic for empty attemptNumber, got none")
		}
	}()
	idempotency.DeriveRequestID("batch-123", "+60126013446", "")
}
