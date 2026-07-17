//! auth: session model + `SessionProvider` trait (ADR-008), plus a
//! feature-gated (`dev-auth`) dev-stub session provider for pre-Armor-
//! integration phases (`implementation-plan.md` §6 risk #2).
//!
//! `auth` is deliberately separated from `bff-core` (ADR-004) so the
//! interim dev-stub can be swapped for a real Armor/OIDC-backed provider
//! without touching domain code. No real Armor/OIDC integration exists
//! here yet — that is explicitly out of scope for this unit; the real
//! contract is unknown until Armor's auth endpoint is confirmed
//! (ADR-008 "Negative / Trade-offs").

use chrono::{DateTime, Utc};
use uuid::Uuid;

#[cfg(feature = "dev-auth")]
pub mod dev_stub;

/// A BFF-managed server-side session (ADR-008): the browser holds only an
/// opaque `session_id` (in an `HttpOnly`/`Secure`/`SameSite=Strict` cookie,
/// set up by U11 — not this crate), which maps to this record identifying
/// the authenticated consultant and when the session grant expires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Opaque session identifier; this is the value stored in the
    /// browser's session cookie.
    pub session_id: Uuid,
    /// Identity of the authenticated consultant. Deliberately just a
    /// reference id, not locally-held identity data, per
    /// `consultant-experience-context.md`'s "Consultant" ubiquitous-
    /// language term.
    pub consultant_id: String,
    /// When this session's grant expires.
    pub expires_at: DateTime<Utc>,
}

/// Errors a [`SessionProvider`] can return.
///
/// Deliberately a small, `auth`-local error type rather than reusing
/// `sqlx::Error` directly: `auth` is a trait-interface-only dependency
/// (ADR-004), and coupling its trait surface to `persistence`'s storage
/// error type would be exactly the kind of leak ADR-004 warns against. A
/// real, Postgres-backed `SessionProvider` (landing in U11) is expected to
/// map `sqlx::Error` into this type at its own boundary.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// The underlying session store could not be reached/used at all
    /// (e.g. a database connection failure).
    #[error("session store unavailable: {0}")]
    StoreUnavailable(String),
    /// The store was reachable, but the lookup/write itself failed.
    #[error("session operation failed: {0}")]
    OperationFailed(String),
}

/// Looks up an existing session by id.
///
/// Async because real implementations hit a database/network (ADR-008:
/// sessions are persisted in the ADR-010 datastore so they survive a BFF
/// instance restart under horizontal scaling). `Send + Sync` so
/// implementations can be shared behind an `Arc<dyn SessionProvider>` in
/// Axum application state (U11).
#[async_trait::async_trait]
pub trait SessionProvider: Send + Sync {
    /// Returns `Ok(Some(session))` if `session_id` maps to a known
    /// session, `Ok(None)` if it does not (e.g. unknown or expired id —
    /// callers decide how to treat expiry), or `Err` if the lookup itself
    /// failed.
    async fn get_session(&self, session_id: Uuid) -> Result<Option<Session>, SessionError>;
}
