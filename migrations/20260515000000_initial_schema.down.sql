DROP TABLE IF EXISTS engagement_audit;

DROP TRIGGER IF EXISTS engagement_events_notify ON engagement_events;
DROP FUNCTION IF EXISTS trg_notify_engagement_event();
DROP TABLE IF EXISTS engagement_events;

DROP TABLE IF EXISTS route_resolutions;
DROP TABLE IF EXISTS engagement_invocations;

DROP TRIGGER IF EXISTS engagements_set_updated_at ON engagements;
DROP TABLE IF EXISTS engagements;

DROP FUNCTION IF EXISTS trg_set_updated_at();
