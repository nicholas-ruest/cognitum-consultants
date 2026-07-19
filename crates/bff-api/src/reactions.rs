//! `POST /api/reactions/:reaction_handler` — nexus's inbound push delivery
//! route (ADR-018), replacing the old (guessed, never-working) Nexus→BFF
//! polling loop.
//!
//! # Path convention
//! Mounted under this repo's existing `/api` namespace (ADR-006), at
//! `/api/reactions/:reaction_handler` — **nexus-server's own `repos.json`
//! entry for `cognitum-consultants` must set `base_url` to this service's
//! `/api` root** (e.g. `https://consultants.cognitum.one/api`) for the
//! `/reactions/` segment nexus's binary itself is confirmed to call
//! (ADR-018's investigation notes) to land here. `reaction_handler` is a
//! path parameter, not a fixed route per handler — one route serves every
//! `{event_type, reaction_handler}` pair nexus's `consumers.json`/
//! `reactions.json` eventually register for this repo (ADR-018), since
//! there is exactly one thing to do with any of them: verify the caller,
//! parse the envelope, ingest it.
//!
//! # Unauthenticated by session cookie, authenticated by caller identity
//! Unlike every other route in this crate, this one is **not** behind
//! [`crate::session::require_session`] — nexus is not a browser holding a
//! consultant's session cookie, it is a peer service. Deliberately *not*
//! merged through [`crate::session::protected_router`]/any router that
//! applies that middleware; this router applies no session layer at all,
//! and [`receive_reaction`] does its own caller verification instead (see
//! below). Do not add this route to a session-gated router — nexus has no
//! session to present.
//!
//! # Caller verification
//! [`AppState::google_identity_verifier`] checks the `Authorization: Bearer
//! <token>` header against nexus-server's real Google-signed identity —
//! same trust model (and, deliberately, the *reverse direction* of) the
//! outbound identity-token check `nexus_client::reqwest_transport` already
//! performs when this repo calls *out* to nexus (ADR-029). A missing
//! header, malformed token, wrong audience, or caller whose `email` doesn't
//! match the configured expectation are all rejected `401` before the body
//! is even parsed — see [`GoogleIdentityTokenError`] for the exact failure
//! modes, all collapsed to `401` here since none of them are actionable
//! information to hand back to an unverified caller.
//!
//! # Idempotency
//! A push can be retried by nexus (timeout, transient 5xx). No new dedup
//! mechanism here — relies entirely on `bff_core::event_ingestion::ingest_events`'s
//! existing `(origin_capability, origin_event_id)` unique-constraint save
//! (ADR-010, PROMPT-29), same as the old polling loop did. A redelivered
//! event is reported back as a duplicate, not an error, and still gets a
//! `200`.
//!
//! # Two entry points, deliberately split (mirrors the old `poll_once`/
//! `run_polling_loop` split)
//! [`receive_reaction`] (the actual Axum handler) does caller verification,
//! then delegates to [`ingest_reaction_body`] for envelope parsing +
//! ingestion. The split exists so tests can exercise the parse/ingest/dedup
//! logic directly — the part with real behavior to prove — without needing
//! a genuine Google-signed token, which no test in this crate can mint. Real
//! end-to-end caller verification is [`auth::google_identity_token`]'s own
//! concern, unit-tested there in isolation.

use auth::google_identity_token::GoogleIdentityTokenError;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use bff_core::{filter_conservative_legal_events, ingest_events, IngestionOutcome};
use serde_json::json;

use crate::event_ingestion::{envelope_into_received, EventEnvelope};
use crate::session::AppState;

/// Audience this endpoint expects on every inbound identity token (ADR-018)
/// — this repo's own choice, symmetric to how ADR-029 has this repo request
/// `aud: nexus-api` when calling *out* to nexus
/// (`nexus_client::reqwest_transport::NEXUS_IDENTITY_TOKEN_AUDIENCE`).
/// Whoever registers `cognitum-consultants` in nexus's `repos.json` must
/// mint its outbound identity token with `?audience=cognitum-consultants-reactions`
/// for this check to ever succeed — tracked as part of ADR-018's external
/// nexus-side dependency, not something this repo can self-serve.
pub const REACTION_TOKEN_AUDIENCE: &str = "cognitum-consultants-reactions";

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// Extracts the bearer token from an `Authorization` header, if present and
/// well-formed (`Bearer <token>`, case-sensitive scheme per RFC 6750).
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?.strip_prefix("Bearer ")
}

/// Parses `body` as a nexus `EventEnvelope` and ingests it via the same
/// `bff_core::event_ingestion` pipeline the old polling loop used —
/// everything [`receive_reaction`] does *after* caller verification passes.
/// Separated out so it's testable without a real bearer token (see the
/// module docs).
async fn ingest_reaction_body(state: &AppState, reaction_handler: &str, body: &[u8]) -> Response {
    let envelope: EventEnvelope = match serde_json::from_slice(body) {
        Ok(envelope) => envelope,
        Err(err) => {
            tracing::warn!(error = %err, reaction_handler, "rejected an inbound reaction call: malformed EventEnvelope body");
            return error_response(StatusCode::BAD_REQUEST, format!("malformed EventEnvelope body: {err}"));
        }
    };

    let event = envelope_into_received(envelope);
    let events = filter_conservative_legal_events(vec![event], state.workflow_session_repository.as_ref()).await;
    let ingestion = ingest_events(
        events,
        state.notification_repository.as_ref(),
        state.action_queue_repository.as_ref(),
        state.event_notify_publisher.as_ref(),
    )
    .await;

    // `filter_conservative_legal_events` may have dropped the event
    // entirely (a `LegalClauseUpdated` with no in-progress correlation, see
    // that function's doc comment) — `outcomes` is then empty, which is a
    // legitimate "nothing to do" outcome, not a failure.
    match ingestion.outcomes.first() {
        Some(IngestionOutcome::Rejected { reason, .. }) => {
            tracing::warn!(reaction_handler, reason, "reaction event rejected during ingestion");
            (StatusCode::OK, Json(json!({ "status": "rejected", "reason": reason }))).into_response()
        }
        Some(_) | None => (StatusCode::OK, Json(json!({ "status": "accepted" }))).into_response(),
    }
}

/// `POST /api/reactions/:reaction_handler`: verifies the caller, then
/// delegates to [`ingest_reaction_body`].
///
/// `reaction_handler` (the path parameter) is accepted but not yet branched
/// on — every currently-possible caller is nexus itself delivering the one
/// wire shape this repo knows how to ingest, and nexus's `consumers.json`
/// has zero entries for this repo as of ADR-018 (nothing to validate the
/// handler name against yet). It is still extracted (rather than an
/// untyped catch-all route) so a future per-handler dispatch has a typed
/// value ready, and so it can be logged for observability now.
pub async fn receive_reaction(
    State(state): State<AppState>,
    Path(reaction_handler): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let Some(token) = bearer_token(&headers) else {
        return error_response(StatusCode::UNAUTHORIZED, "missing bearer token");
    };

    if let Err(err) = state.google_identity_verifier.verify(token).await {
        tracing::warn!(error = %err, reaction_handler, "rejected an inbound reaction call: caller verification failed");
        let message = match err {
            GoogleIdentityTokenError::NoExpectedCallerConfigured => "reactions endpoint is not configured",
            _ => "caller verification failed",
        };
        return error_response(StatusCode::UNAUTHORIZED, message);
    }

    ingest_reaction_body(&state, &reaction_handler, &body).await
}

/// Builds the reactions sub-router. Deliberately **no**
/// [`crate::session::require_session`] layer — see the module docs' "not
/// authenticated by session cookie" section.
pub fn reactions_router(_state: AppState) -> Router<AppState> {
    Router::new().route("/reactions/{reaction_handler}", post(receive_reaction))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use auth::dev_stub::DevStubSessionProvider;
    use auth::google_identity_token::GoogleIdentityTokenVerifier;
    use axum::body::Body;
    use axum::http::Request;
    use bff_core::EventBus;
    use persistence::{PgActionQueueRepository, PgNotificationRepository, PgWorkflowSessionRepository};
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;
    use tower::ServiceExt;

    use super::*;
    use crate::permissions::PermissionCache;

    // --- Unused-gateway stubs, same "AppState needs the field regardless"
    // rationale `dashboard`'s own test module documents on its first one.
    // Reactions tests never call any capability gateway. ---

    struct UnusedArmorGateway;
    #[async_trait::async_trait]
    impl nexus_client::ArmorGateway for UnusedArmorGateway {
        async fn fetch_assertions(
            &self,
            _consultant_id: &str,
            _credential: &str,
        ) -> Result<Vec<nexus_client::PermissionAssertion>, nexus_client::ArmorGatewayError> {
            unimplemented!("reactions tests never call the armor gateway")
        }
    }

    struct UnusedSalesGateway;
    #[async_trait::async_trait]
    impl nexus_client::SalesGateway for UnusedSalesGateway {
        async fn check_account_claim(
            &self,
            _company_name: &str,
            _consultant_id: &str,
        ) -> Result<nexus_client::AccountClaimResult, nexus_client::SalesGatewayError> {
            unimplemented!("reactions tests never call the sales gateway")
        }
        async fn request_collaboration(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _message: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("reactions tests never call the sales gateway")
        }
        async fn submit_referral(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _notes: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("reactions tests never call the sales gateway")
        }
    }

    struct UnusedCommitGateway;
    #[async_trait::async_trait]
    impl nexus_client::CommitGateway for UnusedCommitGateway {
        async fn create_proposal(
            &self,
            _origin_reference: &str,
            _consultant_id: &str,
        ) -> Result<nexus_client::ProposalSummary, nexus_client::CommitGatewayError> {
            unimplemented!("reactions tests never call the commit gateway")
        }
        async fn list_proposals(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<nexus_client::ProposalSummary>, nexus_client::CommitGatewayError> {
            unimplemented!("reactions tests never call the commit gateway")
        }
        async fn request_proposal_action(
            &self,
            _proposal_id: &str,
            _action: &str,
        ) -> Result<(), nexus_client::CommitGatewayError> {
            unimplemented!("reactions tests never call the commit gateway")
        }
    }

    struct UnusedEduGateway;
    #[async_trait::async_trait]
    impl nexus_client::EduGateway for UnusedEduGateway {
        async fn request_learning_catalog(
            &self,
            _consultant_id: &str,
            _filters: Option<&[String]>,
        ) -> Result<Vec<nexus_client::LearningSnapshot>, nexus_client::EduGatewayError> {
            unimplemented!("reactions tests never call the edu gateway")
        }
    }

    struct UnusedCapacityGateway;
    #[async_trait::async_trait]
    impl nexus_client::CapacityGateway for UnusedCapacityGateway {
        async fn update_own_profile(
            &self,
            _consultant_id: &str,
            _profile_fields: nexus_client::ConsultantProfileIntake,
        ) -> Result<nexus_client::ProfileUpdateResult, nexus_client::CapacityGatewayError> {
            unimplemented!("reactions tests never call the capacity gateway")
        }
        async fn get_own_profile(
            &self,
            _consultant_id: &str,
        ) -> Result<nexus_client::ConsultantProfileIntake, nexus_client::CapacityGatewayError> {
            unimplemented!("reactions tests never call the capacity gateway")
        }
    }

    struct UnusedCustomerGateway;
    #[async_trait::async_trait]
    impl nexus_client::CustomerGateway for UnusedCustomerGateway {
        async fn request_assigned_customer_context(
            &self,
            _consultant_id: &str,
            _customer_id: Option<&str>,
        ) -> Result<Vec<nexus_client::CustomerContextCard>, nexus_client::CustomerGatewayError> {
            unimplemented!("reactions tests never call the customer gateway")
        }
    }

    struct UnusedExecutionGateway;
    #[async_trait::async_trait]
    impl nexus_client::ExecutionGateway for UnusedExecutionGateway {
        async fn request_assigned_engagements(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<nexus_client::EngagementSnapshot>, nexus_client::ExecutionGatewayError> {
            unimplemented!("reactions tests never call the execution gateway")
        }
        async fn confirm_task_completion(
            &self,
            _task_id: &str,
            _consultant_id: &str,
        ) -> Result<(), nexus_client::ExecutionGatewayError> {
            unimplemented!("reactions tests never call the execution gateway")
        }
    }

    struct UnusedProductsGateway;
    #[async_trait::async_trait]
    impl nexus_client::ProductsGateway for UnusedProductsGateway {
        async fn request_product_catalog(
            &self,
            _filters: Option<&[String]>,
        ) -> Result<Vec<nexus_client::ProductReferenceCard>, nexus_client::ProductsGatewayError> {
            unimplemented!("reactions tests never call the products gateway")
        }
    }

    struct UnusedLandscapeGateway;
    #[async_trait::async_trait]
    impl nexus_client::LandscapeGateway for UnusedLandscapeGateway {
        async fn request_intelligence_digest(
            &self,
        ) -> Result<Vec<nexus_client::IntelligenceDigestItem>, nexus_client::LandscapeGatewayError> {
            unimplemented!("reactions tests never call the landscape gateway")
        }
        async fn submit_field_observation(
            &self,
            _submission: nexus_client::FieldObservationSubmission,
        ) -> Result<(), nexus_client::LandscapeGatewayError> {
            unimplemented!("reactions tests never call the landscape gateway")
        }
    }

    struct UnusedLegalGateway;
    #[async_trait::async_trait]
    impl nexus_client::LegalGateway for UnusedLegalGateway {
        async fn request_approved_clauses(
            &self,
            _context: nexus_client::ClauseContext<'_>,
        ) -> Result<Vec<nexus_client::ApprovedLegalSnippet>, nexus_client::LegalGatewayError> {
            unimplemented!("reactions tests never call the legal gateway")
        }
    }

    async fn migrated_pool() -> (persistence::Pool, testcontainers_modules::testcontainers::ContainerAsync<Postgres>) {
        let container = Postgres::default().start().await.expect("failed to start postgres container");
        let host = container.get_host().await.expect("failed to resolve container host");
        let port = container.get_host_port_ipv4(5432).await.expect("failed to resolve container port");
        let database_url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

        let pool = persistence::create_pool(&database_url).await.expect("create_pool failed to connect");
        sqlx::migrate!("../persistence/migrations").run(&pool).await.expect("migration failed to run");

        (pool, container)
    }

    fn dev_config() -> config::Config {
        config::Config {
            database_url: "postgres://localhost:5432/test".to_owned(),
            port: 3000,
            log_level: "info".to_owned(),
            nexus_endpoint_url: "http://localhost:8080".to_owned(),
            environment: config::DEV_ENVIRONMENT.to_owned(),
            static_dir: None,
            firebase_project_id: None,
            nexus_caller_service_account_email: None,
        }
    }

    /// A verifier that always rejects (no expected caller configured) — no
    /// test in this module has a real Google-signed token to present.
    fn never_configured_verifier() -> Arc<GoogleIdentityTokenVerifier> {
        Arc::new(GoogleIdentityTokenVerifier::new(REACTION_TOKEN_AUDIENCE.to_owned(), None))
    }

    async fn test_state() -> (AppState, testcontainers_modules::testcontainers::ContainerAsync<Postgres>) {
        let (pool, container) = migrated_pool().await;

        let dev_session_provider = Arc::new(DevStubSessionProvider::new(&dev_config()));
        let session_provider: Arc<dyn auth::SessionProvider> = dev_session_provider.clone();

        let armor_gateway: Arc<dyn nexus_client::ArmorGateway> = Arc::new(UnusedArmorGateway);
        let permission_cache = Arc::new(PermissionCache::new(armor_gateway));

        let notification_repository: Arc<dyn bff_core::NotificationRepository> =
            Arc::new(PgNotificationRepository::new(pool.clone()));
        let action_queue_repository: Arc<dyn bff_core::ActionQueueRepository> =
            Arc::new(PgActionQueueRepository::new(pool.clone()));
        let workflow_session_repository: Arc<dyn bff_core::WorkflowSessionRepository> =
            Arc::new(PgWorkflowSessionRepository::new(pool.clone()));

        let state = AppState {
            db_pool: pool.clone(),
            session_provider,
            dev_session_provider: Some(dev_session_provider),
            firebase_session_provider: None,
            secure_cookies: false,
            prometheus_handle: crate::metrics::shared_test_handle(),
            permission_cache,
            dashboard_repository: Arc::new(persistence::PgDashboardConfigurationRepository::new(pool.clone())),
            sales_query_gateway: Arc::new(UnusedSalesGateway),
            sales_command_gateway: Arc::new(UnusedSalesGateway),
            commit_query_gateway: Arc::new(UnusedCommitGateway),
            commit_command_gateway: Arc::new(UnusedCommitGateway),
            edu_gateway: Arc::new(UnusedEduGateway),
            capacity_query_gateway: Arc::new(UnusedCapacityGateway),
            capacity_command_gateway: Arc::new(UnusedCapacityGateway),
            customer_gateway: Arc::new(UnusedCustomerGateway),
            execution_query_gateway: Arc::new(UnusedExecutionGateway),
            execution_command_gateway: Arc::new(UnusedExecutionGateway),
            products_gateway: Arc::new(UnusedProductsGateway),
            landscape_query_gateway: Arc::new(UnusedLandscapeGateway),
            landscape_command_gateway: Arc::new(UnusedLandscapeGateway),
            legal_gateway: Arc::new(UnusedLegalGateway),
            workflow_session_repository,
            notification_repository,
            action_queue_repository,
            event_bus: Arc::new(EventBus::default()),
            event_notify_publisher: Arc::new(EventBus::default()),
            google_identity_verifier: never_configured_verifier(),
            prospect_repository: Arc::new(persistence::PgProspectRepository::new(pool.clone())),
            action_item_repository: Arc::new(persistence::PgConsultantActionItemRepository::new(pool.clone())),
        };

        (state, container)
    }

    fn router_for(state: AppState) -> Router<()> {
        Router::new().merge(reactions_router(state.clone())).with_state(state)
    }

    #[tokio::test]
    async fn rejects_a_call_with_no_authorization_header() {
        let (state, _container) = test_state().await;

        let response = router_for(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/reactions/some.handler")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rejects_a_call_with_no_expected_caller_configured_even_with_a_bearer_header() {
        let (state, _container) = test_state().await;

        let response = router_for(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/reactions/some.handler")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer not-a-real-token")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    // --- `ingest_reaction_body` exercised directly (post-auth logic; see
    // the module docs for why this is split from `receive_reaction`) ---

    /// The same real `EventEnvelope` wire shape
    /// `crate::event_ingestion`'s own tests use, including the fields the
    /// mapping ignores.
    fn event_envelope_body(event_id: &str) -> serde_json::Value {
        serde_json::json!({
            "event_id": event_id,
            "event_type": "collaboration_request_acknowledged",
            "event_version": {"major": 1, "minor": 0, "patch": 0},
            "occurred_at": "2026-01-01T00:00:00Z",
            "producer_repo": "sales",
            "aggregate_id": "collab-1",
            "aggregate_type": "collaboration_request",
            "organization_id": "org-1",
            "actor": {"user_id": "sales-user-9", "service_account": null, "role": "sales-rep"},
            "correlation_id": "corr-1",
            "payload": {
                "summary": "Sales acknowledged your collaboration request.",
                "deep_link": "https://app.example.com/sales/collab/1",
                "consultant_id": "consultant-1"
            },
            "metadata": {}
        })
    }

    #[tokio::test]
    async fn ingests_a_valid_pushed_event_and_saves_it() {
        let (state, _container) = test_state().await;
        let body = event_envelope_body("cra-push-1").to_string();

        let response = ingest_reaction_body(&state, "some.handler", body.as_bytes()).await;

        assert_eq!(response.status(), StatusCode::OK);

        let entries = state.action_queue_repository.find_by_consultant_id("consultant-1").await.expect("find failed");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].origin_event_id(), "cra-push-1");
    }

    /// The headline idempotency proof for the push path (mirrors the old
    /// polling loop's own equivalent test): the *same* event pushed twice
    /// (simulating a nexus retry) results in exactly one saved row, not two.
    #[tokio::test]
    async fn redelivering_the_same_event_is_idempotent() {
        let (state, _container) = test_state().await;
        let body = event_envelope_body("cra-push-2").to_string();

        let first = ingest_reaction_body(&state, "some.handler", body.as_bytes()).await;
        let second = ingest_reaction_body(&state, "some.handler", body.as_bytes()).await;

        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(second.status(), StatusCode::OK);

        let entries = state.action_queue_repository.find_by_consultant_id("consultant-1").await.expect("find failed");
        assert_eq!(entries.len(), 1, "a redelivered event must not create a second row");
    }

    #[tokio::test]
    async fn rejects_a_malformed_body_as_bad_request() {
        let (state, _container) = test_state().await;

        let response = ingest_reaction_body(&state, "some.handler", b"not json").await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
