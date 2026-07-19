//! Nexus `EventEnvelope` → [`CapabilityEventReceived`] mapping, shared by
//! [`crate::reactions`]'s inbound push receiver (ADR-018).
//!
//! `bff_core::event_ingestion` owns everything capability-agnostic — the
//! `CapabilityEventReceived` envelope, the classify-and-route decision, the
//! idempotent-ingestion service, and the `EventPublisher` trait it publishes
//! a freshly-inserted aggregate to. This module owns the one part that
//! belongs in `bff-api` instead (ADR-004): translating nexus's own wire
//! shape into that envelope.
//!
//! # History: from polling to push (ADR-018 supersedes this module's
//! original PROMPT-30/ADR-011 polling-loop design)
//! This module used to *also* own a background polling loop
//! (`run_polling_loop`) that called a guessed `GET .../events/poll` route on
//! an interval. That route never existed on the real, deployed nexus-server
//! — confirmed live via nexus's own public `/openapi.json`, which lists no
//! consumer-facing poll route at all, only `POST /api/v1/events` (event
//! *ingestion into* nexus, the opposite direction) — and nexus's real
//! architecture turned out to be push-based reaction dispatch (nexus calls
//! *out* to a registered `consumer_repo`'s `base_url`, confirmed by reading
//! the live `nexus-server` image's own `config/registries/*.json` and
//! binary strings). See `ADR-018-nexus-event-delivery-push-reaction.md` for
//! the full writeup. The polling loop and its transport/cursor plumbing
//! (`EVENTS_POLL_PATH`, `build_poll_path`, `fetch_events`, `poll_once`,
//! `run_polling_loop`) were removed with that ADR — what remains here (the
//! `EventEnvelope` mirror and [`envelope_into_received`]) is exactly the
//! part that's still correct and reused: nexus delivers the same wire shape
//! either way, only the transport direction changed.
//!
//! ## Response shape: nexus `EventEnvelope`, mapped to `CapabilityEventReceived`
//! [`crate::reactions`]'s handler deserializes an inbound push's body into
//! the local [`EventEnvelope`] mirror (independent-repo, no cross-Cargo-dep,
//! same pattern as ADR-029's capability structs) and maps it into a
//! [`CapabilityEventReceived`] via [`envelope_into_received`]. The
//! envelope-level fields map 1:1 (`producer_repo`→`origin_capability`,
//! `event_id`→`origin_event_id`, `event_type`→`event_type`,
//! `occurred_at`→`received_at`); the BFF-domain projection fields
//! (`summary`, `deep_link`, `consultant_id`, `related_origin_event_id`,
//! `related_proposal_id`) are read from the envelope's event-type-specific
//! `payload`. That payload mapping is **provisional** — nexus's real
//! per-`event_type` payload schema isn't declared yet (this repo currently
//! consumes zero event types in nexus's `consumers.json`, so no real push
//! has ever been received to confirm it against), so it follows the same
//! "read what's structurally required from the rough payload and flag it
//! pending nexus's real contract" convention
//! [`CapabilityEventReceived::consultant_id`]/`related_proposal_id` already
//! document — see [`envelope_into_received`] for the exact field rules and
//! defaults.
//!
//! # Idempotency
//! A push can be retried by nexus (timeout, transient 5xx) — dedup relies
//! entirely on `bff_core::event_ingestion::ingest_events`'s `(origin_capability,
//! origin_event_id)` unique constraint (ADR-010, PROMPT-29): a redelivered
//! event only ever inserts a row, and publishes to the `EventPublisher`,
//! once. There is no cursor/watermark layer here the way the old polling
//! loop had one — a push has nothing to track "how far read" against, the
//! idempotent save is the only dedup layer needed or possible.

use bff_core::CapabilityEventReceived;
use chrono::{DateTime, Utc};

/// Local mirror of nexus's `EventEnvelope` wire shape, just the fields this
/// consumer reads. Only the envelope-level fields mapped 1:1 plus the
/// event-type-specific `payload` are declared; every other envelope field
/// (`event_version`, `aggregate_id`/`aggregate_type`, `actor`,
/// `organization_id`, `causation_id`, `correlation_id`, `sequence_number`,
/// `metadata`) is intentionally ignored via serde's default unknown-field
/// tolerance — so this struct needs no `nexus_contracts` dependency and is
/// unaffected by e.g. `Semver`'s exact serde repr. A local plain-serde
/// struct, never a cross-repo Rust dependency (ADR-007/ADR-029).
#[derive(Debug, serde::Deserialize)]
pub(crate) struct EventEnvelope {
    pub(crate) event_id: String,
    pub(crate) event_type: String,
    pub(crate) occurred_at: DateTime<Utc>,
    pub(crate) producer_repo: String,
    #[serde(default)]
    pub(crate) payload: serde_json::Value,
}

/// Reads a string field from an `EventEnvelope`'s `payload`, if present.
fn payload_str(payload: &serde_json::Value, key: &str) -> Option<String> {
    payload.get(key).and_then(serde_json::Value::as_str).map(str::to_owned)
}

/// Maps a nexus [`EventEnvelope`] into this repo's [`CapabilityEventReceived`]
/// projection (see the module docs' "Response shape" section).
///
/// Envelope-level fields map 1:1. The BFF-domain projection fields are read
/// from the event-type-specific `payload` by their `CapabilityEventReceived`
/// field name (`related_proposal_id` also accepts a bare `proposal_id`, the
/// key nexus's Legal events are expected to carry). This payload mapping is
/// **provisional** pending nexus's real per-`event_type` payload schema — no
/// real push has been received yet to confirm it against. Defaults for the
/// two required fields a payload might omit:
/// - `summary` falls back to `event_type` — an unrecognized/summary-less
///   event still surfaces something display-safe rather than an empty body.
/// - `consultant_id` falls back to `""` — an event that names no consultant
///   in its payload cannot be routed to one; the empty id is a documented,
///   inert placeholder (it simply won't match any real consultant's feed)
///   rather than a guess, and will be revisited once the real payload schema
///   is known. Matches this file's existing conservative-provisional
///   convention rather than dropping the event silently.
pub(crate) fn envelope_into_received(env: EventEnvelope) -> CapabilityEventReceived {
    let EventEnvelope { event_id, event_type, occurred_at, producer_repo, payload } = env;
    let summary = payload_str(&payload, "summary").unwrap_or_else(|| event_type.clone());
    CapabilityEventReceived {
        origin_capability: producer_repo,
        origin_event_id: event_id,
        event_type,
        summary,
        deep_link: payload_str(&payload, "deep_link"),
        received_at: occurred_at,
        consultant_id: payload_str(&payload, "consultant_id").unwrap_or_default(),
        related_origin_event_id: payload_str(&payload, "related_origin_event_id"),
        related_proposal_id: payload_str(&payload, "related_proposal_id")
            .or_else(|| payload_str(&payload, "proposal_id")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Pure unit tests for the EventEnvelope -> CapabilityEventReceived
    // mapping (no Postgres/container needed). The mapping itself is
    // untouched by ADR-018's push-vs-poll transport change — nexus still
    // delivers this exact wire shape, just via `crate::reactions` now
    // instead of a polling loop. ---

    /// The full real `EventEnvelope` shape, including the fields the
    /// mapping ignores — proves they neither break deserialization nor leak
    /// into [`CapabilityEventReceived`].
    fn event_batch_body() -> serde_json::Value {
        serde_json::json!([
            {
                "event_id": "cra-1",
                "event_type": "collaboration_request_acknowledged",
                "event_version": {"major": 1, "minor": 0, "patch": 0},
                "occurred_at": "2026-01-01T00:00:00Z",
                "producer_repo": "sales",
                "aggregate_id": "collab-1",
                "aggregate_type": "collaboration_request",
                "organization_id": "org-1",
                "actor": {"user_id": "sales-user-9", "service_account": null, "role": "sales-rep"},
                "correlation_id": "corr-1",
                "payload": {
                    "summary": "Sales acknowledged your collaboration request.",
                    "deep_link": "https://app.example.com/sales/collab/1",
                    "consultant_id": "consultant-1"
                },
                "metadata": {}
            }
        ])
    }

    fn envelope_from(value: serde_json::Value) -> EventEnvelope {
        serde_json::from_value(value).expect("valid EventEnvelope")
    }

    #[test]
    fn maps_envelope_level_fields_and_reads_projection_fields_from_payload() {
        // The full real EventEnvelope shape, including the fields the mapping
        // ignores — proves they neither break deserialization nor leak in.
        let batch = event_batch_body();
        let env = envelope_from(batch.as_array().unwrap()[0].clone());
        let received = envelope_into_received(env);

        assert_eq!(received.origin_capability, "sales"); // <- producer_repo
        assert_eq!(received.origin_event_id, "cra-1"); // <- event_id
        assert_eq!(received.event_type, "collaboration_request_acknowledged");
        assert_eq!(received.received_at, "2026-01-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap()); // <- occurred_at
        assert_eq!(received.summary, "Sales acknowledged your collaboration request."); // <- payload
        assert_eq!(received.deep_link.as_deref(), Some("https://app.example.com/sales/collab/1")); // <- payload
        assert_eq!(received.consultant_id, "consultant-1"); // <- payload
        assert_eq!(received.related_origin_event_id, None);
        assert_eq!(received.related_proposal_id, None);
    }

    #[test]
    fn summary_falls_back_to_event_type_when_payload_omits_it() {
        let env = envelope_from(serde_json::json!({
            "event_id": "e-1",
            "event_type": "task_assigned",
            "occurred_at": "2026-01-01T00:00:00Z",
            "producer_repo": "execution",
            "payload": { "consultant_id": "consultant-1" }
        }));
        let received = envelope_into_received(env);
        assert_eq!(received.summary, "task_assigned", "summary should fall back to event_type");
        assert_eq!(received.consultant_id, "consultant-1");
    }

    #[test]
    fn consultant_id_defaults_to_empty_when_payload_omits_it() {
        let env = envelope_from(serde_json::json!({
            "event_id": "e-2",
            "event_type": "proposal_status_changed",
            "occurred_at": "2026-01-01T00:00:00Z",
            "producer_repo": "commit",
            "payload": { "summary": "Status changed." }
        }));
        let received = envelope_into_received(env);
        assert_eq!(received.consultant_id, "", "a payload with no consultant_id yields the documented empty placeholder");
        assert_eq!(received.summary, "Status changed.");
    }

    #[test]
    fn related_proposal_id_reads_the_bare_proposal_id_alias_from_payload() {
        let env = envelope_from(serde_json::json!({
            "event_id": "e-3",
            "event_type": "legal_clause_updated",
            "occurred_at": "2026-01-01T00:00:00Z",
            "producer_repo": "legal",
            "payload": { "summary": "Clause updated.", "consultant_id": "consultant-1", "proposal_id": "prop-7" }
        }));
        let received = envelope_into_received(env);
        assert_eq!(received.related_proposal_id.as_deref(), Some("prop-7"));
    }
}
