//! Landscape ACL gateway (ADR-007, ADR-016, PROMPT-40).
//!
//! Landscape owns Market Intelligence; this repo never re-derives or
//! overrides what counts as "approved" intelligence — it only reads the
//! already-approved digest and, in the one direction this ACL is not purely
//! a consumer, submits field observations *for Landscape to adjudicate*
//! (`../../.plans/ddd/anti-corruption-layers.md` §8: "this is the one
//! context where this repo is a minor upstream contributor as well as a
//! consumer... but Landscape still governs what counts as 'approved' — this
//! repo's ACL has no concept of publishing directly"). [`LandscapeGateway`]
//! is a thin translation boundary over two outbound calls, mirroring
//! [`crate::execution::ExecutionGateway`]'s shape exactly (see that module's
//! docs for the pattern this one replicates): one idempotent read
//! ([`LandscapeGateway::request_intelligence_digest`]) and one non-
//! idempotent command ([`LandscapeGateway::submit_field_observation`]).
//!
//! # Request paths: provisional, matching Execution's `.../v1/...` convention
//! Nexus's real Landscape contract is not finalized. This gateway assumes:
//! - `GET landscape/v1/intelligence` — response an envelope
//!   `{"items": [IntelligenceDigestItem, ...]}`, matching
//!   [`crate::execution::NexusExecutionGateway`]'s `EngagementsEnvelope`
//!   convention. No `consultant_id` query parameter: `anti-corruption-layers.md`
//!   §8 describes this as an *approved, published* digest (the "our term:
//!   Market Intelligence Digest" line), not a per-consultant-scoped read the
//!   way Capacity's/Customer's own reads are — the same "global reference
//!   data" shape [`crate::products::NexusProductsGateway::request_product_catalog`]
//!   already establishes for this repo's other unscoped read.
//! - `POST landscape/v1/observations` — body [`FieldObservationSubmission`]
//!   serialized directly (not nested inside a wrapper command struct — see
//!   the "`FieldObservationSubmission` is sent as-is" section below),
//!   fire-and-confirm (no documented ack body — see the "beyond the DDD doc"
//!   section below, same gap [`crate::sales::NexusSalesGateway`]'s module
//!   docs already call out for `request_collaboration`/`submit_referral`).
//!
//! Update these once Nexus's actual Landscape contract is known.
//!
//! # `FieldObservationSubmission` is sent as-is, not wrapped in a private command struct
//! Every other ACL's non-idempotent command (`CreateProposalCommand`,
//! `RequestCollaborationCommand`, `UpdateOwnProfileCommand`, ...) is a
//! *private*, gateway-internal struct with borrowed `&str` fields, kept
//! separate from any public DTO. Landscape's own worked example
//! (`anti-corruption-layers.md` §8) is different: it names exactly one
//! outbound shape, `FieldObservationSubmission { observation_text,
//! related_company_reference?, submitted_by }`, as *both* "crosses the
//! boundary as" and the field list `SubmitFieldObservationCommand` sends —
//! there is no second, richer command envelope described anywhere to wrap
//! it in. Rather than invent an envelope with no worked example to match
//! (the same "don't invent DTO fields with no worked example" discipline
//! [`crate::sales`]'s ack-response-shape doc comment already applies),
//! [`LandscapeGateway::submit_field_observation`] takes ownership of a
//! public [`FieldObservationSubmission`] value and serializes it directly as
//! the request body. This also gives [`FieldObservationSubmission`] the same
//! "pass the DTO itself, not its fields piecemeal" shape
//! [`crate::capacity::CapacityGateway::update_own_profile`] already
//! establishes for `ConsultantProfileIntake`.
//!
//! # `submitted_by`: always the caller's own session, never client input
//! `bff-api`'s handler for this call (`crate::bff_api::landscape`, this
//! repo's actual name `bff-api::landscape`) always constructs
//! [`FieldObservationSubmission::submitted_by`] from the authenticated
//! session's `consultant_id`, never from request-body input a consultant
//! could tamper with — the same "own data only, by construction" invariant
//! [`crate::capacity`]'s module docs establish for `ConsultantProfileIntake`.
//! This gateway itself has no way to enforce that (it just serializes
//! whatever [`FieldObservationSubmission`] it's given), so the invariant
//! lives at the BFF handler layer, same division of responsibility as every
//! other ACL in this repo (ADR-007: a narrow gateway trait shape, not
//! runtime filtering, is what's structurally enforceable here; the *caller*
//! is what's trusted to construct the argument honestly).
//!
//! # Transport-stack-assembly convention
//! Same convention as [`crate::execution::NexusExecutionGateway`]:
//! [`NexusLandscapeGateway::new`] takes an already-fully-decorated
//! `Arc<dyn NexusTransport>` and does not assemble the ADR-016
//! timeout/retry/circuit-breaker stack itself.
//!
//! # Timeout budget choice
//! `request_intelligence_digest` is a background/page-load read (the
//! consultant is not actively blocked on it mid-keystroke, and PROMPT-40's
//! own inbound-event handling explicitly treats this capability's updates as
//! "a low-priority refresh") — it uses
//! [`crate::timeout::DEFAULT_READ_TIMEOUT`], the same budget
//! `CommitGateway::list_proposals`/`ExecutionGateway::request_assigned_engagements`
//! use, and MAY be retried ([`crate::retry::RetryingTransport`]-wrapped)
//! since it has no side effect. `submit_field_observation` is a
//! consultant-initiated, side-effecting write — it uses
//! [`crate::timeout::DEFAULT_WRITE_TIMEOUT`] and must **never** be retried
//! (ADR-016: a retry against an unknown-outcome prior attempt risks
//! submitting a duplicate field observation to Landscape).
//!
//! # Two-gateway-instances-for-retry-safety convention
//! Exactly [`crate::execution::NexusExecutionGateway`]'s documented
//! constraint: because [`NexusLandscapeGateway`] holds one shared
//! `transport` field used by every trait method, one instance cannot safely
//! serve both retry profiles at once. `main.rs` therefore constructs **two**
//! `NexusLandscapeGateway` instances — `landscape_query_gateway`
//! (retry-wrapped, `request_intelligence_digest` only) and
//! `landscape_command_gateway` (no retry, `submit_field_observation` only) —
//! over the same base transport, mirroring the `execution_query_gateway`/
//! `execution_command_gateway` split exactly.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::transport::{CapabilityCall, CapabilityCaller, NexusTransport, NexusTransportError};

/// ADR-029 capability ids + target repo for this gateway's two calls.
const CAPABILITY_INTELLIGENCE: &str = "landscape.intelligence";
const CAPABILITY_OBSERVATIONS: &str = "landscape.observations";
const TARGET_REPO: &str = "cognitum-landscape";

/// Landscape's Market Intelligence Digest item projection
/// (`anti-corruption-layers.md` §8): this repo never models Landscape's full
/// intelligence-gathering/publishing pipeline — only this read projection of
/// already-*approved* items, plus a `deep_link` back into Landscape's own UI
/// for anything beyond it.
///
/// `Serialize` (alongside `Deserialize`, used to decode Landscape's
/// response) is derived so `bff-api` can relay this same shape verbatim to
/// the frontend, matching `ProposalSummary`'s/`EngagementSnapshot`'s "no BFF
/// re-shaping" convention.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IntelligenceDigestItem {
    pub intel_id: String,
    pub topic: String,
    pub summary: String,
    pub published_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deep_link: Option<String>,
}

/// Envelope this gateway expects `GET landscape/v1/intelligence`'s response
/// body to match. See the module docs for why an envelope (vs. a bare array)
/// was chosen (mirrors [`crate::execution::NexusExecutionGateway`]'s
/// `EngagementsEnvelope` rationale).
#[derive(Debug, serde::Deserialize)]
struct IntelligenceDigestEnvelope {
    items: Vec<IntelligenceDigestItem>,
}

/// Outbound submission: a consultant's own field observation, offered up to
/// Landscape for it to adjudicate — this repo never treats a submission as
/// itself "approved intelligence" (`anti-corruption-layers.md` §8's "this
/// repo's ACL has no concept of publishing directly"). Has a side effect
/// (creates a pending observation record in Landscape) — never
/// idempotent-safe to blindly retry. See the module docs for why this is
/// sent directly as the request body, and why `submitted_by` is always
/// caller-session-derived, never client input.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FieldObservationSubmission {
    pub observation_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_company_reference: Option<String>,
    pub submitted_by: String,
}

#[derive(Debug, thiserror::Error)]
pub enum LandscapeGatewayError {
    #[error(transparent)]
    Transport(#[from] NexusTransportError),
    #[error("Landscape returned a response body that did not match the expected shape: {0}")]
    UnexpectedResponseShape(#[source] serde_json::Error),
}

/// ACL over Landscape's Market Intelligence Digest and field-observation
/// capabilities. No re-adjudication of what counts as "approved" happens on
/// this trait — see the module docs.
#[async_trait]
pub trait LandscapeGateway: Send + Sync {
    /// Fetches the current approved Market Intelligence Digest, per
    /// `anti-corruption-layers.md` §8. A **query** in DDD terms — reading
    /// Landscape's current published-digest state has no side effect, so
    /// retrying it is safe/idempotent. See [`NexusLandscapeGateway`]'s doc
    /// comment for the transport requirement this method needs from its
    /// caller.
    async fn request_intelligence_digest(&self) -> Result<Vec<IntelligenceDigestItem>, LandscapeGatewayError>;

    /// Submits `submission` as a field observation for Landscape to
    /// adjudicate, per `anti-corruption-layers.md` §8's
    /// `SubmitFieldObservationCommand`. **Not idempotent-safe to retry** — a
    /// retry against an unknown-outcome prior attempt risks submitting a
    /// duplicate observation to Landscape.
    async fn submit_field_observation(&self, submission: FieldObservationSubmission) -> Result<(), LandscapeGatewayError>;
}

/// [`LandscapeGateway`] implementation backed by a [`NexusTransport`]. See
/// the module docs for the required transport decoration per method.
pub struct NexusLandscapeGateway {
    caller: CapabilityCaller,
}

impl NexusLandscapeGateway {
    /// See the module docs for the required transport decoration (read
    /// timeout + optional retry for `request_intelligence_digest`; write
    /// timeout, never retried, for `submit_field_observation`). The
    /// `transport` is wrapped in a [`CapabilityCaller`] so each method
    /// issues the ADR-029 capability envelope.
    pub fn new(transport: Arc<dyn NexusTransport>) -> Self {
        Self { caller: CapabilityCaller::new(transport) }
    }
}

#[async_trait]
impl LandscapeGateway for NexusLandscapeGateway {
    async fn request_intelligence_digest(&self) -> Result<Vec<IntelligenceDigestItem>, LandscapeGatewayError> {
        let response_payload = self
            .caller
            .call(CapabilityCall {
                capability_id: CAPABILITY_INTELLIGENCE.to_owned(),
                target_repo: TARGET_REPO.to_owned(),
                payload: serde_json::json!({}),
            })
            .await?;

        let envelope: IntelligenceDigestEnvelope =
            serde_json::from_value(response_payload).map_err(LandscapeGatewayError::UnexpectedResponseShape)?;
        Ok(envelope.items)
    }

    async fn submit_field_observation(&self, submission: FieldObservationSubmission) -> Result<(), LandscapeGatewayError> {
        self.caller
            .call(CapabilityCall {
                capability_id: CAPABILITY_OBSERVATIONS.to_owned(),
                target_repo: TARGET_REPO.to_owned(),
                payload: serde_json::to_value(&submission).expect("submission always serializes"),
            })
            .await?;
        Ok(())
    }
}
