//! `CrossCapabilityWorkflowSession` aggregate (`consultant-experience-context.md`
//! §1.2) and its repository port (`WorkflowSessionRepository`, implemented
//! against Postgres in `persistence`, ADR-010).
//!
//! Tracks an in-progress hop between capabilities (e.g. from Sales to
//! Commit) as transient, correlation-tracked state. Per the DDD doc's
//! repo-wide invariant ("owns zero business records"), this aggregate never
//! holds the business entity itself — only opaque references to it.
//!
//! Invariants enforced here:
//! 1. **Opaque references only.** [`CrossCapabilityWorkflowSession::origin_reference`]
//!    and [`CrossCapabilityWorkflowSession::target_reference`] are plain
//!    `String` ids, never a richer business payload. There is no field on
//!    this type that could hold one, and no constructor accepts one — this
//!    is a structural property, not a runtime check.
//! 2. **Bounded TTL.** [`CrossCapabilityWorkflowSession::is_expired`] checks
//!    `now` against `expires_at`, and every state-changing method
//!    ([`CrossCapabilityWorkflowSession::transition_to`],
//!    [`CrossCapabilityWorkflowSession::set_target_reference`]) refuses to
//!    proceed (returning [`WorkflowSessionError::SessionExpired`], never
//!    silently succeeding) once a session is expired **by time**, regardless
//!    of what `status` currently says — a session can be time-expired for a
//!    while before any housekeeping sweep ([`WorkflowSessionRepository::expire_older_than`])
//!    gets around to flipping its `status` to [`WorkflowSessionStatus::Expired`],
//!    and this invariant must hold from the moment `now >= expires_at`, not
//!    from the moment the row is swept. **Assumption** (flagged in the DDD
//!    doc as unspecified by research.md): the default TTL is
//!    [`DEFAULT_WORKFLOW_SESSION_TTL_MINUTES`] = 30 minutes — chosen as a
//!    generous-but-bounded window for a consultant to complete a single
//!    cross-capability hand-off (e.g. "start a proposal from this lead")
//!    without the session lingering indefinitely, matching the DDD doc's
//!    concern that an unbounded TTL would make this context "silently
//!    become a long-term store of cross-capability state". Revisit once
//!    real usage data (Phase 4, U34+) shows hand-offs routinely take longer
//!    or shorter than this.
//! 3. **Linear state machine, no regression.** See
//!    [`WorkflowSessionStatus::is_valid_transition`] for the exact matrix
//!    and the `Started -> {Abandoned, Expired}` design decision (documented
//!    on that function). No transition is ever valid *out of* a terminal
//!    state ([`WorkflowSessionStatus::Completed`],
//!    [`WorkflowSessionStatus::Abandoned`], [`WorkflowSessionStatus::Expired`]).
//! 4. **Completion never mutates the target capability's data.** This
//!    aggregate has no method, field, or dependency that could perform such
//!    a mutation — [`CrossCapabilityWorkflowSession::transition_to`] with
//!    [`WorkflowSessionStatus::Completed`] only flips this aggregate's own
//!    `status`. The actual mutation (e.g. a proposal being created) is owned
//!    and confirmed by the target capability via Nexus, entirely outside
//!    this type — there is nothing here to enforce beyond "this type cannot
//!    do it", which is a structural (not runtime-checked) property, same as
//!    invariant 1.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

/// Default time-to-live for a freshly [`CrossCapabilityWorkflowSession::start`]ed
/// session. See invariant 2's doc comment (module docs) for the rationale.
pub const DEFAULT_WORKFLOW_SESSION_TTL_MINUTES: i64 = 30;

/// A [`CrossCapabilityWorkflowSession`]'s linear state machine status
/// (`consultant-experience-context.md` §1.2 invariant 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowSessionStatus {
    /// The hand-off has been initiated but the consultant has not yet
    /// engaged with the target capability.
    Started,
    /// The consultant is actively mid-hand-off in the target capability.
    InProgress,
    /// Terminal: the hand-off completed successfully.
    Completed,
    /// Terminal: the consultant abandoned the hand-off before completing it.
    Abandoned,
    /// Terminal: the session's TTL elapsed before it completed.
    Expired,
}

impl WorkflowSessionStatus {
    /// The wire/storage string for this status (DB `status` column).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Abandoned => "abandoned",
            Self::Expired => "expired",
        }
    }

    /// Terminal states admit no further transition (invariant 3) — see
    /// [`Self::is_valid_transition`], which this helper is consistent with
    /// by construction (every arm returning `true` there has a non-terminal
    /// `from`).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Abandoned | Self::Expired)
    }

    /// Whether `from -> to` is a valid transition in the linear state
    /// machine `started -> in_progress -> {completed|abandoned|expired}`.
    ///
    /// **Design decision** (the DDD doc explicitly leaves this to
    /// implementation judgment): `Started -> Abandoned` and
    /// `Started -> Expired` are *also* valid, direct transitions, bypassing
    /// `InProgress`. A consultant can abandon a hand-off (e.g. close the tab)
    /// before ever engaging with the target capability, and the TTL
    /// housekeeping sweep must be able to expire a session that never left
    /// `Started` just as much as one that reached `InProgress` — there is no
    /// domain reason `InProgress` should be a mandatory waypoint for those
    /// two terminal outcomes specifically.
    ///
    /// `Started -> Completed` is deliberately **not** allowed, unlike the
    /// two transitions above: "completed" means the hand-off was actually
    /// carried out in the target capability (invariant 4), which requires
    /// having been `InProgress` first — jumping straight from `Started` to
    /// `Completed` would let a session record success without ever having
    /// recorded that the consultant engaged with the target capability at
    /// all, which is precisely the correlation data this aggregate exists
    /// to hold.
    ///
    /// Every other pair — including any transition where `from` is already
    /// terminal, and same-state no-ops like `Started -> Started` — is
    /// invalid.
    pub fn is_valid_transition(from: Self, to: Self) -> bool {
        matches!(
            (from, to),
            (Self::Started, Self::InProgress)
                | (Self::Started, Self::Abandoned)
                | (Self::Started, Self::Expired)
                | (Self::InProgress, Self::Completed)
                | (Self::InProgress, Self::Abandoned)
                | (Self::InProgress, Self::Expired)
        )
    }
}

impl fmt::Display for WorkflowSessionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A workflow-session status string that isn't a known
/// [`WorkflowSessionStatus`] variant.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown workflow session status: {0:?}")]
pub struct ParseWorkflowSessionStatusError(String);

impl FromStr for WorkflowSessionStatus {
    type Err = ParseWorkflowSessionStatusError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "started" => Ok(Self::Started),
            "in_progress" => Ok(Self::InProgress),
            "completed" => Ok(Self::Completed),
            "abandoned" => Ok(Self::Abandoned),
            "expired" => Ok(Self::Expired),
            other => Err(ParseWorkflowSessionStatusError(other.to_string())),
        }
    }
}

/// Transient, correlation-tracked state describing an in-progress hop
/// between capabilities (`consultant-experience-context.md` §1.2). Root of
/// its own aggregate — no child entities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossCapabilityWorkflowSession {
    session_id: Uuid,
    consultant_id: String,
    origin_capability: String,
    origin_reference: String,
    target_capability: String,
    target_reference: Option<String>,
    status: WorkflowSessionStatus,
    expires_at: DateTime<Utc>,
}

/// Errors constructing/mutating a [`CrossCapabilityWorkflowSession`]
/// aggregate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkflowSessionError {
    /// `consultant_id` was empty/blank.
    #[error("consultant_id must not be empty")]
    EmptyConsultantId,
    /// `origin_capability` was empty/blank.
    #[error("origin_capability must not be empty")]
    EmptyOriginCapability,
    /// `origin_reference` was empty/blank.
    #[error("origin_reference must not be empty")]
    EmptyOriginReference,
    /// `target_capability` was empty/blank.
    #[error("target_capability must not be empty")]
    EmptyTargetCapability,
    /// A `Some("")`/blank `target_reference` was supplied — `None` is the
    /// correct way to say "not yet resolved", not an empty string.
    #[error("target_reference must not be empty when present")]
    EmptyTargetReference,
    /// Invariant 3: `from -> to` is not a valid state-machine transition
    /// (including any attempted transition out of a terminal state).
    #[error("invalid transition from {from} to {to}")]
    InvalidTransition { from: WorkflowSessionStatus, to: WorkflowSessionStatus },
    /// Invariant 2: the session is past its `expires_at` and so cannot be
    /// resumed or transitioned, regardless of its current `status` — it
    /// must be re-initiated.
    #[error("session expired at {expires_at} and cannot be resumed or transitioned")]
    SessionExpired { expires_at: DateTime<Utc> },
}

impl CrossCapabilityWorkflowSession {
    /// Starts a brand-new session: fresh `session_id`, [`WorkflowSessionStatus::Started`],
    /// `target_reference` unresolved (`None`), and `expires_at` set
    /// [`DEFAULT_WORKFLOW_SESSION_TTL_MINUTES`] after `now`.
    pub fn start(
        consultant_id: impl Into<String>,
        origin_capability: impl Into<String>,
        origin_reference: impl Into<String>,
        target_capability: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<Self, WorkflowSessionError> {
        Self::from_parts(
            Uuid::new_v4(),
            consultant_id.into(),
            origin_capability.into(),
            origin_reference.into(),
            target_capability.into(),
            None,
            WorkflowSessionStatus::Started,
            now + Duration::minutes(DEFAULT_WORKFLOW_SESSION_TTL_MINUTES),
        )
    }

    /// Reconstructs an aggregate from already-known parts (e.g. a
    /// repository loading a persisted row). Re-validates every field the
    /// same as [`Self::start`] would.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        session_id: Uuid,
        consultant_id: String,
        origin_capability: String,
        origin_reference: String,
        target_capability: String,
        target_reference: Option<String>,
        status: WorkflowSessionStatus,
        expires_at: DateTime<Utc>,
    ) -> Result<Self, WorkflowSessionError> {
        if consultant_id.trim().is_empty() {
            return Err(WorkflowSessionError::EmptyConsultantId);
        }
        if origin_capability.trim().is_empty() {
            return Err(WorkflowSessionError::EmptyOriginCapability);
        }
        if origin_reference.trim().is_empty() {
            return Err(WorkflowSessionError::EmptyOriginReference);
        }
        if target_capability.trim().is_empty() {
            return Err(WorkflowSessionError::EmptyTargetCapability);
        }
        if let Some(reference) = &target_reference
            && reference.trim().is_empty()
        {
            return Err(WorkflowSessionError::EmptyTargetReference);
        }

        Ok(Self {
            session_id,
            consultant_id,
            origin_capability,
            origin_reference,
            target_capability,
            target_reference,
            status,
            expires_at,
        })
    }

    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    pub fn consultant_id(&self) -> &str {
        &self.consultant_id
    }

    pub fn origin_capability(&self) -> &str {
        &self.origin_capability
    }

    pub fn origin_reference(&self) -> &str {
        &self.origin_reference
    }

    pub fn target_capability(&self) -> &str {
        &self.target_capability
    }

    pub fn target_reference(&self) -> Option<&str> {
        self.target_reference.as_deref()
    }

    pub fn status(&self) -> WorkflowSessionStatus {
        self.status
    }

    pub fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    /// Invariant 2's TTL check: whether this session is expired as of
    /// `now`. Boundary is inclusive — `now == expires_at` counts as
    /// expired, so a session's validity window is `now < expires_at`.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.expires_at
    }

    /// Resolves the target capability's opaque reference once the hand-off
    /// target is known. Guarded by the same TTL check as
    /// [`Self::transition_to`] (invariant 2 applies to every state-changing
    /// method, not just status transitions).
    pub fn set_target_reference(
        &mut self,
        target_reference: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<(), WorkflowSessionError> {
        if self.is_expired(now) {
            return Err(WorkflowSessionError::SessionExpired { expires_at: self.expires_at });
        }

        let target_reference = target_reference.into();
        if target_reference.trim().is_empty() {
            return Err(WorkflowSessionError::EmptyTargetReference);
        }

        self.target_reference = Some(target_reference);
        Ok(())
    }

    /// Attempts to move this session to `next`. Enforces both halves of
    /// invariant 2/3:
    /// 1. TTL (invariant 2): if `now` is past `expires_at`, the transition
    ///    is refused with [`WorkflowSessionError::SessionExpired`] — this is
    ///    checked *before* the state-machine check, so a time-expired
    ///    session cannot be transitioned even if, considered purely by
    ///    `status`, the move would otherwise be legal (e.g. `InProgress ->
    ///    Completed` on a session whose TTL has silently elapsed but whose
    ///    `status` hasn't been swept to `Expired` yet).
    /// 2. State machine (invariant 3): delegates to
    ///    [`WorkflowSessionStatus::is_valid_transition`].
    pub fn transition_to(
        &mut self,
        next: WorkflowSessionStatus,
        now: DateTime<Utc>,
    ) -> Result<(), WorkflowSessionError> {
        if self.is_expired(now) {
            return Err(WorkflowSessionError::SessionExpired { expires_at: self.expires_at });
        }
        if !WorkflowSessionStatus::is_valid_transition(self.status, next) {
            return Err(WorkflowSessionError::InvalidTransition { from: self.status, to: next });
        }

        self.status = next;
        Ok(())
    }
}

/// Repository port for [`CrossCapabilityWorkflowSession`]
/// (`consultant-experience-context.md` §1.4). Implemented against Postgres
/// in `persistence` (ADR-010); `bff-core` only defines the interface, per
/// ADR-004's trait-interface-only dependency direction.
///
/// `Send + Sync` so implementations can be shared behind an
/// `Arc<dyn WorkflowSessionRepository>` in Axum application state, matching
/// `DashboardConfigurationRepository`'s convention.
#[async_trait::async_trait]
pub trait WorkflowSessionRepository: Send + Sync {
    /// Looks up a session by id. `Ok(None)` means no such session exists
    /// (not an error).
    async fn find_by_id(
        &self,
        session_id: Uuid,
    ) -> Result<Option<CrossCapabilityWorkflowSession>, crate::RepoError>;

    /// All of `consultant_id`'s *active* sessions — non-terminal `status`
    /// **and** not yet expired by time (both halves matter: a session can
    /// be time-expired before housekeeping flips its `status`, and such a
    /// session must not be reported as active either).
    async fn find_active_by_consultant_id(
        &self,
        consultant_id: &str,
    ) -> Result<Vec<CrossCapabilityWorkflowSession>, crate::RepoError>;

    /// Persists the full aggregate (insert-or-update on `session_id`).
    async fn save(&self, session: &CrossCapabilityWorkflowSession) -> Result<(), crate::RepoError>;

    /// Housekeeping sweep: bulk-transitions every non-terminal session whose
    /// `expires_at` is before `cutoff` to [`WorkflowSessionStatus::Expired`].
    /// Returns the number of rows affected. Already-terminal sessions and
    /// sessions not yet past `cutoff` are left untouched.
    async fn expire_older_than(&self, cutoff: DateTime<Utc>) -> Result<u64, crate::RepoError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: &str = "2026-01-01T00:00:00Z";

    fn t0() -> DateTime<Utc> {
        T0.parse().unwrap()
    }

    fn started_session() -> CrossCapabilityWorkflowSession {
        CrossCapabilityWorkflowSession::start("consultant-1", "sales", "lead-42", "commit", t0())
            .unwrap()
    }

    #[test]
    fn start_rejects_empty_consultant_id() {
        let err =
            CrossCapabilityWorkflowSession::start("", "sales", "lead-42", "commit", t0())
                .unwrap_err();
        assert_eq!(err, WorkflowSessionError::EmptyConsultantId);
    }

    #[test]
    fn start_rejects_empty_origin_capability() {
        let err =
            CrossCapabilityWorkflowSession::start("consultant-1", "", "lead-42", "commit", t0())
                .unwrap_err();
        assert_eq!(err, WorkflowSessionError::EmptyOriginCapability);
    }

    #[test]
    fn start_rejects_empty_origin_reference() {
        let err =
            CrossCapabilityWorkflowSession::start("consultant-1", "sales", "", "commit", t0())
                .unwrap_err();
        assert_eq!(err, WorkflowSessionError::EmptyOriginReference);
    }

    #[test]
    fn start_rejects_empty_target_capability() {
        let err =
            CrossCapabilityWorkflowSession::start("consultant-1", "sales", "lead-42", "", t0())
                .unwrap_err();
        assert_eq!(err, WorkflowSessionError::EmptyTargetCapability);
    }

    #[test]
    fn start_sets_the_default_ttl_from_now() {
        let session = started_session();
        assert_eq!(session.status(), WorkflowSessionStatus::Started);
        assert_eq!(
            session.expires_at(),
            t0() + Duration::minutes(DEFAULT_WORKFLOW_SESSION_TTL_MINUTES)
        );
        assert_eq!(session.target_reference(), None);
    }

    #[test]
    fn set_target_reference_rejects_blank_value() {
        let mut session = started_session();
        let err = session.set_target_reference("   ", t0()).unwrap_err();
        assert_eq!(err, WorkflowSessionError::EmptyTargetReference);
    }

    #[test]
    fn set_target_reference_accepts_a_value_before_expiry() {
        let mut session = started_session();
        session.set_target_reference("proposal-7", t0()).unwrap();
        assert_eq!(session.target_reference(), Some("proposal-7"));
    }

    #[test]
    fn set_target_reference_rejects_after_expiry() {
        let mut session = started_session();
        let after_expiry = session.expires_at() + Duration::seconds(1);

        let err = session.set_target_reference("proposal-7", after_expiry).unwrap_err();

        assert_eq!(err, WorkflowSessionError::SessionExpired { expires_at: session.expires_at() });
        assert_eq!(session.target_reference(), None);
    }

    // --- state machine ---------------------------------------------------

    #[test]
    fn valid_transition_started_to_in_progress_succeeds() {
        let mut session = started_session();
        session.transition_to(WorkflowSessionStatus::InProgress, t0()).unwrap();
        assert_eq!(session.status(), WorkflowSessionStatus::InProgress);
    }

    #[test]
    fn valid_transition_in_progress_to_completed_succeeds() {
        let mut session = started_session();
        session.transition_to(WorkflowSessionStatus::InProgress, t0()).unwrap();
        session.transition_to(WorkflowSessionStatus::Completed, t0()).unwrap();
        assert_eq!(session.status(), WorkflowSessionStatus::Completed);
    }

    #[test]
    fn valid_transition_in_progress_to_abandoned_succeeds() {
        let mut session = started_session();
        session.transition_to(WorkflowSessionStatus::InProgress, t0()).unwrap();
        session.transition_to(WorkflowSessionStatus::Abandoned, t0()).unwrap();
        assert_eq!(session.status(), WorkflowSessionStatus::Abandoned);
    }

    #[test]
    fn valid_transition_started_directly_to_abandoned_succeeds() {
        let mut session = started_session();
        session.transition_to(WorkflowSessionStatus::Abandoned, t0()).unwrap();
        assert_eq!(session.status(), WorkflowSessionStatus::Abandoned);
    }

    #[test]
    fn valid_transition_started_directly_to_expired_succeeds() {
        let mut session = started_session();
        session.transition_to(WorkflowSessionStatus::Expired, t0()).unwrap();
        assert_eq!(session.status(), WorkflowSessionStatus::Expired);
    }

    #[test]
    fn invalid_transition_started_to_completed_is_rejected() {
        let mut session = started_session();
        let err = session.transition_to(WorkflowSessionStatus::Completed, t0()).unwrap_err();
        assert_eq!(
            err,
            WorkflowSessionError::InvalidTransition {
                from: WorkflowSessionStatus::Started,
                to: WorkflowSessionStatus::Completed
            }
        );
        assert_eq!(session.status(), WorkflowSessionStatus::Started);
    }

    #[test]
    fn invalid_transition_started_to_started_is_rejected() {
        let mut session = started_session();
        let err = session.transition_to(WorkflowSessionStatus::Started, t0()).unwrap_err();
        assert_eq!(
            err,
            WorkflowSessionError::InvalidTransition {
                from: WorkflowSessionStatus::Started,
                to: WorkflowSessionStatus::Started
            }
        );
    }

    /// No-op-from-terminal-state: once `Completed`, no further transition
    /// (including to another terminal state) is valid.
    #[test]
    fn invalid_transition_out_of_completed_is_rejected() {
        let mut session = started_session();
        session.transition_to(WorkflowSessionStatus::InProgress, t0()).unwrap();
        session.transition_to(WorkflowSessionStatus::Completed, t0()).unwrap();

        let err = session.transition_to(WorkflowSessionStatus::Abandoned, t0()).unwrap_err();

        assert_eq!(
            err,
            WorkflowSessionError::InvalidTransition {
                from: WorkflowSessionStatus::Completed,
                to: WorkflowSessionStatus::Abandoned
            }
        );
        assert_eq!(session.status(), WorkflowSessionStatus::Completed);
    }

    #[test]
    fn invalid_transition_out_of_abandoned_is_rejected() {
        let mut session = started_session();
        session.transition_to(WorkflowSessionStatus::Abandoned, t0()).unwrap();

        let err = session.transition_to(WorkflowSessionStatus::InProgress, t0()).unwrap_err();

        assert_eq!(
            err,
            WorkflowSessionError::InvalidTransition {
                from: WorkflowSessionStatus::Abandoned,
                to: WorkflowSessionStatus::InProgress
            }
        );
    }

    #[test]
    fn invalid_transition_out_of_expired_is_rejected() {
        let mut session = started_session();
        session.transition_to(WorkflowSessionStatus::Expired, t0()).unwrap();

        let err = session.transition_to(WorkflowSessionStatus::InProgress, t0()).unwrap_err();

        assert_eq!(
            err,
            WorkflowSessionError::InvalidTransition {
                from: WorkflowSessionStatus::Expired,
                to: WorkflowSessionStatus::InProgress
            }
        );
    }

    #[test]
    fn is_valid_transition_exhaustively_matches_the_documented_matrix() {
        use WorkflowSessionStatus::*;
        let valid_pairs = [
            (Started, InProgress),
            (Started, Abandoned),
            (Started, Expired),
            (InProgress, Completed),
            (InProgress, Abandoned),
            (InProgress, Expired),
        ];
        let all = [Started, InProgress, Completed, Abandoned, Expired];

        for from in all {
            for to in all {
                let expected = valid_pairs.contains(&(from, to));
                assert_eq!(
                    WorkflowSessionStatus::is_valid_transition(from, to),
                    expected,
                    "from={from} to={to}"
                );
            }
        }
    }

    // --- TTL ---------------------------------------------------------------

    #[test]
    fn is_expired_false_strictly_before_expires_at() {
        let session = started_session();
        let just_before = session.expires_at() - Duration::seconds(1);
        assert!(!session.is_expired(just_before));
    }

    #[test]
    fn is_expired_true_exactly_at_expires_at() {
        let session = started_session();
        assert!(session.is_expired(session.expires_at()));
    }

    #[test]
    fn is_expired_true_after_expires_at() {
        let session = started_session();
        let just_after = session.expires_at() + Duration::seconds(1);
        assert!(session.is_expired(just_after));
    }

    /// TTL enforcement is about time, not just status: an `InProgress`
    /// session (non-terminal) whose TTL has silently elapsed must still
    /// reject a transition attempt, even though the state machine itself
    /// would otherwise allow `InProgress -> Completed`.
    #[test]
    fn transition_attempt_on_time_expired_but_non_terminal_status_session_is_rejected() {
        let mut session = started_session();
        session.transition_to(WorkflowSessionStatus::InProgress, t0()).unwrap();

        let after_expiry = session.expires_at() + Duration::seconds(1);
        let err = session.transition_to(WorkflowSessionStatus::Completed, after_expiry).unwrap_err();

        assert_eq!(err, WorkflowSessionError::SessionExpired { expires_at: session.expires_at() });
        assert_eq!(session.status(), WorkflowSessionStatus::InProgress);
    }

    #[test]
    fn transition_attempt_exactly_at_expires_at_is_rejected() {
        let mut session = started_session();
        let err =
            session.transition_to(WorkflowSessionStatus::InProgress, session.expires_at()).unwrap_err();
        assert_eq!(err, WorkflowSessionError::SessionExpired { expires_at: session.expires_at() });
    }

    // --- status wire format --------------------------------------------------

    #[test]
    fn status_round_trips_through_as_str_and_from_str() {
        for status in
            [
                WorkflowSessionStatus::Started,
                WorkflowSessionStatus::InProgress,
                WorkflowSessionStatus::Completed,
                WorkflowSessionStatus::Abandoned,
                WorkflowSessionStatus::Expired,
            ]
        {
            assert_eq!(status.as_str().parse::<WorkflowSessionStatus>().unwrap(), status);
        }
    }

    #[test]
    fn status_from_str_rejects_unknown_value() {
        let err = "not_a_real_status".parse::<WorkflowSessionStatus>().unwrap_err();
        assert_eq!(err.to_string(), "unknown workflow session status: \"not_a_real_status\"");
    }

    #[test]
    fn terminal_states_report_is_terminal_true() {
        assert!(WorkflowSessionStatus::Completed.is_terminal());
        assert!(WorkflowSessionStatus::Abandoned.is_terminal());
        assert!(WorkflowSessionStatus::Expired.is_terminal());
        assert!(!WorkflowSessionStatus::Started.is_terminal());
        assert!(!WorkflowSessionStatus::InProgress.is_terminal());
    }

    #[test]
    fn from_parts_rejects_a_blank_target_reference() {
        let err = CrossCapabilityWorkflowSession::from_parts(
            Uuid::new_v4(),
            "consultant-1".to_string(),
            "sales".to_string(),
            "lead-42".to_string(),
            "commit".to_string(),
            Some("   ".to_string()),
            WorkflowSessionStatus::Started,
            t0() + Duration::minutes(DEFAULT_WORKFLOW_SESSION_TTL_MINUTES),
        )
        .unwrap_err();
        assert_eq!(err, WorkflowSessionError::EmptyTargetReference);
    }
}
