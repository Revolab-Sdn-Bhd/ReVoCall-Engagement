-- ReVoCall Engagement Hub — initial schema (PRD §11)
-- Forward-only in prod; this .up.sql + the matching .down.sql exist for local dev rollback.

CREATE OR REPLACE FUNCTION trg_set_updated_at() RETURNS trigger AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TABLE engagements (
    engagement_id            UUID PRIMARY KEY,
    organization_id          UUID NOT NULL,
    request_id               UUID NOT NULL,
    payload_hash             BYTEA NOT NULL,
    channel                  SMALLINT NOT NULL,
    mode                     SMALLINT NOT NULL,
    journey_id               TEXT NOT NULL,
    journey_version          TEXT NOT NULL,
    snapshot_id              UUID,
    contact_kind             SMALLINT NOT NULL,
    contact_id               UUID,
    contact_phone_e164       TEXT,
    contact_display_name     TEXT,
    batch_id                 UUID,
    created_by_kind          SMALLINT NOT NULL,
    created_by_id            TEXT NOT NULL,
    status                   SMALLINT NOT NULL,
    status_reason            TEXT,
    error_code               SMALLINT,
    error_message            TEXT,
    error_downstream         TEXT,
    error_details_json       JSONB,
    metadata                 JSONB NOT NULL DEFAULT '{}',
    trace_context            JSONB,
    voice_session_ref        TEXT,
    voice_session_unbound_at TIMESTAMPTZ,
    journey_execution_ref    TEXT,
    journey_cleanup_done_at  TIMESTAMPTZ,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at               TIMESTAMPTZ,
    ended_at                 TIMESTAMPTZ,
    CONSTRAINT engagements_request_unique UNIQUE (organization_id, request_id),
    CONSTRAINT engagements_contact_check
        CHECK ((contact_kind = 1 AND contact_id IS NOT NULL)
            OR (contact_kind = 2 AND contact_phone_e164 IS NOT NULL))
);

CREATE INDEX engagements_org_updated_id_idx
    ON engagements (organization_id, updated_at DESC, engagement_id DESC);

CREATE INDEX engagements_batch_idx
    ON engagements (batch_id, updated_at DESC)
    WHERE batch_id IS NOT NULL;

CREATE INDEX engagements_active_idx
    ON engagements (status, updated_at)
    WHERE status IN (1, 2, 3);

CREATE INDEX engagements_contact_idx
    ON engagements (organization_id, contact_id)
    WHERE contact_id IS NOT NULL;

CREATE INDEX engagements_contact_phone_idx
    ON engagements (organization_id, contact_phone_e164)
    WHERE contact_phone_e164 IS NOT NULL;

CREATE INDEX engagements_orphan_sweep_idx
    ON engagements (updated_at)
    WHERE status = 5
      AND ((voice_session_ref IS NOT NULL AND voice_session_unbound_at IS NULL)
        OR (journey_execution_ref IS NOT NULL AND journey_cleanup_done_at IS NULL));

CREATE TRIGGER engagements_set_updated_at
    BEFORE UPDATE ON engagements
    FOR EACH ROW EXECUTE FUNCTION trg_set_updated_at();


CREATE TABLE engagement_invocations (
    invocation_id        UUID PRIMARY KEY,
    engagement_id        UUID NOT NULL REFERENCES engagements(engagement_id) ON DELETE CASCADE,
    channel              SMALLINT NOT NULL,
    channel_session_ref  TEXT NOT NULL,
    bound_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    unbound_at           TIMESTAMPTZ,
    unbound_reason       TEXT,
    CONSTRAINT engagement_invocations_session_unique UNIQUE (channel, channel_session_ref)
);

CREATE INDEX engagement_invocations_engagement_idx
    ON engagement_invocations (engagement_id);


CREATE TABLE route_resolutions (
    resolution_id              UUID PRIMARY KEY,
    engagement_id              UUID NOT NULL UNIQUE
                                  REFERENCES engagements(engagement_id) ON DELETE CASCADE,
    input_journey_id           TEXT NOT NULL,
    input_journey_version      TEXT NOT NULL,
    input_channel              SMALLINT NOT NULL,
    input_contact_kind         SMALLINT NOT NULL,
    input_contact_value        TEXT NOT NULL,
    resolved_snapshot_id       UUID NOT NULL,
    resolved_journey_version   TEXT NOT NULL,
    resolved_telephony_id      UUID,
    resolved_voice_profile_id  UUID,
    resolver_inputs            JSONB,
    resolved_at                TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX route_resolutions_snapshot_idx
    ON route_resolutions (resolved_snapshot_id);


CREATE TABLE engagement_events (
    event_id         UUID PRIMARY KEY,
    engagement_id    UUID NOT NULL REFERENCES engagements(engagement_id) ON DELETE CASCADE,
    organization_id  UUID NOT NULL,
    sequence         BIGINT NOT NULL,
    event_type       SMALLINT NOT NULL,
    occurred_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    status_after     SMALLINT NOT NULL,
    payload          JSONB NOT NULL DEFAULT '{}',
    error_code       SMALLINT,
    error_message    TEXT,
    source           SMALLINT NOT NULL,
    trace_context    JSONB,
    event_pk         BIGSERIAL NOT NULL UNIQUE,
    CONSTRAINT engagement_events_sequence_unique UNIQUE (engagement_id, sequence)
);

CREATE INDEX engagement_events_engagement_seq_idx
    ON engagement_events (engagement_id, sequence DESC);

CREATE INDEX engagement_events_batch_idx
    ON engagement_events (organization_id, occurred_at DESC);

CREATE INDEX engagement_events_event_pk_idx
    ON engagement_events (event_pk);

CREATE OR REPLACE FUNCTION trg_notify_engagement_event() RETURNS trigger AS $$
BEGIN
    PERFORM pg_notify('engagement_events',
        json_build_object(
            'engagement_id', NEW.engagement_id,
            'organization_id', NEW.organization_id,
            'batch_id', (SELECT batch_id FROM engagements WHERE engagement_id = NEW.engagement_id),
            'sequence', NEW.sequence,
            'event_pk', NEW.event_pk,
            'event_type', NEW.event_type,
            'traceparent', NEW.trace_context->>'traceparent'
        )::text
    );
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER engagement_events_notify
    AFTER INSERT ON engagement_events
    FOR EACH ROW EXECUTE FUNCTION trg_notify_engagement_event();


CREATE TABLE engagement_audit (
    audit_id                UUID PRIMARY KEY,
    occurred_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    finalized_at            TIMESTAMPTZ,
    organization_id         UUID,
    acting_principal_kind   SMALLINT NOT NULL,
    acting_principal_id     TEXT NOT NULL,
    acting_user_id          UUID,
    acting_via              TEXT NOT NULL,
    rpc_name                TEXT NOT NULL,
    engagement_id           UUID,
    request_id              UUID,
    outcome                 SMALLINT NOT NULL,
    error_code              TEXT,
    request_summary         JSONB,
    trace_id                TEXT,
    archived_at             TIMESTAMPTZ
);

CREATE INDEX engagement_audit_pending_idx
    ON engagement_audit (occurred_at)
    WHERE outcome = 0;

CREATE INDEX engagement_audit_org_time_idx ON engagement_audit (organization_id, occurred_at DESC);
CREATE INDEX engagement_audit_engagement_idx ON engagement_audit (engagement_id) WHERE engagement_id IS NOT NULL;
CREATE INDEX engagement_audit_trace_idx ON engagement_audit (trace_id);
