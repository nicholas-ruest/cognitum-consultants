//! Edu ACL gateway (ADR-007, ADR-016, PROMPT-35).
//!
//! Edu owns each course/certification/training-requirement's own status;
//! this repo never re-derives or overrides a `LearningSnapshot`'s
//! `progress_status`/`certification_status`, and never stores assessment
//! content or certification criteria itself
//! (`../../.plans/ddd/anti-corruption-layers.md` §3). [`EduGateway`] is a
//! thin translation boundary over Edu's single outbound call — a read-only
//! catalog query, mirroring [`crate::commit::CommitGateway`]'s
//! `list_proposals` shape (a query in DDD terms, no side effect on Edu).
//!
//! # Read-mostly: no side-effecting command, no two-gateway split
//! Unlike Sales/Commit, Edu's `anti-corruption-layers.md` §3 entry lists no
//! outbound command with a side effect — only
//! `RequestLearningCatalogQuery`. There is therefore nothing here for the
//! "two `Nexus<Capability>Gateway` instances, one per retry-safety profile"
//! convention (`crate::sales`/`crate::commit` module docs) to split: a
//! single [`NexusEduGateway`] instance, constructed once over a
//! `RetryingTransport`-wrapped stack, safely serves
//! [`EduGateway::request_learning_catalog`] — the only method this trait
//! has.
//!
//! # Request path: provisional, matching Commit's `.../v1/...` convention
//! Nexus's real Edu contract is not finalized. This gateway assumes:
//! - `GET edu/v1/catalog?consultant_id=...` (repeated `filter=...` query
//!   params for `filters`, if any) — response an envelope
//!   `{"snapshots": [LearningSnapshot, ...]}`, matching
//!   [`crate::commit::NexusCommitGateway`]'s `ProposalsEnvelope`
//!   convention (see that module's doc comment for why an envelope was
//!   chosen over a bare array).
//!
//! Update this once Nexus's actual Edu contract is known.
//!
//! # Timeout budget choice (ADR-016, PROMPT-35's explicit "longer timeout")
//! `request_learning_catalog` is a read-mostly, background/page-load call
//! with no synchronous UI-blocking counterpart sharing this gateway (unlike
//! Sales' `check_account_claim`) — it uses
//! [`crate::timeout::DEFAULT_EXTENDED_READ_TIMEOUT`], not the plain
//! [`crate::timeout::DEFAULT_READ_TIMEOUT`] `Commit`'s `list_proposals`
//! uses, per PROMPT-35's explicit "apply a longer timeout (read-mostly, per
//! ADR-016)" instruction — see that constant's doc comment for the full
//! rationale. It MAY be retried ([`crate::retry::RetryingTransport`]
//! -wrapped) since it has no side effect.
//!
//! # Transport-stack-assembly convention
//! Same convention as [`crate::sales::NexusSalesGateway`]/
//! [`crate::commit::NexusCommitGateway`]: [`NexusEduGateway::new`] takes an
//! already-fully-decorated `Arc<dyn NexusTransport>` and does not assemble
//! the ADR-016 timeout/retry/circuit-breaker stack itself.

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Method;
use reqwest::header::HeaderMap;

use crate::transport::{NexusRequest, NexusTransport, NexusTransportError};

/// Edu's read-mostly Learning Snapshot projection
/// (`anti-corruption-layers.md` §3): one entry per course, carrying that
/// course's own progress and certification status. This repo never stores
/// assessment content or certification criteria — only this snapshot.
///
/// `Serialize` (alongside `Deserialize`, used to decode Edu's response) is
/// derived so `bff-api` can relay this same shape verbatim to the frontend,
/// matching `ProposalSummary`'s/`AccountClaimResult`'s "no BFF re-shaping"
/// convention.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LearningSnapshot {
    pub course_id: String,
    pub title: String,
    pub progress_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certification_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deep_link: Option<String>,
}

/// Envelope this gateway expects `GET edu/v1/catalog`'s response body to
/// match. See the module docs for why an envelope (vs. a bare array) was
/// chosen (mirrors [`crate::commit::NexusCommitGateway`]'s
/// `ProposalsEnvelope` rationale).
#[derive(Debug, serde::Deserialize)]
struct LearningCatalogEnvelope {
    snapshots: Vec<LearningSnapshot>,
}

#[derive(Debug, thiserror::Error)]
pub enum EduGatewayError {
    #[error(transparent)]
    Transport(#[from] NexusTransportError),
    #[error("Edu returned a non-success status {status}")]
    UnexpectedStatus { status: reqwest::StatusCode },
    #[error("Edu returned a response body that did not match the expected shape: {0}")]
    UnexpectedResponseShape(#[source] serde_json::Error),
}

/// ACL over Edu's read-only learning-catalog capability. No re-adjudication
/// of Edu's own `progress_status`/`certification_status` happens on this
/// trait — see the module docs.
#[async_trait]
pub trait EduGateway: Send + Sync {
    /// Fetches `consultant_id`'s current Learning Snapshot set, per
    /// `anti-corruption-layers.md` §3's `RequestLearningCatalogQuery`.
    /// `filters`, if non-empty, is passed through to Edu untouched — this
    /// repo has no opinion on what a valid filter value is (Edu owns that
    /// vocabulary).
    ///
    /// A **query** in DDD terms — reading Edu's current catalog state has
    /// no side effect, so retrying it is safe/idempotent. See
    /// [`NexusEduGateway`]'s doc comment for the transport requirement this
    /// method needs from its caller.
    async fn request_learning_catalog(
        &self,
        consultant_id: &str,
        filters: Option<&[String]>,
    ) -> Result<Vec<LearningSnapshot>, EduGatewayError>;
}

/// [`EduGateway`] implementation backed by a [`NexusTransport`]. See the
/// module docs for the required transport decoration.
pub struct NexusEduGateway {
    transport: Arc<dyn NexusTransport>,
}

impl NexusEduGateway {
    /// `transport` is expected to already be decorated per the ADR-016
    /// extended-read-call convention (see module docs) — this constructor
    /// does not assemble timeout/retry/circuit-breaker layers itself.
    pub fn new(transport: Arc<dyn NexusTransport>) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl EduGateway for NexusEduGateway {
    async fn request_learning_catalog(
        &self,
        consultant_id: &str,
        filters: Option<&[String]>,
    ) -> Result<Vec<LearningSnapshot>, EduGatewayError> {
        let path = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.append_pair("consultant_id", consultant_id);
            for filter in filters.unwrap_or_default() {
                query.append_pair("filter", filter);
            }
            format!("edu/v1/catalog?{}", query.finish())
        };

        let request = NexusRequest { method: Method::GET, path, headers: HeaderMap::new(), body: None };
        let response = self.transport.send(request).await?;

        if !response.status.is_success() {
            return Err(EduGatewayError::UnexpectedStatus { status: response.status });
        }

        let envelope: LearningCatalogEnvelope =
            serde_json::from_value(response.body).map_err(EduGatewayError::UnexpectedResponseShape)?;
        Ok(envelope.snapshots)
    }
}
