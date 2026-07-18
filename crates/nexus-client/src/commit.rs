//! Commit ACL gateway (ADR-007, ADR-016, PROMPT-34).
//!
//! Commit owns the proposal-workspace lifecycle; this repo never re-derives
//! or overrides a `ProposalSummary`'s `status`/`stage`
//! (`../../.plans/ddd/anti-corruption-layers.md` §2). [`CommitGateway`] is a
//! thin translation boundary over three outbound calls, mirroring
//! [`crate::sales::SalesGateway`]'s shape exactly (see that module's docs
//! for the pattern this one replicates): one idempotent read
//! ([`CommitGateway::list_proposals`]) and two non-idempotent commands
//! ([`CommitGateway::create_proposal`], [`CommitGateway::request_proposal_action`]).
//!
//! # Request paths: provisional, matching Sales'/Armor's `.../v1/...` convention
//! Nexus's real Commit contract is not finalized. This gateway assumes:
//! - `POST commit/v1/proposals` — body [`CreateProposalCommand`], response a
//!   [`ProposalSummary`].
//! - `GET commit/v1/proposals?consultant_id=...` — response an envelope
//!   `{"proposals": [ProposalSummary, ...]}`, matching
//!   [`crate::armor::NexusArmorGateway`]'s envelope convention (see that
//!   module's doc comment for why an envelope was chosen over a bare
//!   array).
//! - `POST commit/v1/proposal-actions` — body
//!   [`RequestProposalActionCommand`] (both `proposal_id` and `action` in
//!   the body, per `anti-corruption-layers.md` §2's DTO shape — not a
//!   `proposal_id`-templated path, since the doc names the outbound shape
//!   as one flat command, matching `SalesGateway::request_collaboration`'s
//!   flat-command-in-body convention rather than inventing a REST-ful path
//!   template with no worked example to match).
//!
//! Update these once Nexus's actual Commit contract is known.
//!
//! # `list_proposals`: this gateway's own addition beyond the DDD doc
//! `anti-corruption-layers.md` §2 lists `ProposalSummary` as the shape
//! "for listing/dashboard purposes" but does not name an explicit outbound
//! query for it (only `CreateProposalCommand`/`RequestProposalActionCommand`
//! are listed). `bff-api`'s `GET /api/commit/proposals` (PROMPT-34) needs a
//! gateway-level read to back it, so [`CommitGateway::list_proposals`] is
//! added here — a query in DDD terms (no side effect on Commit), following
//! the same "one query-shaped read + N side-effecting commands" shape
//! `SalesGateway` documents in its own module docs as the expected default.
//!
//! # Transport-stack-assembly convention
//! Same convention as [`crate::sales::NexusSalesGateway`]:
//! [`NexusCommitGateway::new`] takes an already-fully-decorated
//! `Arc<dyn NexusTransport>` and does not assemble the ADR-016
//! timeout/retry/circuit-breaker stack itself.
//!
//! # Timeout budget choice
//! `list_proposals` is a background/page-load read (the consultant is not
//! actively blocked on it mid-keystroke the way Sales' `check_account_claim`
//! is) — it uses [`crate::timeout::DEFAULT_READ_TIMEOUT`], the same budget
//! `ArmorGateway::fetch_assertions` uses, and MAY be retried
//! ([`crate::retry::RetryingTransport`]-wrapped) since it has no side
//! effect. `create_proposal` and `request_proposal_action` are both
//! consultant-initiated, synchronous, side-effecting writes the consultant
//! is actively waiting on (e.g. having just clicked "Start Proposal") —
//! they use [`crate::timeout::DEFAULT_WRITE_TIMEOUT`], matching
//! `SalesGateway`'s commands, and must **never** be retried (ADR-016: a
//! retry against an unknown-outcome prior attempt risks creating a
//! duplicate proposal or duplicate action request in Commit).
//!
//! # Two-gateway-instances-for-retry-safety convention
//! Exactly [`crate::sales::NexusSalesGateway`]'s documented constraint:
//! because [`NexusCommitGateway`] holds one shared `transport` field used by
//! every trait method, one instance cannot safely serve both retry profiles
//! at once. `main.rs` therefore constructs **two** `NexusCommitGateway`
//! instances — `commit_query_gateway` (retry-wrapped, `list_proposals`
//! only) and `commit_command_gateway` (no retry, `create_proposal`/
//! `request_proposal_action`) — over the same base transport, mirroring the
//! `sales_query_gateway`/`sales_command_gateway` split exactly.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::transport::{CapabilityCall, CapabilityCaller, NexusTransport, NexusTransportError};

/// ADR-029 capability ids + target repo. `create_proposal` and
/// `list_proposals` shared one path (`commit/v1/proposals`) historically, so
/// both map to `commit.proposals`; the nexus fixture distinguishes them by
/// payload (an `origin_reference`-carrying create body vs. a bare
/// `consultant_id` list query). `request_proposal_action` is its own
/// capability.
const CAPABILITY_PROPOSALS: &str = "commit.proposals";
const CAPABILITY_PROPOSAL_ACTIONS: &str = "commit.proposal_actions";
const TARGET_REPO: &str = "cognitum-commit";

/// Commit's proposal-workspace-handle projection
/// (`anti-corruption-layers.md` §2). This repo never re-implements
/// proposal editing over raw Commit data — full editing stays Commit-hosted
/// UI/flows this repo only frames and deep-links into via `deep_link`.
///
/// `Serialize` (alongside `Deserialize`, used to decode Commit's response)
/// is derived so `bff-api` can relay this same shape verbatim to the
/// frontend, matching `AccountClaimResult`'s "no BFF re-shaping" convention
/// (`crate::sales` module docs).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProposalSummary {
    pub proposal_id: String,
    pub title: String,
    pub status: String,
    pub stage: String,
    pub last_updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deep_link: Option<String>,
}

/// Envelope this gateway expects `GET commit/v1/proposals`'s response body
/// to match. See the module docs for why an envelope (vs. a bare array) was
/// chosen (mirrors [`crate::armor::AssertionsEnvelope`]'s rationale).
#[derive(Debug, serde::Deserialize)]
struct ProposalsEnvelope {
    proposals: Vec<ProposalSummary>,
}

/// Outbound command: creates a new proposal from an origin reference (e.g.
/// a Sales company/lead id). Has a side effect (creates a proposal record
/// in Commit) — never idempotent-safe to blindly retry.
#[derive(Debug, Clone, serde::Serialize)]
struct CreateProposalCommand<'a> {
    origin_reference: &'a str,
    consultant_id: &'a str,
}

/// Outbound command: requests an action (e.g. resend, request revision) on
/// an existing proposal. Has a side effect in Commit — never idempotent-safe
/// to blindly retry.
#[derive(Debug, Clone, serde::Serialize)]
struct RequestProposalActionCommand<'a> {
    proposal_id: &'a str,
    action: &'a str,
}

#[derive(Debug, thiserror::Error)]
pub enum CommitGatewayError {
    #[error(transparent)]
    Transport(#[from] NexusTransportError),
    #[error("Commit returned a response body that did not match the expected shape: {0}")]
    UnexpectedResponseShape(#[source] serde_json::Error),
}

/// ACL over Commit's proposal-workspace capability. No re-adjudication of
/// Commit's own `status`/`stage` happens on this trait — see the module
/// docs.
#[async_trait]
pub trait CommitGateway: Send + Sync {
    /// Creates a new proposal from `origin_reference` (e.g. a Sales
    /// company/lead id). **Not idempotent-safe to retry** — a retry against
    /// an unknown-outcome prior attempt risks creating a duplicate
    /// proposal record in Commit.
    async fn create_proposal(
        &self,
        origin_reference: &str,
        consultant_id: &str,
    ) -> Result<ProposalSummary, CommitGatewayError>;

    /// Lists `consultant_id`'s current proposals. A **query** in DDD
    /// terms — reading Commit's current proposal set has no side effect,
    /// so retrying it is safe/idempotent. See [`NexusCommitGateway`]'s
    /// doc comment for the transport requirement this needs from its
    /// caller.
    async fn list_proposals(
        &self,
        consultant_id: &str,
    ) -> Result<Vec<ProposalSummary>, CommitGatewayError>;

    /// Requests `action` on `proposal_id` (e.g. resend, request revision).
    /// **Not idempotent-safe to retry** — a retry against an
    /// unknown-outcome prior attempt risks issuing a duplicate action
    /// request in Commit.
    async fn request_proposal_action(
        &self,
        proposal_id: &str,
        action: &str,
    ) -> Result<(), CommitGatewayError>;
}

/// [`CommitGateway`] implementation backed by a [`NexusTransport`]. See the
/// module docs for the required transport decoration per method.
pub struct NexusCommitGateway {
    caller: CapabilityCaller,
}

impl NexusCommitGateway {
    /// See the module docs for the required transport decoration
    /// (read timeout + optional retry for `list_proposals`; write timeout,
    /// never retried, for `create_proposal`/`request_proposal_action`). The
    /// `transport` is wrapped in a [`CapabilityCaller`] so each method
    /// issues the ADR-029 capability envelope.
    pub fn new(transport: Arc<dyn NexusTransport>) -> Self {
        Self { caller: CapabilityCaller::new(transport) }
    }

    async fn call(
        &self,
        capability_id: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, CommitGatewayError> {
        Ok(self
            .caller
            .call(CapabilityCall { capability_id: capability_id.to_owned(), target_repo: TARGET_REPO.to_owned(), payload })
            .await?)
    }
}

#[async_trait]
impl CommitGateway for NexusCommitGateway {
    async fn create_proposal(
        &self,
        origin_reference: &str,
        consultant_id: &str,
    ) -> Result<ProposalSummary, CommitGatewayError> {
        let command = CreateProposalCommand { origin_reference, consultant_id };
        let payload =
            self.call(CAPABILITY_PROPOSALS, serde_json::to_value(command).expect("command always serializes")).await?;
        serde_json::from_value(payload).map_err(CommitGatewayError::UnexpectedResponseShape)
    }

    async fn list_proposals(&self, consultant_id: &str) -> Result<Vec<ProposalSummary>, CommitGatewayError> {
        let payload = self.call(CAPABILITY_PROPOSALS, serde_json::json!({ "consultant_id": consultant_id })).await?;

        let envelope: ProposalsEnvelope =
            serde_json::from_value(payload).map_err(CommitGatewayError::UnexpectedResponseShape)?;
        Ok(envelope.proposals)
    }

    async fn request_proposal_action(&self, proposal_id: &str, action: &str) -> Result<(), CommitGatewayError> {
        let command = RequestProposalActionCommand { proposal_id, action };
        self.call(CAPABILITY_PROPOSAL_ACTIONS, serde_json::to_value(command).expect("command always serializes")).await?;
        Ok(())
    }
}
