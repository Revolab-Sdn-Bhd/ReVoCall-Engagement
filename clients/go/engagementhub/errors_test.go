package engagementhub_test

import (
	"errors"
	"fmt"
	"testing"

	"connectrpc.com/connect"
	eh "github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/clients/go/engagementhub"
	engagementv1 "github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/clients/go/engagementhub/internal/gen/revocall/engagement/v1"
)

func TestError_Format(t *testing.T) {
	e := &eh.Error{Code: eh.CodeRequestIDConflict, Message: "duplicate"}
	want := "engagement_hub: ENGAGEMENT_ERROR_CODE_REQUEST_ID_CONFLICT: duplicate"
	if got := e.Error(); got != want {
		t.Fatalf("Error() = %q, want %q", got, want)
	}

	e2 := &eh.Error{Code: eh.CodeRegistryUnavailable, Message: "down", DownstreamService: "registry"}
	want2 := "engagement_hub: ENGAGEMENT_ERROR_CODE_REGISTRY_UNAVAILABLE: down (downstream=registry)"
	if got := e2.Error(); got != want2 {
		t.Fatalf("Error() with downstream = %q, want %q", got, want2)
	}
}

func TestError_Unwrap(t *testing.T) {
	cause := errors.New("underlying")
	e := eh.NewError(eh.CodeInternal, "boom", cause)
	if !errors.Is(e, cause) {
		t.Fatal("errors.Is(e, cause) should be true via Unwrap")
	}
}

func TestErrorsIs_MatchesSentinelThroughWrap(t *testing.T) {
	e := &eh.Error{Code: eh.CodeRequestIDConflict, Message: "x"}
	wrapped := fmt.Errorf("rpc failed: %w", e)
	if !errors.Is(wrapped, eh.ErrRequestIDConflict) {
		t.Fatal("errors.Is should match wrapped sentinel by code")
	}
}

func TestError_Is_RejectsNonError(t *testing.T) {
	e := &eh.Error{Code: eh.CodeInternal}
	other := errors.New("unrelated")
	if errors.Is(e, other) {
		t.Fatal("errors.Is(*Error, non-*Error) should return false")
	}
}

func TestSentinels_AllDistinct(t *testing.T) {
	sentinels := []*eh.Error{
		eh.ErrRouteResolutionFailed,
		eh.ErrJourneyVersionNotFound,
		eh.ErrTelephonyNotAvailable,
		eh.ErrVoiceProfileNotFound,
		eh.ErrVoiceSessionRejected,
		eh.ErrJourneyExecutionRejected,
		eh.ErrRegistryUnavailable,
		eh.ErrContactUnreachable,
		eh.ErrCallEndedWithError,
		eh.ErrOrgQuotaExceeded,
		eh.ErrEngagementNotFound,
		eh.ErrEngagementAlreadyTerminal,
		eh.ErrRequestIDConflict,
		eh.ErrInternal,
	}
	if len(sentinels) != 14 {
		t.Fatalf("expected 14 sentinels, got %d", len(sentinels))
	}
	seen := map[eh.EngagementErrorCode]bool{}
	for _, s := range sentinels {
		if int32(s.Code) == 0 {
			t.Fatalf("sentinel has zero (UNSPECIFIED) code: %+v", s)
		}
		if seen[s.Code] {
			t.Fatalf("duplicate sentinel code: %v", s.Code)
		}
		seen[s.Code] = true
	}
}

func TestClassifiers_TruthTable(t *testing.T) {
	cases := []struct {
		code      eh.EngagementErrorCode
		transient bool
		terminal  bool
		client    bool
		server    bool
	}{
		{eh.CodeRouteResolutionFailed, false, false, true, false},
		{eh.CodeJourneyVersionNotFound, false, false, true, false},
		{eh.CodeTelephonyNotAvailable, false, false, true, false},
		{eh.CodeVoiceProfileNotFound, false, false, true, false},
		{eh.CodeVoiceSessionRejected, false, false, true, false},
		{eh.CodeJourneyExecutionRejected, false, false, true, false},
		{eh.CodeRegistryUnavailable, true, false, false, true},
		{eh.CodeContactUnreachable, false, true, true, false},
		{eh.CodeCallEndedWithError, false, false, true, false},
		{eh.CodeOrgQuotaExceeded, false, true, true, false},
		{eh.CodeEngagementNotFound, false, true, true, false},
		{eh.CodeEngagementAlreadyTerminal, false, true, true, false},
		{eh.CodeRequestIDConflict, false, true, true, false},
		{eh.CodeInternal, true, false, false, true},
		// UNSPECIFIED — must not be classified as client/server/transient/terminal
		{eh.EngagementErrorCode(0), false, false, false, false},
	}
	for _, c := range cases {
		e := &eh.Error{Code: c.code}
		if got := eh.IsTransient(e); got != c.transient {
			t.Errorf("IsTransient(%v) = %v, want %v", c.code, got, c.transient)
		}
		if got := eh.IsTerminal(e); got != c.terminal {
			t.Errorf("IsTerminal(%v) = %v, want %v", c.code, got, c.terminal)
		}
		if got := eh.IsClientError(e); got != c.client {
			t.Errorf("IsClientError(%v) = %v, want %v", c.code, got, c.client)
		}
		if got := eh.IsServerError(e); got != c.server {
			t.Errorf("IsServerError(%v) = %v, want %v", c.code, got, c.server)
		}
	}
}

func TestClassifiers_NonEngagementError(t *testing.T) {
	plain := errors.New("plain")
	if eh.IsTransient(plain) || eh.IsTerminal(plain) || eh.IsClientError(plain) || eh.IsServerError(plain) {
		t.Fatal("plain error should not be classified")
	}
	if eh.IsTransient(nil) || eh.IsTerminal(nil) || eh.IsClientError(nil) || eh.IsServerError(nil) {
		t.Fatal("nil should not be classified")
	}
}

func TestClassifiers_UnwrapsThroughWrap(t *testing.T) {
	e := &eh.Error{Code: eh.CodeRegistryUnavailable}
	wrapped := fmt.Errorf("rpc failed: %w", e)
	if !eh.IsTransient(wrapped) {
		t.Fatal("IsTransient should unwrap through fmt.Errorf wrap")
	}
}

func TestFromConnectError_HappyPath(t *testing.T) {
	downstream := "engagement_hub"
	proto := &engagementv1.EngagementError{
		Code:              engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_REQUEST_ID_CONFLICT,
		Message:           "duplicate request_id",
		DownstreamService: &downstream,
		Details:           map[string]string{"existing_engagement_id": "eng-123"},
	}
	connErr := connect.NewError(connect.CodeAlreadyExists, errors.New("dup"))
	detail, err := connect.NewErrorDetail(proto)
	if err != nil {
		t.Fatalf("NewErrorDetail: %v", err)
	}
	connErr.AddDetail(detail)

	got, ok := eh.FromConnectError(connErr)
	if !ok {
		t.Fatal("expected ok=true")
	}
	if got.Code != eh.CodeRequestIDConflict {
		t.Errorf("Code = %v, want %v", got.Code, eh.CodeRequestIDConflict)
	}
	if got.Message != "duplicate request_id" {
		t.Errorf("Message = %q", got.Message)
	}
	if got.DownstreamService != "engagement_hub" {
		t.Errorf("DownstreamService = %q", got.DownstreamService)
	}
	if got.Details["existing_engagement_id"] != "eng-123" {
		t.Errorf("Details lost: %+v", got.Details)
	}
	if !errors.Is(got, connErr) {
		t.Fatal("FromConnectError should preserve original error as cause")
	}
	if !errors.Is(got, eh.ErrRequestIDConflict) {
		t.Fatal("converted error should match sentinel via errors.Is")
	}
}

func TestFromConnectError_NoDetail(t *testing.T) {
	connErr := connect.NewError(connect.CodeUnavailable, errors.New("down"))
	if _, ok := eh.FromConnectError(connErr); ok {
		t.Fatal("expected ok=false when connect error has no EngagementError detail")
	}
}

func TestFromConnectError_PlainError(t *testing.T) {
	if _, ok := eh.FromConnectError(errors.New("plain")); ok {
		t.Fatal("expected ok=false for plain (non-connect) error")
	}
}

func TestFromConnectError_NilError(t *testing.T) {
	if _, ok := eh.FromConnectError(nil); ok {
		t.Fatal("expected ok=false for nil")
	}
}

func TestFromConnectError_UnwrapsThroughWrap(t *testing.T) {
	proto := &engagementv1.EngagementError{
		Code:    engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_INTERNAL,
		Message: "boom",
	}
	connErr := connect.NewError(connect.CodeInternal, errors.New("x"))
	detail, _ := connect.NewErrorDetail(proto)
	connErr.AddDetail(detail)
	wrapped := fmt.Errorf("wrap: %w", connErr)

	got, ok := eh.FromConnectError(wrapped)
	if !ok {
		t.Fatal("FromConnectError should unwrap to find *connect.Error")
	}
	if got.Code != eh.CodeInternal {
		t.Errorf("Code = %v, want %v", got.Code, eh.CodeInternal)
	}
}

// TestClassifiers_CoverAllCodes asserts that every non-UNSPECIFIED proto code
// is classified by at least one of IsTransient / IsTerminal, OR is one of the
// three codes (VoiceSessionRejected, JourneyExecutionRejected, CallEndedWithError)
// intentionally classified as neither. If a new proto code is added without
// updating the classifiers, this test fails.
func TestClassifiers_CoverAllCodes(t *testing.T) {
	intentionallyNeither := map[eh.EngagementErrorCode]bool{
		// Pure client errors: neither transient (retriable) nor terminal (never-succeed).
		// They are caller-attributable but may succeed on a corrected retry.
		eh.CodeRouteResolutionFailed:    true,
		eh.CodeJourneyVersionNotFound:   true,
		eh.CodeTelephonyNotAvailable:    true,
		eh.CodeVoiceProfileNotFound:     true,
		eh.CodeVoiceSessionRejected:     true,
		eh.CodeJourneyExecutionRejected: true,
		eh.CodeCallEndedWithError:       true,
	}
	for codeInt, name := range engagementv1.EngagementErrorCode_name {
		if codeInt == 0 {
			continue // skip UNSPECIFIED
		}
		code := eh.EngagementErrorCode(codeInt)
		e := &eh.Error{Code: code}
		transient := eh.IsTransient(e)
		terminal := eh.IsTerminal(e)
		if !transient && !terminal && !intentionallyNeither[code] {
			t.Errorf("code %s (%d) is neither transient nor terminal and not in intentionallyNeither set — update classifiers or the test", name, codeInt)
		}
		if transient && terminal {
			t.Errorf("code %s (%d) is both transient and terminal — classifications must be mutually exclusive", name, codeInt)
		}
	}
}
