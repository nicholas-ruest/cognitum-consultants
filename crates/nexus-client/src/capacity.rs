//! Capacity ACL gateway (ADR-007, ADR-016, PROMPT-36).
//!
//! Capacity is this repo's narrowest, most gated relationship of the ten
//! (`../../.plans/ddd/domain-map.md`: "Consultants must not receive internal
//! Capacity access"). `../../.plans/ddd/anti-corruption-layers.md` §4 names
//! it explicitly **write-heavy and read-narrow**: this ACL may submit the
//! consultant's own profile updates, and may read back only what that same
//! consultant is permitted to see about themself — never anyone else's.
//!
//! # Structural, not filtering: no cross-consultant query shape exists
//! Every method on [`CapacityGateway`] takes exactly one identifying
//! parameter, `consultant_id: &str` — the caller's own id, always threaded
//! in by `bff-api` from the authenticated session, never taken from request
//! input a consultant could tamper with (see `bff-api`'s `capacity` module
//! for that wiring). Unlike the Customer ACL's (PROMPT-37, not yet built)
//! documented `RequestAssignedCustomerContextQuery { consultant_id,
//! customer_id? }` shape (`anti-corruption-layers.md` §5 — which accepts an
//! optional second `customer_id` to further scope a query), this trait has
//! **no method, filter, or optional parameter that could name a second
//! consultant at all** — there is no
//! `list_profiles`/`get_profile(other_consultant_id)` shape to even attempt
//! calling. This is ADR-007's own worked rationale for rejecting "one shared
//! generic `NexusClient::call(capability, command, payload)`" in favor of
//! narrow per-capability traits: "a generic client can't structurally
//! forbid a call shape the way a narrow trait can." A code reviewer can
//! confirm the "no cross-consultant query path" acceptance criterion by
//! reading this trait's method signatures alone — no runtime filtering
//! logic to audit.
//!
//! # Request paths: provisional, matching Commit's `.../v1/...` convention
//! Nexus's real Capacity contract is not finalized. This gateway assumes:
//! - `GET capacity/v1/profile?consultant_id=...` — response a
//!   [`ConsultantProfileIntake`] (a single object, not an enveloped list —
//!   there is exactly one profile per consultant, unlike
//!   [`crate::commit::NexusCommitGateway`]'s `ProposalsEnvelope`).
//! - `POST capacity/v1/profile` — body [`UpdateOwnProfileCommand`],
//!   response a [`ProfileUpdateResult`]. Both verbs share one path, the
//!   same `GET`-lists/`POST`-creates convention
//!   [`crate::commit::NexusCommitGateway`]'s `commit/v1/proposals` already
//!   establishes.
//!
//! Update these once Nexus's actual Capacity contract is known.
//!
//! # `ProfileUpdateResult`: the inbound verdict, returned synchronously
//! `anti-corruption-layers.md` §4 names the inbound events
//! `ProfileUpdateAccepted`/`ProfileUpdateRejected { reason }`. Following
//! [`crate::sales::AccountClaimResult`]'s worked convention (a synchronous,
//! user-blocking command whose own response body carries the verdict, not a
//! later async event this repo would need to poll for), `update_own_profile`
//! decodes Capacity's direct response into [`ProfileUpdateResult`] —
//! PROMPT-36's own acceptance criterion ("Return the response from Capacity
//! (accepted/rejected + reason)") describes exactly this synchronous shape,
//! not an async notification. This repo never re-derives `accepted` or
//! `reason` — both are relayed verbatim, same "never re-adjudicate the
//! verdict" rule `AccountClaimResult`'s module docs establish for
//! `creation_allowed`.
//!
//! # Transport-stack-assembly convention
//! Same convention as [`crate::sales::NexusSalesGateway`]/
//! [`crate::commit::NexusCommitGateway`]: [`NexusCapacityGateway::new`]
//! takes an already-fully-decorated `Arc<dyn NexusTransport>` and does not
//! assemble the ADR-016 timeout/retry/circuit-breaker stack itself.
//!
//! # Timeout budget choice
//! `get_own_profile` is a background/page-load read (the consultant is not
//! actively blocked on it mid-keystroke) — it uses
//! [`crate::timeout::DEFAULT_READ_TIMEOUT`], the same budget
//! `CommitGateway::list_proposals` uses, and MAY be retried
//! ([`crate::retry::RetryingTransport`]-wrapped) since it has no side
//! effect. `update_own_profile` is a consultant-initiated, synchronous,
//! side-effecting write the consultant is actively waiting on (having just
//! clicked "Save Profile") — it uses
//! [`crate::timeout::DEFAULT_WRITE_TIMEOUT`], matching `CommitGateway::
//! create_proposal`, and must **never** be retried (ADR-016: a retry
//! against an unknown-outcome prior attempt risks submitting a duplicate,
//! possibly conflicting profile update to Capacity).
//!
//! # Two-gateway-instances-for-retry-safety convention
//! Exactly [`crate::commit::NexusCommitGateway`]'s documented constraint:
//! because [`NexusCapacityGateway`] holds one shared `transport` field used
//! by every trait method, one instance cannot safely serve both retry
//! profiles at once. `main.rs` therefore constructs **two**
//! `NexusCapacityGateway` instances — `capacity_query_gateway`
//! (retry-wrapped, `get_own_profile` only) and `capacity_command_gateway`
//! (no retry, `update_own_profile` only) — over the same base transport,
//! mirroring the `sales_query_gateway`/`sales_command_gateway` and
//! `commit_query_gateway`/`commit_command_gateway` splits exactly.

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Method;
use reqwest::header::HeaderMap;

use crate::transport::{NexusRequest, NexusTransport, NexusTransportError};

/// Capacity's restricted Consultant Profile Intake shape
/// (`anti-corruption-layers.md` §4): "this repo's ACL is intentionally
/// write-heavy and read-narrow" — this is the *only* shape of Capacity data
/// this repo ever sees, whether submitting an update (as
/// [`UpdateOwnProfileCommand::profile_fields`]) or reading the consultant's
/// own current profile back ([`CapacityGateway::get_own_profile`]'s return
/// value). This repo never models Capacity's internal capacity-planning
/// data or any other consultant's profile.
///
/// `Serialize` (alongside `Deserialize`, used to decode Capacity's `GET`
/// response) is derived so `bff-api` can relay this same shape verbatim to
/// the frontend, matching `ProposalSummary`'s/`LearningSnapshot`'s "no BFF
/// re-shaping" convention.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ConsultantProfileIntake {
    pub skills: Vec<String>,
    pub certifications: Vec<String>,
    pub languages: Vec<String>,
    pub availability_window: String,
    pub geographic_coverage: Vec<String>,
}

/// Capacity's verdict on a submitted [`UpdateOwnProfileCommand`], per the
/// module docs' "returned synchronously" section. `Serialize` (alongside
/// `Deserialize`) is derived for the same "relay verbatim to the frontend"
/// reason [`ConsultantProfileIntake`] derives it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProfileUpdateResult {
    pub accepted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Outbound command: submits the consultant's own restricted profile
/// update. Has a side effect in Capacity — never idempotent-safe to blindly
/// retry.
#[derive(Debug, Clone, serde::Serialize)]
struct UpdateOwnProfileCommand<'a> {
    consultant_id: &'a str,
    profile_fields: &'a ConsultantProfileIntake,
}

#[derive(Debug, thiserror::Error)]
pub enum CapacityGatewayError {
    #[error(transparent)]
    Transport(#[from] NexusTransportError),
    #[error("Capacity returned a non-success status {status}")]
    UnexpectedStatus { status: reqwest::StatusCode },
    #[error("Capacity returned a response body that did not match the expected shape: {0}")]
    UnexpectedResponseShape(#[source] serde_json::Error),
}

/// ACL over Capacity's deliberately restricted intake capability. See the
/// module docs for why this trait's shape *structurally* forbids querying
/// another consultant's data, rather than relying on runtime filtering.
#[async_trait]
pub trait CapacityGateway: Send + Sync {
    /// Submits `profile_fields` as `consultant_id`'s own profile update, per
    /// `anti-corruption-layers.md` §4's `UpdateOwnProfileCommand`. **Not
    /// idempotent-safe to retry** — a retry against an unknown-outcome
    /// prior attempt risks submitting a duplicate, possibly conflicting
    /// update to Capacity.
    async fn update_own_profile(
        &self,
        consultant_id: &str,
        profile_fields: ConsultantProfileIntake,
    ) -> Result<ProfileUpdateResult, CapacityGatewayError>;

    /// Fetches `consultant_id`'s own current profile — the read-narrow half
    /// of this ACL. A **query** in DDD terms — reading Capacity's current
    /// profile state has no side effect, so retrying it is safe/idempotent.
    /// See the module docs for why this signature cannot express a
    /// cross-consultant lookup.
    async fn get_own_profile(&self, consultant_id: &str) -> Result<ConsultantProfileIntake, CapacityGatewayError>;
}

/// [`CapacityGateway`] implementation backed by a [`NexusTransport`]. See
/// the module docs for the required transport decoration per method.
pub struct NexusCapacityGateway {
    transport: Arc<dyn NexusTransport>,
}

impl NexusCapacityGateway {
    /// See the module docs for the required transport decoration (read
    /// timeout + optional retry for `get_own_profile`; write timeout, never
    /// retried, for `update_own_profile`).
    pub fn new(transport: Arc<dyn NexusTransport>) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl CapacityGateway for NexusCapacityGateway {
    async fn update_own_profile(
        &self,
        consultant_id: &str,
        profile_fields: ConsultantProfileIntake,
    ) -> Result<ProfileUpdateResult, CapacityGatewayError> {
        let command = UpdateOwnProfileCommand { consultant_id, profile_fields: &profile_fields };
        let request = NexusRequest {
            method: Method::POST,
            path: "capacity/v1/profile".to_string(),
            headers: HeaderMap::new(),
            body: Some(serde_json::to_value(&command).expect("command always serializes")),
        };
        let response = self.transport.send(request).await?;

        if !response.status.is_success() {
            return Err(CapacityGatewayError::UnexpectedStatus { status: response.status });
        }

        serde_json::from_value(response.body).map_err(CapacityGatewayError::UnexpectedResponseShape)
    }

    async fn get_own_profile(&self, consultant_id: &str) -> Result<ConsultantProfileIntake, CapacityGatewayError> {
        let path = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.append_pair("consultant_id", consultant_id);
            format!("capacity/v1/profile?{}", query.finish())
        };

        let request = NexusRequest { method: Method::GET, path, headers: HeaderMap::new(), body: None };
        let response = self.transport.send(request).await?;

        if !response.status.is_success() {
            return Err(CapacityGatewayError::UnexpectedStatus { status: response.status });
        }

        serde_json::from_value(response.body).map_err(CapacityGatewayError::UnexpectedResponseShape)
    }
}
