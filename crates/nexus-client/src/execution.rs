//! Execution ACL gateway (ADR-007, ADR-016, PROMPT-38).
//!
//! Execution owns the consultant's assigned delivery workspace (engagements,
//! workstreams, milestones, tasks); this repo never re-derives or overrides
//! an [`EngagementSnapshot`]'s `delivery_status`, and never stores a full
//! copy of Execution's engagement/task graph — only this read projection
//! (`../../.plans/ddd/anti-corruption-layers.md` §6). [`ExecutionGateway`] is
//! a thin translation boundary over two outbound calls, mirroring
//! [`crate::commit::CommitGateway`]'s shape: one idempotent read
//! ([`ExecutionGateway::request_assigned_engagements`]) and one non-
//! idempotent command ([`ExecutionGateway::confirm_task_completion`]).
//!
//! # Request paths: provisional, matching Commit's `.../v1/...` convention
//! Nexus's real Execution contract is not finalized. This gateway assumes:
//! - `GET execution/v1/engagements?consultant_id=...` — response an envelope
//!   `{"engagements": [EngagementSnapshot, ...]}`, matching
//!   [`crate::commit::NexusCommitGateway`]'s `ProposalsEnvelope` convention.
//! - `POST execution/v1/task-completions` — body
//!   [`ConfirmTaskCompletionCommand`], fire-and-confirm (no documented ack
//!   body — see the "beyond the DDD doc" section below).
//!
//! Update these once Nexus's actual Execution contract is known.
//!
//! # `confirm_task_completion`: this gateway's own addition beyond the DDD doc
//! `anti-corruption-layers.md` §6 names exactly one outbound call
//! (`RequestAssignedEngagementsQuery`) and three inbound events
//! (`MilestoneCompleted`, `DeliveryRiskRaised`, `TaskAssigned`) — it does not
//! name an outbound command for requesting task completion. PROMPT-38's own
//! prompt text is explicit, though: "Action queue items for assigned tasks
//! should route completion through the BFF back to Execution" — a consultant
//! marking a task done must have *something* to call through Nexus, the same
//! "the DDD doc doesn't name an explicit outbound query/command for a route
//! this unit needs, so this gateway adds one" reasoning
//! [`crate::commit::NexusCommitGateway`]'s module docs use for
//! `list_proposals`. [`ExecutionGateway::confirm_task_completion`] is that
//! addition. **Critical**: this call only *requests* completion — it is not,
//! and must never become, the mechanism that flips a
//! `bff_core::ActionQueueEntry` to `Completed` locally. Per
//! `consultant-experience-context.md` §2.2 invariant 3, only a confirmation
//! event routed back through Nexus's event-ingestion pipeline
//! (`bff_core::event_ingestion`) may do that — see `crate::bff_api::execution`
//! (this repo's BFF handler for this call) for the enforcement of that
//! boundary.
//!
//! # Transport-stack-assembly convention
//! Same convention as [`crate::commit::NexusCommitGateway`]:
//! [`NexusExecutionGateway::new`] takes an already-fully-decorated
//! `Arc<dyn NexusTransport>` and does not assemble the ADR-016
//! timeout/retry/circuit-breaker stack itself.
//!
//! # Timeout budget choice
//! `request_assigned_engagements` is a background/page-load read (the
//! consultant is not actively blocked on it mid-keystroke) — it uses
//! [`crate::timeout::DEFAULT_READ_TIMEOUT`], the same budget
//! `CommitGateway::list_proposals`/`CustomerGateway::request_assigned_customer_context`
//! use, and MAY be retried ([`crate::retry::RetryingTransport`]-wrapped)
//! since it has no side effect. `confirm_task_completion` is a consultant-
//! initiated, side-effecting write — it uses
//! [`crate::timeout::DEFAULT_WRITE_TIMEOUT`] and must **never** be retried
//! (ADR-016: a retry against an unknown-outcome prior attempt risks
//! double-requesting completion in Execution).
//!
//! # Two-gateway-instances-for-retry-safety convention
//! Exactly [`crate::commit::NexusCommitGateway`]'s documented constraint:
//! because [`NexusExecutionGateway`] holds one shared `transport` field used
//! by every trait method, one instance cannot safely serve both retry
//! profiles at once. `main.rs` therefore constructs **two**
//! `NexusExecutionGateway` instances — `execution_query_gateway`
//! (retry-wrapped, `request_assigned_engagements` only) and
//! `execution_command_gateway` (no retry, `confirm_task_completion` only) —
//! over the same base transport, mirroring the `commit_query_gateway`/
//! `commit_command_gateway` split exactly.

use std::sync::Arc;

use async_trait::async_trait;

use crate::transport::{CapabilityCall, CapabilityCaller, NexusTransport, NexusTransportError};

/// ADR-029 capability id + target repo. The ADR-029 capability table names
/// only `execution.task_completions` for this repo, but this gateway has two
/// methods (the engagements read + the completion write) that shared no
/// single path historically. Both are routed through this one capability id;
/// the nexus fixture distinguishes them by payload (a bare `consultant_id`
/// for the read vs. a `task_id`-carrying body for the write). This is the
/// one place the ADR's table under-specifies (it omits an engagements-read
/// id); routing both here keeps to the table's declared 14 capabilities.
const CAPABILITY_TASK_COMPLETIONS: &str = "execution.task_completions";
const TARGET_REPO: &str = "cognitum-execution";

/// One assigned task within an [`EngagementSnapshot`]. Structured (not a
/// bare `String`, unlike `workstreams`/`milestones` below) because
/// [`ExecutionGateway::confirm_task_completion`] needs a stable `task_id`
/// handle to reference — the same `task_id` `TaskAssigned`
/// (`anti-corruption-layers.md` §6) carries on its own inbound payload
/// (`../ddd/domain-events.md` §3 Execution).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EngagementTaskSummary {
    pub task_id: String,
    pub title: String,
    pub status: String,
}

/// Execution's Engagement Workspace Snapshot projection
/// (`anti-corruption-layers.md` §6): this repo never models Execution's full
/// engagement/workstream/task graph — only this read projection, plus a
/// `deep_link` back into Execution's own UI for anything beyond it.
///
/// `Serialize` (alongside `Deserialize`, used to decode Execution's
/// response) is derived so `bff-api` can relay this same shape verbatim to
/// the frontend, matching `ProposalSummary`'s/`CustomerContextCard`'s "no BFF
/// re-shaping" convention.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EngagementSnapshot {
    pub engagement_id: String,
    pub workstreams: Vec<String>,
    pub milestones: Vec<String>,
    pub tasks: Vec<EngagementTaskSummary>,
    pub delivery_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deep_link: Option<String>,
}

/// Envelope this gateway expects `GET execution/v1/engagements`'s response
/// body to match. See the module docs for why an envelope (vs. a bare array)
/// was chosen (mirrors [`crate::commit::NexusCommitGateway`]'s
/// `ProposalsEnvelope` rationale).
#[derive(Debug, serde::Deserialize)]
struct EngagementsEnvelope {
    engagements: Vec<EngagementSnapshot>,
}

/// Outbound command: requests that Execution record `task_id` as complete.
/// Has a side effect in Execution (initiates its own completion workflow) —
/// never idempotent-safe to blindly retry. See the module docs'
/// "`confirm_task_completion`: this gateway's own addition" section.
#[derive(Debug, Clone, serde::Serialize)]
struct ConfirmTaskCompletionCommand<'a> {
    task_id: &'a str,
    consultant_id: &'a str,
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutionGatewayError {
    #[error(transparent)]
    Transport(#[from] NexusTransportError),
    #[error("Execution returned a response body that did not match the expected shape: {0}")]
    UnexpectedResponseShape(#[source] serde_json::Error),
}

/// ACL over Execution's delivery-workspace and task-completion-request
/// capabilities. No re-adjudication of Execution's own `delivery_status`/
/// task `status` happens on this trait — see the module docs.
#[async_trait]
pub trait ExecutionGateway: Send + Sync {
    /// Fetches `consultant_id`'s assigned [`EngagementSnapshot`] set, per
    /// `anti-corruption-layers.md` §6's `RequestAssignedEngagementsQuery`.
    ///
    /// A **query** in DDD terms — reading Execution's current workspace
    /// state has no side effect, so retrying it is safe/idempotent. See
    /// [`NexusExecutionGateway`]'s doc comment for the transport requirement
    /// this method needs from its caller.
    async fn request_assigned_engagements(
        &self,
        consultant_id: &str,
    ) -> Result<Vec<EngagementSnapshot>, ExecutionGatewayError>;

    /// Requests that Execution record `task_id` as complete, on behalf of
    /// `consultant_id`. **Not idempotent-safe to retry** — a retry against
    /// an unknown-outcome prior attempt risks double-requesting completion
    /// in Execution. **Does not itself mark anything complete in this
    /// repo** — see the module docs' "critical" note.
    async fn confirm_task_completion(
        &self,
        task_id: &str,
        consultant_id: &str,
    ) -> Result<(), ExecutionGatewayError>;
}

/// [`ExecutionGateway`] implementation backed by a [`NexusTransport`]. See
/// the module docs for the required transport decoration per method.
pub struct NexusExecutionGateway {
    caller: CapabilityCaller,
}

impl NexusExecutionGateway {
    /// See the module docs for the required transport decoration (read
    /// timeout + optional retry for `request_assigned_engagements`; write
    /// timeout, never retried, for `confirm_task_completion`). The
    /// `transport` is wrapped in a [`CapabilityCaller`] so each method
    /// issues the ADR-029 capability envelope.
    pub fn new(transport: Arc<dyn NexusTransport>) -> Self {
        Self { caller: CapabilityCaller::new(transport) }
    }
}

#[async_trait]
impl ExecutionGateway for NexusExecutionGateway {
    async fn request_assigned_engagements(
        &self,
        consultant_id: &str,
    ) -> Result<Vec<EngagementSnapshot>, ExecutionGatewayError> {
        let response_payload = self
            .caller
            .call(CapabilityCall {
                capability_id: CAPABILITY_TASK_COMPLETIONS.to_owned(),
                target_repo: TARGET_REPO.to_owned(),
                payload: serde_json::json!({ "consultant_id": consultant_id }),
            })
            .await?;

        let envelope: EngagementsEnvelope =
            serde_json::from_value(response_payload).map_err(ExecutionGatewayError::UnexpectedResponseShape)?;
        Ok(envelope.engagements)
    }

    async fn confirm_task_completion(&self, task_id: &str, consultant_id: &str) -> Result<(), ExecutionGatewayError> {
        let command = ConfirmTaskCompletionCommand { task_id, consultant_id };
        self.caller
            .call(CapabilityCall {
                capability_id: CAPABILITY_TASK_COMPLETIONS.to_owned(),
                target_repo: TARGET_REPO.to_owned(),
                payload: serde_json::to_value(command).expect("command always serializes"),
            })
            .await?;
        Ok(())
    }
}
