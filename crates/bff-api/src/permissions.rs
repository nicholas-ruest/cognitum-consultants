//! Permission-aware presentation enforcement (ADR-009, PROMPT-15).
//!
//! This module is the BFF's "layer 1" (server-side filtering) from
//! ADR-009's three-layer enforcement model: it never computes or overrides
//! an authorization decision — it only caches and re-serves the
//! [`PermissionAssertion`]s Armor already granted, and short-circuits a
//! handler with `403 Forbidden` before that handler would otherwise
//! attempt a Nexus call the consultant has no assertion for. The real,
//! authoritative authorization check always happens downstream, in the
//! owning capability (ADR-009 "layer 3") — this cache is a UX/latency
//! optimization and a defense-in-depth check on this repo's own
//! aggregates, never a substitute for that.
//!
//! # Cache key: `consultant_id`, not `session_id`
//! ADR-009 says assertions are cached "in-memory per session", but
//! [`PermissionAssertion`] itself is intrinsically consultant-scoped (its
//! own shape is `{ consultant_id, capability, scope, expires_at }` — no
//! `session_id` field), [`crate::session::Session`] carries a
//! `consultant_id` (not a servable "credential" a second concurrent
//! session would differ on), and `ArmorGateway::fetch_assertions` is
//! keyed by `consultant_id`. Keying this cache by `consultant_id` rather
//! than `session_id` is therefore both the simpler option and the more
//! correct one: if a consultant somehow held two concurrent sessions,
//! Armor would grant both the *same* assertion set, so sharing one cache
//! entry between them avoids a redundant Armor round-trip and avoids the
//! two sessions ever observing divergent permission state. Nothing in
//! ADR-009 or `consultant-experience-context.md` distinguishes "per
//! session" from "per consultant" beyond that phrasing, so this reads as
//! "cached for the duration of a session" rather than "keyed by session
//! id" — the acceptance-criteria-suggested signature
//! (`is_permitted(&consultant_id, capability)`) confirms this reading.
//!
//! # TTL: bounded by the minimum `expires_at` across the fetched set
//! Per ADR-009 ("bounded by the assertion's own expiry so staleness is
//! never unbounded"), a cache entry's TTL is **not** a fixed duration —
//! it is computed as the minimum `expires_at` across every assertion
//! returned in that fetch. This means the cache re-fetches as soon as the
//! *first* assertion in the set would expire, even if every other
//! assertion in the set is still valid; that is a deliberate
//! over-cautious choice (a partially-stale set is still a stale set) and
//! is what "TTL bounded by the shortest `expires_at`" means for a set,
//! not a single record.
//!
//! An empty assertion set (a consultant Armor currently grants nothing to)
//! has no `expires_at` to derive a bound from, so it falls back to
//! [`EMPTY_ASSERTIONS_TTL`] — a short, fixed window chosen so a
//! newly-granted permission is picked up reasonably quickly rather than
//! caching "no permissions" indefinitely.
//!
//! # Forward-compatibility: event-driven invalidation (U30)
//! [`PermissionCache::invalidate`] exists today even though nothing calls
//! it yet. Once U30 lands a `PermissionAssertionChanged` consumer, that
//! consumer should call `invalidate(consultant_id)` on receipt so the next
//! `is_permitted` lookup re-fetches from Armor immediately rather than
//! waiting out the TTL — the cache is structured (a plain keyed map behind
//! a lock, no background timers/tasks) so that slots in without any
//! restructuring.
//!
//! # `credential` placeholder, pending a real per-session upstream token
//! `ArmorGateway::fetch_assertions` takes a `credential` to attach to the
//! outbound Armor call (ADR-008: "the consultant's session-derived
//! token"). No real upstream token exists yet — `auth::Session` (U11,
//! ADR-008 "Interim dev-stub") carries only `session_id`/`consultant_id`/
//! `expires_at`, not a forwardable credential. Following the same
//! "interim dev-stub, no real credential check" precedent already
//! established for session auth itself, this module passes `consultant_id`
//! as the credential placeholder. Replace this with the real per-session
//! upstream token once ADR-008's real Armor/OIDC integration lands.

use std::collections::HashMap;
use std::sync::Arc;

use auth::Session;
use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use nexus_client::{ArmorGateway, ArmorGatewayError, PermissionAssertion};
use serde_json::json;
use tokio::sync::RwLock;

use crate::session::{self, AppState};

/// Fallback TTL applied when a fetched assertion set is empty (no
/// `expires_at` exists to derive a bound from). See the module docs.
const EMPTY_ASSERTIONS_TTL: ChronoDuration = ChronoDuration::seconds(30);

struct CacheEntry {
    assertions: Vec<PermissionAssertion>,
    /// The minimum `expires_at` across `assertions` (or `Utc::now() +
    /// EMPTY_ASSERTIONS_TTL` when `assertions` is empty) — this entry must
    /// not be served once `Utc::now()` passes this point.
    expires_at: DateTime<Utc>,
}

/// In-memory, per-consultant cache of Armor's [`PermissionAssertion`]s
/// (ADR-009). See the module docs for the keying decision, TTL semantics,
/// and forward-compatibility notes.
pub struct PermissionCache {
    gateway: Arc<dyn ArmorGateway>,
    entries: RwLock<HashMap<String, CacheEntry>>,
}

impl PermissionCache {
    pub fn new(gateway: Arc<dyn ArmorGateway>) -> Self {
        Self { gateway, entries: RwLock::new(HashMap::new()) }
    }

    /// Returns whether `consultant_id` currently holds a
    /// [`PermissionAssertion`] for `capability`.
    ///
    /// **Scope of this check, today: capability name only.** No route
    /// exists yet whose access depends on `scope` (Sales et al. land in
    /// PROMPT-24+), so there is nothing concrete to test scope-matching
    /// against; this deliberately checks only whether *some* assertion for
    /// `capability` exists, regardless of its `scope`. Extend this (e.g.
    /// an `is_permitted_in_scope(consultant_id, capability, scope)`
    /// variant, or a `scope` parameter here) once a real capability-scoped
    /// route needs it.
    ///
    /// On a cache miss or TTL expiry, this fetches fresh assertions via
    /// [`ArmorGateway::fetch_assertions`]. If that fetch itself fails
    /// (Armor/Nexus unreachable, etc.), this fails closed — returns
    /// `false` — rather than serving a stale cache entry or assuming
    /// permission, matching the same fail-closed treatment
    /// `session::resolve_session` already applies to session lookups.
    pub async fn is_permitted(&self, consultant_id: &str, capability: &str) -> bool {
        match self.get_or_refresh(consultant_id).await {
            Ok(assertions) => assertions.iter().any(|assertion| assertion.capability == capability),
            Err(err) => {
                tracing::error!(
                    error = %err,
                    consultant_id,
                    capability,
                    "permission assertion fetch failed; denying by default"
                );
                false
            }
        }
    }

    /// Returns the full current set of [`PermissionAssertion`]s cached (or
    /// freshly fetched) for `consultant_id` — the full grant set, not a
    /// single-capability check (ADR-009, PROMPT-19's `GET /api/session`
    /// `permission_assertions` field, consumed client-side as a UX/
    /// rendering-only signal — see `../../../.plans/adr/ADR-009-authorization-permission-aware-presentation.md`
    /// layer 2; it is never itself an enforcement decision).
    ///
    /// Shares [`Self::get_or_refresh`] with [`Self::is_permitted`] so both
    /// methods populate/read the same cache entries under the same TTL
    /// rules rather than duplicating the fetch-and-cache logic.
    ///
    /// On a fetch failure this fails closed — returns an empty `Vec` rather
    /// than a stale entry or a propagated error — matching `is_permitted`'s
    /// fail-closed treatment of gateway errors. An empty result here means
    /// "render no permission-gated nav items", never "the consultant has no
    /// session"; that distinction is `require_session`'s job, not this
    /// method's.
    pub async fn assertions_for(&self, consultant_id: &str) -> Vec<PermissionAssertion> {
        match self.get_or_refresh(consultant_id).await {
            Ok(assertions) => assertions,
            Err(err) => {
                tracing::error!(
                    error = %err,
                    consultant_id,
                    "permission assertion fetch failed; returning empty assertion set"
                );
                Vec::new()
            }
        }
    }

    /// Clears any cached entry for `consultant_id`, forcing the next
    /// `is_permitted` call to re-fetch from Armor. Nothing calls this yet
    /// — see the module docs' "Forward-compatibility" section for why it
    /// exists regardless (U30's future `PermissionAssertionChanged`
    /// consumer).
    pub async fn invalidate(&self, consultant_id: &str) {
        self.entries.write().await.remove(consultant_id);
    }

    async fn get_or_refresh(&self, consultant_id: &str) -> Result<Vec<PermissionAssertion>, ArmorGatewayError> {
        if let Some(assertions) = self.cached_if_fresh(consultant_id).await {
            return Ok(assertions);
        }
        self.refresh(consultant_id).await
    }

    async fn cached_if_fresh(&self, consultant_id: &str) -> Option<Vec<PermissionAssertion>> {
        let entries = self.entries.read().await;
        let entry = entries.get(consultant_id)?;
        (entry.expires_at > Utc::now()).then(|| entry.assertions.clone())
    }

    async fn refresh(&self, consultant_id: &str) -> Result<Vec<PermissionAssertion>, ArmorGatewayError> {
        // Placeholder credential — see the module docs' "`credential`
        // placeholder" section.
        let assertions = self.gateway.fetch_assertions(consultant_id, consultant_id).await?;

        let expires_at =
            assertions.iter().map(|assertion| assertion.expires_at).min().unwrap_or_else(|| Utc::now() + EMPTY_ASSERTIONS_TTL);

        let mut entries = self.entries.write().await;
        entries.insert(consultant_id.to_owned(), CacheEntry { assertions: assertions.clone(), expires_at });

        Ok(assertions)
    }
}

/// Query params for the temporary diagnostic route below.
#[derive(Debug, serde::Deserialize)]
pub struct PermissionCheckParams {
    capability: String,
}

/// Extractor proving the `is_permitted` + `403` short-circuit mechanism
/// (ADR-009) at the handler level: applying `RequirePermission` as a
/// handler parameter rejects the request with `403 Forbidden` *before*
/// the handler body runs if the current session's consultant has no
/// assertion for the requested capability — exactly the "short-circuit
/// with 403 before attempting any Nexus call" behavior ADR-009 requires,
/// just with no Nexus call to protect yet (see
/// [`permission_check_example`]'s doc comment).
///
/// This particular extractor reads its target `capability` from the
/// request's `?capability=` query parameter, which only makes sense for
/// this temporary, capability-supplied-by-the-caller diagnostic route.
/// A real protected route (PROMPT-24 Sales, etc.) will know its required
/// capability at compile time, and should express that as a fixed
/// argument to `PermissionCache::is_permitted` inside the handler (or a
/// small compile-time-capability-parameterized wrapper built the same
/// way), not by reading it from the query string.
pub struct RequirePermission;

impl FromRequestParts<AppState> for RequirePermission {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let Query(params) = Query::<PermissionCheckParams>::from_request_parts(parts, state)
            .await
            .map_err(|err| (StatusCode::BAD_REQUEST, Json(json!({ "error": err.to_string() }))).into_response())?;

        // `require_session` (applied to every route this extractor is used
        // on, see `diagnostic_router` below) always inserts a `Session`
        // before a handler/extractor runs, but this is defensive rather
        // than assumed, in case the extractor is ever reused on a route
        // that forgot the middleware.
        let session = parts
            .extensions
            .get::<Session>()
            .cloned()
            .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(json!({ "error": "unauthorized" }))).into_response())?;

        if state.permission_cache.is_permitted(&session.consultant_id, &params.capability).await {
            Ok(RequirePermission)
        } else {
            Err((StatusCode::FORBIDDEN, Json(json!({ "permitted": false }))).into_response())
        }
    }
}

/// `GET /api/_permission-check-example?capability=X`
///
/// **Temporary, proof-of-mechanism-only route — delete once a real
/// protected route exists.** No capability-specific business route exists
/// in this repo yet (Sales lands in PROMPT-24 as the first real consumer
/// of `is_permitted`), so there is no actual downstream Nexus call for
/// this route to short-circuit before making. Its only purpose is to
/// prove, end-to-end through a real HTTP request, that
/// `RequirePermission`/`is_permitted` correctly returns `200` for a
/// granted capability and `403` for one the consultant has no assertion
/// for. Once PROMPT-24 lands, remove this route (and, if unused
/// elsewhere by then, this handler) in favor of that real route
/// demonstrating the same mechanism.
pub async fn permission_check_example(_permitted: RequirePermission) -> Response {
    (StatusCode::OK, Json(json!({ "permitted": true }))).into_response()
}

/// Builds the diagnostic route's sub-router, with the same
/// [`session::require_session`] middleware [`session::protected_router`]
/// applies, so an unauthenticated request still 401s (never reaching
/// `RequirePermission`) rather than 403ing.
pub fn diagnostic_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/_permission-check-example", get(permission_check_example))
        .layer(axum::middleware::from_fn_with_state(state, session::require_session))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use chrono::{Duration as ChronoDuration, Utc};

    use super::*;

    struct MockArmorGateway {
        assertions: Vec<PermissionAssertion>,
        call_count: AtomicUsize,
    }

    impl MockArmorGateway {
        fn new(assertions: Vec<PermissionAssertion>) -> Self {
            Self { assertions, call_count: AtomicUsize::new(0) }
        }

        fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl ArmorGateway for MockArmorGateway {
        async fn fetch_assertions(
            &self,
            _consultant_id: &str,
            _credential: &str,
        ) -> Result<Vec<PermissionAssertion>, ArmorGatewayError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Ok(self.assertions.clone())
        }
    }

    fn assertion(capability: &str, expires_at: DateTime<Utc>) -> PermissionAssertion {
        PermissionAssertion {
            consultant_id: "consultant-1".to_owned(),
            capability: capability.to_owned(),
            scope: "default".to_owned(),
            expires_at,
        }
    }

    #[tokio::test]
    async fn is_permitted_true_for_a_granted_capability() {
        let gateway = Arc::new(MockArmorGateway::new(vec![assertion("sales:read", Utc::now() + ChronoDuration::minutes(5))]));
        let cache = PermissionCache::new(gateway.clone());

        assert!(cache.is_permitted("consultant-1", "sales:read").await);
    }

    #[tokio::test]
    async fn is_permitted_false_for_a_capability_with_no_assertion() {
        let gateway = Arc::new(MockArmorGateway::new(vec![assertion("sales:read", Utc::now() + ChronoDuration::minutes(5))]));
        let cache = PermissionCache::new(gateway.clone());

        assert!(!cache.is_permitted("consultant-1", "sales:write").await);
    }

    #[tokio::test]
    async fn second_lookup_within_ttl_does_not_refetch_the_gateway() {
        let gateway = Arc::new(MockArmorGateway::new(vec![assertion("sales:read", Utc::now() + ChronoDuration::minutes(5))]));
        let cache = PermissionCache::new(gateway.clone());

        assert!(cache.is_permitted("consultant-1", "sales:read").await);
        assert!(cache.is_permitted("consultant-1", "sales:read").await);

        assert_eq!(gateway.calls(), 1, "second lookup within TTL must be served from cache");
    }

    #[tokio::test]
    async fn lookup_after_ttl_expiry_refetches_the_gateway() {
        // Already-past expires_at: the very first cached entry is stale on
        // arrival, so a second lookup must trigger a fresh fetch.
        let gateway = Arc::new(MockArmorGateway::new(vec![assertion("sales:read", Utc::now() - ChronoDuration::seconds(1))]));
        let cache = PermissionCache::new(gateway.clone());

        assert!(cache.is_permitted("consultant-1", "sales:read").await);
        assert!(cache.is_permitted("consultant-1", "sales:read").await);

        assert_eq!(gateway.calls(), 2, "an expired entry must trigger a fresh fetch");
    }

    #[tokio::test]
    async fn invalidate_forces_a_refetch_on_the_next_lookup() {
        let gateway = Arc::new(MockArmorGateway::new(vec![assertion("sales:read", Utc::now() + ChronoDuration::minutes(5))]));
        let cache = PermissionCache::new(gateway.clone());

        assert!(cache.is_permitted("consultant-1", "sales:read").await);
        cache.invalidate("consultant-1").await;
        assert!(cache.is_permitted("consultant-1", "sales:read").await);

        assert_eq!(gateway.calls(), 2, "invalidate must force the next lookup to refetch");
    }

    #[tokio::test]
    async fn assertions_for_returns_the_full_cached_set() {
        let gateway = Arc::new(MockArmorGateway::new(vec![
            assertion("sales:read", Utc::now() + ChronoDuration::minutes(5)),
            assertion("delivery:read", Utc::now() + ChronoDuration::minutes(5)),
        ]));
        let cache = PermissionCache::new(gateway.clone());

        let assertions = cache.assertions_for("consultant-1").await;

        assert_eq!(assertions.len(), 2);
        assert!(assertions.iter().any(|a| a.capability == "sales:read"));
        assert!(assertions.iter().any(|a| a.capability == "delivery:read"));
    }

    #[tokio::test]
    async fn assertions_for_shares_the_cache_with_is_permitted() {
        let gateway = Arc::new(MockArmorGateway::new(vec![assertion("sales:read", Utc::now() + ChronoDuration::minutes(5))]));
        let cache = PermissionCache::new(gateway.clone());

        assert!(cache.is_permitted("consultant-1", "sales:read").await);
        cache.assertions_for("consultant-1").await;

        assert_eq!(gateway.calls(), 1, "assertions_for must reuse is_permitted's cached entry");
    }

    #[tokio::test]
    async fn assertions_for_returns_empty_when_the_gateway_fetch_fails() {
        struct FailingGateway;

        #[async_trait::async_trait]
        impl ArmorGateway for FailingGateway {
            async fn fetch_assertions(
                &self,
                _consultant_id: &str,
                _credential: &str,
            ) -> Result<Vec<PermissionAssertion>, ArmorGatewayError> {
                Err(ArmorGatewayError::Transport(nexus_client::NexusTransportError::CircuitOpen))
            }
        }

        let cache = PermissionCache::new(Arc::new(FailingGateway));

        assert!(cache.assertions_for("consultant-1").await.is_empty());
    }

    #[tokio::test]
    async fn is_permitted_false_when_the_gateway_fetch_fails() {
        struct FailingGateway;

        #[async_trait::async_trait]
        impl ArmorGateway for FailingGateway {
            async fn fetch_assertions(
                &self,
                _consultant_id: &str,
                _credential: &str,
            ) -> Result<Vec<PermissionAssertion>, ArmorGatewayError> {
                Err(ArmorGatewayError::Transport(nexus_client::NexusTransportError::CircuitOpen))
            }
        }

        let cache = PermissionCache::new(Arc::new(FailingGateway));

        assert!(!cache.is_permitted("consultant-1", "sales:read").await);
    }
}
