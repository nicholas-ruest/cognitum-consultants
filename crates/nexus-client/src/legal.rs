//! Legal ACL gateway (ADR-007, PROMPT-41).
//!
//! Legal owns approved legal/contract policy; this repo never becomes a
//! second store of Legal's own clause library (invariant 3 of the repo's
//! own "Out-of-Scope Reminders") — only the [`ApprovedLegalSnippet`]
//! projection `anti-corruption-layers.md` §9 names, read-only.
//! [`LegalGateway`] is a thin translation boundary over Legal's single
//! outbound call — a read-only clause query, mirroring
//! [`crate::products::ProductsGateway`]'s single-method, no-command shape
//! (a query in DDD terms, no side effect on Legal, and — per this unit's own
//! governing ADR — a "pure read-only, conformist relationship": this repo
//! never negotiates or amends what Legal considers approved).
//!
//! # `context`: a proposal id *or* a topic, never both
//! `anti-corruption-layers.md` §9 names exactly one outbound shape,
//! `RequestApprovedClausesQuery { context: proposal_id | topic }` — an
//! either/or, not two independent optional fields. [`ClauseContext`] models
//! that directly as a two-variant enum rather than
//! `Option<&str>, Option<&str>` (which would let a caller construct an
//! invalid "both set" or "neither set" request the type system should rule
//! out), the same "let the type structurally forbid an invalid shape"
//! reasoning ADR-007's own "Alternatives Considered" section applies to
//! rejecting one generic `NexusClient::call(...)` method in favor of narrow,
//! typed per-capability gateways.
//!
//! # Request path: provisional, matching Products'/Customer's `.../v1/...` convention
//! Nexus's real Legal contract is not finalized. This gateway assumes:
//! - `GET legal/v1/clauses?proposal_id=...` or `GET legal/v1/clauses?topic=...`
//!   — response an envelope `{"clauses": [ApprovedLegalSnippet, ...]}`,
//!   matching [`crate::products::NexusProductsGateway`]'s
//!   `ProductCatalogEnvelope` convention.
//!
//! Update this once Nexus's actual Legal contract is known.
//!
//! # Read-only: no side-effecting command, no two-gateway split
//! Same shape as [`crate::customer::CustomerGateway`]/
//! [`crate::products::ProductsGateway`]: Legal's `anti-corruption-layers.md`
//! §9 entry lists no outbound command with a side effect — only
//! `RequestApprovedClausesQuery`. There is therefore nothing here for the
//! "two `Nexus<Capability>Gateway` instances, one per retry-safety profile"
//! convention (`crate::sales`/`crate::commit` module docs) to split: a
//! single [`NexusLegalGateway`] instance, constructed once over a
//! `RetryingTransport`-wrapped stack, safely serves
//! [`LegalGateway::request_approved_clauses`] — the only method this trait
//! has.
//!
//! # Transport-stack-assembly convention
//! Same convention as [`crate::products::NexusProductsGateway`]:
//! [`NexusLegalGateway::new`] takes an already-fully-decorated
//! `Arc<dyn NexusTransport>` and does not assemble the ADR-016
//! timeout/retry/circuit-breaker stack itself.
//!
//! # Timeout budget choice
//! `request_approved_clauses` is a background/page-load-ish read (a
//! consultant reviewing/editing a Commit proposal, not a single
//! mid-keystroke-blocking call) — it uses
//! [`crate::timeout::DEFAULT_READ_TIMEOUT`], the same budget
//! `CustomerGateway::request_assigned_customer_context`/
//! `ExecutionGateway::request_assigned_engagements`'s query side use, and
//! MAY be retried ([`crate::retry::RetryingTransport`]-wrapped) since it has
//! no side effect.

use std::sync::Arc;

use async_trait::async_trait;

use crate::transport::{CapabilityCall, CapabilityCaller, NexusTransport, NexusTransportError};

/// ADR-029 capability id + target repo for this gateway's single call.
const CAPABILITY_CLAUSES: &str = "legal.clauses";
const TARGET_REPO: &str = "cognitum-legal";

/// Legal's Approved Legal Snippet projection (`anti-corruption-layers.md`
/// §9): this repo never models Legal's own clause library, approval
/// workflow, or policy authoring — only this read-only projection of
/// already-*approved* clause text.
///
/// `Serialize` (alongside `Deserialize`, used to decode Legal's response) is
/// derived so `bff-api` can relay this same shape verbatim to the frontend,
/// matching `ProductReferenceCard`'s/`ProposalSummary`'s "no BFF re-shaping"
/// convention.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ApprovedLegalSnippet {
    pub clause_id: String,
    pub title: String,
    pub approved_text: String,
    pub policy_reference: String,
}

/// Envelope this gateway expects `GET legal/v1/clauses`'s response body to
/// match. See the module docs for why an envelope (vs. a bare array) was
/// chosen (mirrors [`crate::products::NexusProductsGateway`]'s
/// `ProductCatalogEnvelope` rationale).
#[derive(Debug, serde::Deserialize)]
struct ClausesEnvelope {
    clauses: Vec<ApprovedLegalSnippet>,
}

/// Outbound query context: either a proposal id or a topic string, never
/// both — see the module docs for why this is a two-variant enum rather
/// than two independent `Option<&str>` fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClauseContext<'a> {
    ProposalId(&'a str),
    Topic(&'a str),
}

#[derive(Debug, thiserror::Error)]
pub enum LegalGatewayError {
    #[error(transparent)]
    Transport(#[from] NexusTransportError),
    #[error("Legal returned a response body that did not match the expected shape: {0}")]
    UnexpectedResponseShape(#[source] serde_json::Error),
}

/// ACL over Legal's read-only, approved-clause capability. No business
/// policy (e.g. which clauses are "approved") is decided here — see the
/// module docs.
#[async_trait]
pub trait LegalGateway: Send + Sync {
    /// Fetches the currently approved [`ApprovedLegalSnippet`]s for
    /// `context`, per `anti-corruption-layers.md` §9's
    /// `RequestApprovedClausesQuery`.
    ///
    /// A **query** in DDD terms — reading Legal's current approved-clause
    /// state has no side effect, so retrying it is safe/idempotent. See
    /// [`NexusLegalGateway`]'s doc comment for the transport requirement
    /// this method needs from its caller.
    async fn request_approved_clauses(
        &self,
        context: ClauseContext<'_>,
    ) -> Result<Vec<ApprovedLegalSnippet>, LegalGatewayError>;
}

/// [`LegalGateway`] implementation backed by a [`NexusTransport`]. See the
/// module docs for the required transport decoration.
pub struct NexusLegalGateway {
    caller: CapabilityCaller,
}

impl NexusLegalGateway {
    /// `transport` is expected to already be decorated per the ADR-016 read
    /// timeout + optional retry convention (see module docs) — this
    /// constructor does not assemble timeout/retry/circuit-breaker layers
    /// itself. It is wrapped in a [`CapabilityCaller`] so this gateway
    /// issues the ADR-029 capability envelope.
    pub fn new(transport: Arc<dyn NexusTransport>) -> Self {
        Self { caller: CapabilityCaller::new(transport) }
    }
}

#[async_trait]
impl LegalGateway for NexusLegalGateway {
    async fn request_approved_clauses(
        &self,
        context: ClauseContext<'_>,
    ) -> Result<Vec<ApprovedLegalSnippet>, LegalGatewayError> {
        // `context` is a proposal id *or* a topic, never both (see the module
        // docs); the payload carries exactly the one field that is set.
        let payload = match context {
            ClauseContext::ProposalId(proposal_id) => serde_json::json!({ "proposal_id": proposal_id }),
            ClauseContext::Topic(topic) => serde_json::json!({ "topic": topic }),
        };

        let response_payload = self
            .caller
            .call(CapabilityCall {
                capability_id: CAPABILITY_CLAUSES.to_owned(),
                target_repo: TARGET_REPO.to_owned(),
                payload,
            })
            .await?;

        let envelope: ClausesEnvelope =
            serde_json::from_value(response_payload).map_err(LegalGatewayError::UnexpectedResponseShape)?;
        Ok(envelope.clauses)
    }
}
