//! Customer ACL gateway (ADR-007, ADR-009, ADR-016, PROMPT-37).
//!
//! Customer owns each customer relationship's own health/interaction
//! history; this repo never re-derives or overrides a `CustomerContextCard`'s
//! `health_status`/`relationship_summary`, and never stores the underlying
//! customer record itself — only this read projection
//! (`../../.plans/ddd/anti-corruption-layers.md` §5). [`CustomerGateway`] is
//! a thin translation boundary over Customer's single outbound call — a
//! read-only, permission-scoped query, mirroring
//! [`crate::edu::EduGateway`]'s `request_learning_catalog` shape (a query in
//! DDD terms, no side effect on Customer).
//!
//! # Permission filtering happens at the query boundary, not client-side
//! `anti-corruption-layers.md` §5: `RequestAssignedCustomerContextQuery` is
//! itself scoped to "assigned or permitted" customers — Customer (via Nexus)
//! decides which customers a given `consultant_id` may see and returns only
//! those, the same way `ArmorGateway::fetch_assertions` is the source of
//! truth for a consultant's own permission grants. This gateway (and
//! `bff-api::customer`, its BFF caller) never fetches a broader set and
//! filters it down locally — there is no code path here that could even
//! attempt that, since the only method this trait exposes already takes
//! `consultant_id` as a required argument scoping every result Customer
//! returns.
//!
//! # Read-mostly: no side-effecting command, no two-gateway split
//! Unlike Sales/Commit/Capacity, Customer's `anti-corruption-layers.md` §5
//! entry lists no outbound command with a side effect — only
//! `RequestAssignedCustomerContextQuery`. There is therefore nothing here for
//! the "two `Nexus<Capability>Gateway` instances, one per retry-safety
//! profile" convention (`crate::sales`/`crate::commit`/`crate::capacity`
//! module docs) to split: a single [`NexusCustomerGateway`] instance,
//! constructed once over a `RetryingTransport`-wrapped stack, safely serves
//! [`CustomerGateway::request_assigned_customer_context`] — the only method
//! this trait has, matching [`crate::edu::NexusEduGateway`]'s "no
//! `edu_command_gateway`" precedent exactly.
//!
//! # Request path: provisional, matching Commit's `.../v1/...` convention
//! Nexus's real Customer contract is not finalized. This gateway assumes:
//! - `GET customer/v1/context?consultant_id=...` (optional
//!   `customer_id=...` query param, when narrowing to one customer) —
//!   response an envelope `{"contexts": [CustomerContextCard, ...]}`,
//!   matching [`crate::commit::NexusCommitGateway`]'s `ProposalsEnvelope`
//!   convention (see that module's doc comment for why an envelope was
//!   chosen over a bare array).
//!
//! Update this once Nexus's actual Customer contract is known.
//!
//! # Timeout budget choice
//! `request_assigned_customer_context` is a background/page-load read (the
//! consultant is not actively blocked on it mid-keystroke the way Sales'
//! `check_account_claim` is) — it uses
//! [`crate::timeout::DEFAULT_READ_TIMEOUT`], the same budget
//! `CommitGateway::list_proposals`/`CapacityGateway::get_own_profile` use,
//! and MAY be retried ([`crate::retry::RetryingTransport`]-wrapped) since it
//! has no side effect.
//!
//! # Transport-stack-assembly convention
//! Same convention as [`crate::sales::NexusSalesGateway`]/
//! [`crate::edu::NexusEduGateway`]: [`NexusCustomerGateway::new`] takes an
//! already-fully-decorated `Arc<dyn NexusTransport>` and does not assemble
//! the ADR-016 timeout/retry/circuit-breaker stack itself.

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Method;
use reqwest::header::HeaderMap;

use crate::transport::{NexusRequest, NexusTransport, NexusTransportError};

/// Customer's Customer Context Card projection (`anti-corruption-layers.md`
/// §5): this repo never models Customer's internal account/contact/
/// interaction-history graph — only this permission-scoped summary card,
/// plus a `deep_link` back into Customer's own UI for anything beyond it.
///
/// `Serialize` (alongside `Deserialize`, used to decode Customer's response)
/// is derived so `bff-api` can relay this same shape verbatim to the
/// frontend, matching `ProposalSummary`'s/`LearningSnapshot`'s "no BFF
/// re-shaping" convention.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CustomerContextCard {
    pub customer_id: String,
    pub name: String,
    pub health_status: String,
    pub relationship_summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deep_link: Option<String>,
}

/// Envelope this gateway expects `GET customer/v1/context`'s response body
/// to match. See the module docs for why an envelope (vs. a bare array) was
/// chosen (mirrors [`crate::commit::NexusCommitGateway`]'s
/// `ProposalsEnvelope` rationale).
#[derive(Debug, serde::Deserialize)]
struct CustomerContextEnvelope {
    contexts: Vec<CustomerContextCard>,
}

#[derive(Debug, thiserror::Error)]
pub enum CustomerGatewayError {
    #[error(transparent)]
    Transport(#[from] NexusTransportError),
    #[error("Customer returned a non-success status {status}")]
    UnexpectedStatus { status: reqwest::StatusCode },
    #[error("Customer returned a response body that did not match the expected shape: {0}")]
    UnexpectedResponseShape(#[source] serde_json::Error),
}

/// ACL over Customer's read-only, assigned/permitted Customer Context Card
/// capability. No re-adjudication of Customer's own `health_status`
/// happens on this trait — see the module docs.
#[async_trait]
pub trait CustomerGateway: Send + Sync {
    /// Fetches `consultant_id`'s assigned/permitted [`CustomerContextCard`]
    /// set, per `anti-corruption-layers.md` §5's
    /// `RequestAssignedCustomerContextQuery`. `customer_id`, when `Some`,
    /// narrows the query to that single customer (still subject to the same
    /// assigned/permitted scoping) — passed through to Customer untouched,
    /// same "this repo has no opinion on the filter vocabulary" convention
    /// [`crate::edu::EduGateway::request_learning_catalog`]'s `filters`
    /// parameter follows.
    ///
    /// A **query** in DDD terms — reading Customer's current context state
    /// has no side effect, so retrying it is safe/idempotent. See
    /// [`NexusCustomerGateway`]'s doc comment for the transport requirement
    /// this method needs from its caller.
    async fn request_assigned_customer_context(
        &self,
        consultant_id: &str,
        customer_id: Option<&str>,
    ) -> Result<Vec<CustomerContextCard>, CustomerGatewayError>;
}

/// [`CustomerGateway`] implementation backed by a [`NexusTransport`]. See
/// the module docs for the required transport decoration.
pub struct NexusCustomerGateway {
    transport: Arc<dyn NexusTransport>,
}

impl NexusCustomerGateway {
    /// `transport` is expected to already be decorated per the ADR-016 read
    /// convention (see module docs) — this constructor does not assemble
    /// timeout/retry/circuit-breaker layers itself.
    pub fn new(transport: Arc<dyn NexusTransport>) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl CustomerGateway for NexusCustomerGateway {
    async fn request_assigned_customer_context(
        &self,
        consultant_id: &str,
        customer_id: Option<&str>,
    ) -> Result<Vec<CustomerContextCard>, CustomerGatewayError> {
        let path = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.append_pair("consultant_id", consultant_id);
            if let Some(customer_id) = customer_id {
                query.append_pair("customer_id", customer_id);
            }
            format!("customer/v1/context?{}", query.finish())
        };

        let request = NexusRequest { method: Method::GET, path, headers: HeaderMap::new(), body: None };
        let response = self.transport.send(request).await?;

        if !response.status.is_success() {
            return Err(CustomerGatewayError::UnexpectedStatus { status: response.status });
        }

        let envelope: CustomerContextEnvelope =
            serde_json::from_value(response.body).map_err(CustomerGatewayError::UnexpectedResponseShape)?;
        Ok(envelope.contexts)
    }
}
