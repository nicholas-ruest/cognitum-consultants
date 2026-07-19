//! `Prospect` aggregate (ADR-020 part A) and its repository port
//! (`ProspectRepository`, implemented against Postgres in `persistence`).
//!
//! Unlike every other aggregate in this crate, a `Prospect` has no upstream
//! Nexus event driving it and no ACL boundary to respect — it is entirely
//! consultant-authored prospecting data, created/read/updated/deleted
//! end-to-end within this repo (ADR-020's own justification: tracking your
//! own prospecting notes is not a decision or record any of the ten
//! external capabilities owns).
//!
//! Invariants enforced here:
//! 1. Belongs to exactly one consultant — `consultant_id` set at
//!    construction, immutable. Ownership (a consultant only reading/
//!    mutating their *own* prospects) is enforced at the `bff-api` route
//!    layer, the same "load by id, compare `consultant_id`, 404 if not
//!    yours" convention `crate::notifications`'s write routes already use —
//!    not this aggregate's job, which has no notion of "the current
//!    session."
//! 2. `company_name` must be non-empty.
//! 3. Stage transitions follow [`ProspectStage::is_valid_transition`]'s
//!    matrix: linear progression through the deal funnel, with
//!    [`ProspectStage::ClosedLost`] reachable from any non-terminal stage
//!    (a deal can die at any point, not only at the end of the funnel), and
//!    no transition ever valid out of a terminal stage
//!    ([`ProspectStage::ClosedWon`]/[`ProspectStage::ClosedLost`]).
//! 4. Notes are append-only — [`Prospect::add_note`] pushes, nothing
//!    removes or edits an existing [`ProspectNote`], so a prospect's history
//!    stays a true history (mirrors `ActionQueueEntry`'s non-mutation of its
//!    own confirmation trail).

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::RepoError;

/// A [`Prospect`]'s position in the deal funnel (ADR-020). Ordered by real
/// deal progression — not the flat, unordered list the original request
/// named (which put "closed" third) — with `ClosedWon`/`ClosedLost` as two
/// distinct terminal states rather than one bare "closed": a funnel that
/// can't represent a lost deal isn't a funnel, just a tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProspectStage {
    Contacted,
    AppointmentScheduled,
    NdaSent,
    NdaSigned,
    RfpSent,
    RfpSigned,
    ProposalSent,
    ProposalSigned,
    SowSent,
    ClosedWon,
    ClosedLost,
}

impl ProspectStage {
    /// The wire/storage string for this stage (DB `stage` column).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Contacted => "contacted",
            Self::AppointmentScheduled => "appointment_scheduled",
            Self::NdaSent => "nda_sent",
            Self::NdaSigned => "nda_signed",
            Self::RfpSent => "rfp_sent",
            Self::RfpSigned => "rfp_signed",
            Self::ProposalSent => "proposal_sent",
            Self::ProposalSigned => "proposal_signed",
            Self::SowSent => "sow_sent",
            Self::ClosedWon => "closed_won",
            Self::ClosedLost => "closed_lost",
        }
    }

    /// The stage every freshly-created [`Prospect`] starts at.
    pub fn initial() -> Self {
        Self::Contacted
    }

    /// No transition is ever valid out of a terminal stage — matches
    /// `WorkflowSessionStatus::is_terminal`'s convention.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::ClosedWon | Self::ClosedLost)
    }

    /// Invariant 3: the funnel's linear forward progression, plus
    /// `ClosedLost` reachable from any non-terminal stage (checked before
    /// the linear matrix, so it doesn't need to be repeated for every
    /// stage), consistent with [`Self::is_terminal`] by construction (every
    /// arm here that returns `true` has a non-terminal `from`).
    pub fn is_valid_transition(from: Self, to: Self) -> bool {
        use ProspectStage::*;

        if from.is_terminal() {
            return false;
        }
        if to == ClosedLost {
            return true;
        }

        matches!(
            (from, to),
            (Contacted, AppointmentScheduled)
                | (AppointmentScheduled, NdaSent)
                | (NdaSent, NdaSigned)
                | (NdaSigned, RfpSent)
                | (RfpSent, RfpSigned)
                | (RfpSigned, ProposalSent)
                | (ProposalSent, ProposalSigned)
                | (ProposalSigned, SowSent)
                | (SowSent, ClosedWon)
        )
    }
}

impl fmt::Display for ProspectStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A stage string that isn't a known [`ProspectStage`] variant.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown prospect stage: {0:?}")]
pub struct ParseProspectStageError(String);

impl FromStr for ProspectStage {
    type Err = ParseProspectStageError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "contacted" => Ok(Self::Contacted),
            "appointment_scheduled" => Ok(Self::AppointmentScheduled),
            "nda_sent" => Ok(Self::NdaSent),
            "nda_signed" => Ok(Self::NdaSigned),
            "rfp_sent" => Ok(Self::RfpSent),
            "rfp_signed" => Ok(Self::RfpSigned),
            "proposal_sent" => Ok(Self::ProposalSent),
            "proposal_signed" => Ok(Self::ProposalSigned),
            "sow_sent" => Ok(Self::SowSent),
            "closed_won" => Ok(Self::ClosedWon),
            "closed_lost" => Ok(Self::ClosedLost),
            other => Err(ParseProspectStageError(other.to_string())),
        }
    }
}

/// One append-only note on a [`Prospect`] (invariant 4). Child entity — no
/// independent identity/lifecycle outside the `Prospect` that owns it,
/// same relationship `CardPlacement` has to `DashboardConfiguration`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProspectNote {
    id: Uuid,
    body: String,
    author_consultant_id: String,
    created_at: DateTime<Utc>,
}

impl ProspectNote {
    /// Reconstructs a note from already-known parts (e.g. a repository
    /// loading a persisted row) — the counterpart to
    /// [`Prospect::add_note`], which only ever appends to an
    /// already-in-memory aggregate and so can't build the initial `Vec`
    /// [`Prospect::from_parts`] needs when hydrating from storage.
    /// Re-validates `body` the same as `add_note` would.
    pub fn from_parts(
        id: Uuid,
        body: String,
        author_consultant_id: String,
        created_at: DateTime<Utc>,
    ) -> Result<Self, ProspectError> {
        if body.trim().is_empty() {
            return Err(ProspectError::EmptyNoteBody);
        }
        Ok(Self { id, body, author_consultant_id, created_at })
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    pub fn author_consultant_id(&self) -> &str {
        &self.author_consultant_id
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }
}

/// A single consultant's tracked prospect, moving through the deal funnel
/// (ADR-020). Root of its own aggregate; contains [`ProspectNote`] child
/// entities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prospect {
    id: Uuid,
    consultant_id: String,
    company_name: String,
    contact_name: Option<String>,
    stage: ProspectStage,
    notes: Vec<ProspectNote>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// Errors constructing/mutating a [`Prospect`] aggregate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProspectError {
    /// `consultant_id` was empty/blank.
    #[error("consultant_id must not be empty")]
    EmptyConsultantId,
    /// `company_name` was empty/blank.
    #[error("company_name must not be empty")]
    EmptyCompanyName,
    /// A `Some("")`/blank `contact_name` was supplied — `None` is the
    /// correct way to say "no contact name yet", not an empty string.
    #[error("contact_name must not be empty when present")]
    EmptyContactName,
    /// Invariant 3: `ProspectStage::is_valid_transition(from, to)` was
    /// `false`.
    #[error("cannot transition prospect from {from} to {to}")]
    InvalidStageTransition { from: ProspectStage, to: ProspectStage },
    /// A note's `body` was empty/blank.
    #[error("note body must not be empty")]
    EmptyNoteBody,
}

impl Prospect {
    /// Creates a brand-new prospect at [`ProspectStage::initial`], with a
    /// fresh `id` and no notes yet.
    pub fn new(
        consultant_id: impl Into<String>,
        company_name: impl Into<String>,
        contact_name: Option<String>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, ProspectError> {
        Self::from_parts(
            Uuid::new_v4(),
            consultant_id.into(),
            company_name.into(),
            contact_name,
            ProspectStage::initial(),
            Vec::new(),
            created_at,
            created_at,
        )
    }

    /// Reconstructs an aggregate from already-known parts (e.g. a
    /// repository loading a persisted row). Re-validates every field the
    /// same as [`Self::new`] would.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        id: Uuid,
        consultant_id: String,
        company_name: String,
        contact_name: Option<String>,
        stage: ProspectStage,
        notes: Vec<ProspectNote>,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Result<Self, ProspectError> {
        if consultant_id.trim().is_empty() {
            return Err(ProspectError::EmptyConsultantId);
        }
        if company_name.trim().is_empty() {
            return Err(ProspectError::EmptyCompanyName);
        }
        if let Some(name) = &contact_name
            && name.trim().is_empty()
        {
            return Err(ProspectError::EmptyContactName);
        }

        Ok(Self { id, consultant_id, company_name, contact_name, stage, notes, created_at, updated_at })
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn consultant_id(&self) -> &str {
        &self.consultant_id
    }

    pub fn company_name(&self) -> &str {
        &self.company_name
    }

    pub fn contact_name(&self) -> Option<&str> {
        self.contact_name.as_deref()
    }

    pub fn stage(&self) -> ProspectStage {
        self.stage
    }

    pub fn notes(&self) -> &[ProspectNote] {
        &self.notes
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }

    /// Invariant 3: moves this prospect to `to`, rejecting the call with
    /// [`ProspectError::InvalidStageTransition`] (leaving `stage`
    /// unchanged) if `ProspectStage::is_valid_transition(self.stage, to)`
    /// is `false`.
    pub fn transition_stage(&mut self, to: ProspectStage, now: DateTime<Utc>) -> Result<(), ProspectError> {
        if !ProspectStage::is_valid_transition(self.stage, to) {
            return Err(ProspectError::InvalidStageTransition { from: self.stage, to });
        }
        self.stage = to;
        self.updated_at = now;
        Ok(())
    }

    /// Invariant 4: appends a new, immutable [`ProspectNote`]. Rejects an
    /// empty/blank `body`.
    pub fn add_note(
        &mut self,
        body: impl Into<String>,
        author_consultant_id: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<(), ProspectError> {
        let body = body.into();
        if body.trim().is_empty() {
            return Err(ProspectError::EmptyNoteBody);
        }

        self.notes.push(ProspectNote {
            id: Uuid::new_v4(),
            body,
            author_consultant_id: author_consultant_id.into(),
            created_at: now,
        });
        self.updated_at = now;
        Ok(())
    }
}

/// Repository port for [`Prospect`]. Implemented against Postgres in
/// `persistence` (ADR-010); `bff-core` only defines the interface, per
/// ADR-004's trait-interface-only dependency direction.
///
/// `Send + Sync` so implementations can be shared behind an
/// `Arc<dyn ProspectRepository>` in Axum application state, matching every
/// other repository trait's convention.
#[async_trait::async_trait]
pub trait ProspectRepository: Send + Sync {
    /// All of `consultant_id`'s prospects, newest first. Route-layer
    /// ownership scoping (invariant 1) starts here.
    async fn find_by_consultant_id(&self, consultant_id: &str) -> Result<Vec<Prospect>, RepoError>;

    /// Looks up a single prospect by id, regardless of owner — callers
    /// (`bff-api` route handlers) are responsible for comparing
    /// `Prospect::consultant_id()` against the current session before
    /// acting on the result (see the module docs' invariant 1 note).
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Prospect>, RepoError>;

    /// Persists the full aggregate — upsert semantics on `id`, including
    /// its full `notes` history.
    async fn save(&self, prospect: &Prospect) -> Result<(), RepoError>;

    /// Deletes a prospect entirely. Not an error if `id` is unknown.
    async fn delete(&self, id: Uuid) -> Result<(), RepoError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> DateTime<Utc> {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    fn prospect() -> Prospect {
        Prospect::new("consultant-1", "Acme Corp", Some("Jane Doe".to_string()), t0()).unwrap()
    }

    #[test]
    fn new_rejects_empty_consultant_id() {
        let err = Prospect::new("", "Acme", None, t0()).unwrap_err();
        assert_eq!(err, ProspectError::EmptyConsultantId);
    }

    #[test]
    fn new_rejects_empty_company_name() {
        let err = Prospect::new("consultant-1", "", None, t0()).unwrap_err();
        assert_eq!(err, ProspectError::EmptyCompanyName);
    }

    #[test]
    fn new_rejects_blank_contact_name() {
        let err = Prospect::new("consultant-1", "Acme", Some("   ".to_string()), t0()).unwrap_err();
        assert_eq!(err, ProspectError::EmptyContactName);
    }

    #[test]
    fn new_starts_at_the_initial_stage_with_no_notes() {
        let p = prospect();
        assert_eq!(p.stage(), ProspectStage::Contacted);
        assert!(p.notes().is_empty());
        assert_eq!(p.created_at(), p.updated_at());
    }

    #[test]
    fn transition_stage_follows_the_linear_funnel() {
        let mut p = prospect();
        let t1 = t0() + chrono::Duration::hours(1);

        p.transition_stage(ProspectStage::AppointmentScheduled, t1).unwrap();

        assert_eq!(p.stage(), ProspectStage::AppointmentScheduled);
        assert_eq!(p.updated_at(), t1);
    }

    #[test]
    fn transition_stage_rejects_skipping_ahead() {
        let mut p = prospect();
        let err = p.transition_stage(ProspectStage::ProposalSent, t0()).unwrap_err();

        assert_eq!(
            err,
            ProspectError::InvalidStageTransition { from: ProspectStage::Contacted, to: ProspectStage::ProposalSent }
        );
        assert_eq!(p.stage(), ProspectStage::Contacted, "a rejected transition must not mutate state");
    }

    #[test]
    fn transition_stage_allows_closed_lost_from_any_non_terminal_stage() {
        let mut p = prospect();
        p.transition_stage(ProspectStage::AppointmentScheduled, t0()).unwrap();
        p.transition_stage(ProspectStage::NdaSent, t0()).unwrap();

        p.transition_stage(ProspectStage::ClosedLost, t0()).unwrap();

        assert_eq!(p.stage(), ProspectStage::ClosedLost);
    }

    #[test]
    fn transition_stage_rejects_any_transition_out_of_a_terminal_stage() {
        let mut p = prospect();
        p.transition_stage(ProspectStage::ClosedLost, t0()).unwrap();

        let err = p.transition_stage(ProspectStage::Contacted, t0()).unwrap_err();

        assert_eq!(
            err,
            ProspectError::InvalidStageTransition { from: ProspectStage::ClosedLost, to: ProspectStage::Contacted }
        );
    }

    #[test]
    fn full_funnel_reaches_closed_won() {
        use ProspectStage::*;
        let mut p = prospect();
        for stage in [
            AppointmentScheduled,
            NdaSent,
            NdaSigned,
            RfpSent,
            RfpSigned,
            ProposalSent,
            ProposalSigned,
            SowSent,
            ClosedWon,
        ] {
            p.transition_stage(stage, t0()).unwrap();
        }
        assert_eq!(p.stage(), ClosedWon);
    }

    #[test]
    fn add_note_appends_without_removing_prior_notes() {
        let mut p = prospect();
        let t1 = t0() + chrono::Duration::hours(1);

        p.add_note("First call went well.", "consultant-1", t0()).unwrap();
        p.add_note("Sent follow-up materials.", "consultant-1", t1).unwrap();

        assert_eq!(p.notes().len(), 2);
        assert_eq!(p.notes()[0].body(), "First call went well.");
        assert_eq!(p.notes()[1].body(), "Sent follow-up materials.");
        assert_eq!(p.updated_at(), t1);
    }

    #[test]
    fn add_note_rejects_an_empty_body() {
        let mut p = prospect();
        let err = p.add_note("   ", "consultant-1", t0()).unwrap_err();
        assert_eq!(err, ProspectError::EmptyNoteBody);
        assert!(p.notes().is_empty());
    }

    #[test]
    fn stage_round_trips_through_as_str_and_from_str() {
        use ProspectStage::*;
        for stage in [
            Contacted,
            AppointmentScheduled,
            NdaSent,
            NdaSigned,
            RfpSent,
            RfpSigned,
            ProposalSent,
            ProposalSigned,
            SowSent,
            ClosedWon,
            ClosedLost,
        ] {
            assert_eq!(stage.as_str().parse::<ProspectStage>().unwrap(), stage);
        }
    }

    #[test]
    fn stage_from_str_rejects_unknown_value() {
        let err = "not_a_real_stage".parse::<ProspectStage>().unwrap_err();
        assert_eq!(err.to_string(), "unknown prospect stage: \"not_a_real_stage\"");
    }
}
