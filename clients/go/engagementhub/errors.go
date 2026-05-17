// Package engagementhub provides the Engagement Hub Go SDK.
package engagementhub

import (
	"errors"
	"fmt"

	"connectrpc.com/connect"
	engagementv1 "github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/clients/go/engagementhub/internal/gen/revocall/engagement/v1"
)

// EngagementErrorCode mirrors the wire-level error code enum from
// revocall.engagement.v1.EngagementError, re-exported so consumers never
// import internal/gen/... directly.
type EngagementErrorCode = engagementv1.EngagementErrorCode

// Short-form code constants for ergonomic comparison.
const (
	CodeRouteResolutionFailed     = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_ROUTE_RESOLUTION_FAILED
	CodeJourneyVersionNotFound    = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_JOURNEY_VERSION_NOT_FOUND
	CodeTelephonyNotAvailable     = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_TELEPHONY_NOT_AVAILABLE
	CodeVoiceProfileNotFound      = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_VOICE_PROFILE_NOT_FOUND
	CodeVoiceSessionRejected      = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_VOICE_SESSION_REJECTED
	CodeJourneyExecutionRejected  = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_JOURNEY_EXECUTION_REJECTED
	CodeRegistryUnavailable       = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_REGISTRY_UNAVAILABLE
	CodeContactUnreachable        = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_CONTACT_UNREACHABLE
	CodeCallEndedWithError        = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_CALL_ENDED_WITH_ERROR
	CodeOrgQuotaExceeded          = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_ORG_QUOTA_EXCEEDED
	CodeEngagementNotFound        = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_ENGAGEMENT_NOT_FOUND
	CodeEngagementAlreadyTerminal = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_ENGAGEMENT_ALREADY_TERMINAL
	CodeRequestIDConflict         = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_REQUEST_ID_CONFLICT
	CodeInternal                  = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_INTERNAL
)

// Error is the SDK's typed representation of revocall.engagement.v1.EngagementError.
// Returned by SDK RPC wrappers when the server attaches an EngagementError detail
// to a *connect.Error. Compare with errors.Is(err, ErrXxx) and classify with
// IsTransient / IsTerminal / IsClientError / IsServerError.
type Error struct {
	Code              EngagementErrorCode
	Message           string
	DownstreamService string
	Details           map[string]string
	cause             error
}

// NewError constructs an *Error with an optional underlying cause.
func NewError(code EngagementErrorCode, message string, cause error) *Error {
	return &Error{Code: code, Message: message, cause: cause}
}

// Error implements the error interface.
func (e *Error) Error() string {
	if e.DownstreamService != "" {
		return fmt.Sprintf("engagement_hub: %s: %s (downstream=%s)", e.Code, e.Message, e.DownstreamService)
	}
	return fmt.Sprintf("engagement_hub: %s: %s", e.Code, e.Message)
}

// Unwrap returns the underlying cause, enabling errors.Is/As chains.
func (e *Error) Unwrap() error { return e.cause }

// Is matches another *Error by Code only, so sentinels work through fmt.Errorf
// wrapping. Returns false for non-*Error targets.
func (e *Error) Is(target error) bool {
	t, ok := target.(*Error)
	if !ok {
		return false
	}
	return e.Code == t.Code
}

// Sentinels — one per non-UNSPECIFIED EngagementErrorCode. Use with errors.Is.
var (
	ErrRouteResolutionFailed     = &Error{Code: CodeRouteResolutionFailed}
	ErrJourneyVersionNotFound    = &Error{Code: CodeJourneyVersionNotFound}
	ErrTelephonyNotAvailable     = &Error{Code: CodeTelephonyNotAvailable}
	ErrVoiceProfileNotFound      = &Error{Code: CodeVoiceProfileNotFound}
	ErrVoiceSessionRejected      = &Error{Code: CodeVoiceSessionRejected}
	ErrJourneyExecutionRejected  = &Error{Code: CodeJourneyExecutionRejected}
	ErrRegistryUnavailable       = &Error{Code: CodeRegistryUnavailable}
	ErrContactUnreachable        = &Error{Code: CodeContactUnreachable}
	ErrCallEndedWithError        = &Error{Code: CodeCallEndedWithError}
	ErrOrgQuotaExceeded          = &Error{Code: CodeOrgQuotaExceeded}
	ErrEngagementNotFound        = &Error{Code: CodeEngagementNotFound}
	ErrEngagementAlreadyTerminal = &Error{Code: CodeEngagementAlreadyTerminal}
	ErrRequestIDConflict         = &Error{Code: CodeRequestIDConflict}
	ErrInternal                  = &Error{Code: CodeInternal}
)

// IsTransient reports whether err carries a code the retry middleware should
// safely re-attempt: registry outage or internal server error.
func IsTransient(err error) bool {
	var e *Error
	if !errors.As(err, &e) {
		return false
	}
	switch e.Code {
	case CodeRegistryUnavailable, CodeInternal:
		return true
	}
	return false
}

// IsTerminal reports whether err carries a code that indicates retrying with
// the same inputs will never succeed.
func IsTerminal(err error) bool {
	var e *Error
	if !errors.As(err, &e) {
		return false
	}
	switch e.Code {
	case CodeEngagementNotFound,
		CodeEngagementAlreadyTerminal,
		CodeContactUnreachable,
		CodeRequestIDConflict,
		CodeOrgQuotaExceeded:
		return true
	}
	return false
}

// IsClientError reports whether err is caller-attributable (every code except
// the two server-side codes).
func IsClientError(err error) bool {
	var e *Error
	if !errors.As(err, &e) {
		return false
	}
	if int32(e.Code) == 0 {
		return false
	}
	switch e.Code {
	case CodeRegistryUnavailable, CodeInternal:
		return false
	}
	return true
}

// IsServerError reports whether err is server-attributable.
func IsServerError(err error) bool {
	var e *Error
	if !errors.As(err, &e) {
		return false
	}
	switch e.Code {
	case CodeRegistryUnavailable, CodeInternal:
		return true
	}
	return false
}

// FromConnectError extracts a typed *Error from err if it (or any error in its
// chain) is a *connect.Error carrying a revocall.engagement.v1.EngagementError
// detail. The original err is preserved as the *Error's cause for Unwrap().
// Returns (nil, false) when no matching detail is found.
func FromConnectError(err error) (*Error, bool) {
	var connectErr *connect.Error
	if !errors.As(err, &connectErr) {
		return nil, false
	}
	for _, detail := range connectErr.Details() {
		msg, valErr := detail.Value()
		if valErr != nil {
			continue
		}
		protoErr, ok := msg.(*engagementv1.EngagementError)
		if !ok {
			continue
		}
		e := &Error{
			Code:    protoErr.Code,
			Message: protoErr.Message,
			Details: protoErr.Details,
			cause:   err,
		}
		if protoErr.DownstreamService != nil {
			e.DownstreamService = *protoErr.DownstreamService
		}
		return e, true
	}
	return nil, false
}
