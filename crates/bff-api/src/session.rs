//! Session state + session-lookup middleware (ADR-008, PROMPT-11).
//!
//! Holds the shared [`AppState`] used across `bff-api`'s router (DB pool,
//! session provider, Prometheus handle), the session-cookie name constant,
//! the `POST /api/login/dev` and `GET /api/session` handlers, and the
//! session-lookup middleware ([`require_session`]) applied uniformly to the
//! `/api/*` protected sub-router ([`protected_router`]) so future protected
//! routes (U15/U20+) only need a `.route(...)` call added there, not
//! repeated auth logic per-handler.
//!
//! **Dev-only login, documented rather than feature-gated**:
//! `POST /api/login/dev` calls `DevStubSessionProvider::create_dev_session`
//! directly. There is no real Armor-backed provider yet (ADR-008 "Interim
//! dev-stub"), so this route is registered unconditionally rather than
//! behind a `bff-api`-level Cargo feature — the actual safety valve is
//! `DevStubSessionProvider::new`'s runtime panic outside a `dev`
//! environment (`crates/auth/src/dev_stub.rs`), which already makes this
//! route a no-op (a panic at startup, in fact) in any non-dev deployment.
//! Once a real provider lands, this route and [`AppState::dev_session_provider`]
//! should become conditional on which provider `main.rs` constructs (see the
//! `TODO` on the `auth` dependency in `Cargo.toml`).

use std::sync::Arc;

use auth::dev_stub::DevStubSessionProvider;
use auth::{Session, SessionProvider};
use axum::extract::{FromRef, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{json, Value};
use uuid::Uuid;

/// Name of the cookie carrying the opaque session id (ADR-008).
pub const SESSION_COOKIE_NAME: &str = "cognitum_session";

/// Shared application state, constructed once at startup in `main` and
/// threaded through the whole router via `Router::with_state`.
#[derive(Clone)]
pub struct AppState {
    /// ADR-010 Postgres pool (PROMPT-09). Not read by any handler yet.
    pub db_pool: persistence::Pool,
    /// The session provider used for session *lookups* ([`require_session`]
    /// below), typed as `dyn SessionProvider` so this field doesn't need to
    /// change shape once a real, Armor-backed provider replaces the
    /// dev-stub.
    pub session_provider: Arc<dyn SessionProvider>,
    /// The concrete dev-stub provider — the same underlying instance as
    /// `session_provider` (both `Arc`s point at it) — kept separately so
    /// `POST /api/login/dev` can call its stub-specific
    /// `create_dev_session`, which isn't (and shouldn't be) part of the
    /// general `SessionProvider` trait.
    pub dev_session_provider: Arc<DevStubSessionProvider>,
    /// Whether to set the `Secure` cookie flag (ADR-008: `Secure` in
    /// non-local environments), driven by `Config::is_dev()` at startup.
    pub secure_cookies: bool,
    pub prometheus_handle: PrometheusHandle,
    /// Per-consultant cache of Armor `PermissionAssertion`s (ADR-009,
    /// PROMPT-15) — see `crate::permissions` module docs for the caching/
    /// TTL semantics and keying decision. `Arc`-wrapped (rather than
    /// cloned per-request like the other fields, which are already cheap
    /// `Arc`/`Copy`) since `PermissionCache` holds its own interior
    /// `RwLock`-guarded state that must be shared, not duplicated, across
    /// every handler invocation.
    pub permission_cache: Arc<crate::permissions::PermissionCache>,
}

impl FromRef<AppState> for PrometheusHandle {
    fn from_ref(state: &AppState) -> Self {
        state.prometheus_handle.clone()
    }
}

/// `POST /api/login/dev`: issues a fixed dev-consultant session
/// (`DevStubSessionProvider::create_dev_session`) and sets it as the
/// session cookie: `HttpOnly`, `SameSite=Strict`, and `Secure` when
/// `AppState::secure_cookies` (i.e. `!Config::is_dev()`).
pub async fn login_dev(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<(CookieJar, Json<Value>), Response> {
    let session = state.dev_session_provider.create_dev_session().await.map_err(|err| {
        tracing::error!(error = %err, "dev session creation failed");
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "session creation failed" })))
            .into_response()
    })?;

    let cookie = Cookie::build((SESSION_COOKIE_NAME, session.session_id.to_string()))
        .http_only(true)
        .secure(state.secure_cookies)
        .same_site(SameSite::Strict)
        .path("/")
        .build();

    Ok((jar.add(cookie), Json(json!({ "consultant_id": session.consultant_id }))))
}

/// Response body for `GET /api/session`.
///
/// `permission_assertions` (ADR-009, PROMPT-19) is the consultant's current
/// Armor-granted [`PermissionAssertion`] set, sourced from
/// [`crate::permissions::PermissionCache`]. **This field is a UX/rendering
/// signal only, never an enforcement mechanism** — the frontend uses it to
/// conditionally render nav items etc. (ADR-009 layer 2), but every real
/// mutation is independently re-authorized server-side by the owning
/// capability (ADR-009 layer 3). A consultant whose client omits or
/// misrepresents this field gains nothing: the BFF's own `RequirePermission`
/// checks (layer 1) and the downstream capability's checks (layer 3) are
/// what actually gate access, not what this response does or doesn't list.
#[derive(serde::Serialize)]
pub struct SessionResponse {
    pub consultant_id: String,
    pub permission_assertions: Vec<nexus_client::PermissionAssertion>,
}

/// `GET /api/session`: returns the authenticated consultant's identity plus
/// their current Permission Assertions (ADR-009, PROMPT-19). Only reachable
/// once [`require_session`] has attached a `Session` to the request
/// extensions — unauthenticated requests never reach this handler (see
/// [`protected_router`]).
pub async fn get_session_handler(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
) -> Json<SessionResponse> {
    let permission_assertions = state.permission_cache.assertions_for(&session.consultant_id).await;
    Json(SessionResponse { consultant_id: session.consultant_id, permission_assertions })
}

/// Session-lookup middleware (ADR-008): reads the session cookie, looks it
/// up via `SessionProvider::get_session`, and either attaches the resolved
/// `Session` to the request extensions and continues, or short-circuits
/// with `401 Unauthorized` if the cookie is missing, the session id is
/// malformed, the session is unknown, or it has expired (`get_session`
/// itself filters expired sessions, per the dev-stub's implementation).
pub async fn require_session(
    State(state): State<AppState>,
    jar: CookieJar,
    mut request: Request,
    next: Next,
) -> Response {
    match resolve_session(&jar, state.session_provider.as_ref()).await {
        Some(session) => {
            request.extensions_mut().insert(session);
            next.run(request).await
        }
        None => unauthorized(),
    }
}

/// Extracts and resolves the session cookie, if any. Returns `None` for a
/// missing cookie, a malformed session id, an unknown/expired session, or a
/// provider-level lookup failure (logged, then treated as unauthenticated
/// rather than a 5xx — a lookup failure should not look distinguishable
/// from "not logged in" to the caller).
async fn resolve_session(jar: &CookieJar, provider: &dyn SessionProvider) -> Option<Session> {
    let session_id =
        jar.get(SESSION_COOKIE_NAME).and_then(|cookie| Uuid::parse_str(cookie.value()).ok())?;

    match provider.get_session(session_id).await {
        Ok(session) => session,
        Err(err) => {
            tracing::error!(error = %err, "session lookup failed");
            None
        }
    }
}

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({ "error": "unauthorized" }))).into_response()
}

/// Builds the `/api/*` protected sub-router: every route added here has
/// [`require_session`] applied uniformly, so future protected routes
/// (U15/U20+) just need a `.route(...)` call added to this function rather
/// than repeating auth logic per-handler.
pub fn protected_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/session", get(get_session_handler))
        .layer(axum::middleware::from_fn_with_state(state, require_session))
}
