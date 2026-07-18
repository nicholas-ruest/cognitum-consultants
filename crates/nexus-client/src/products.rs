//! Products ACL gateway (ADR-007, ADR-016, PROMPT-39).
//!
//! Products owns the approved-for-selling catalog; this repo never becomes
//! a second store of Products' own catalog/pricing data (invariant 3 of the
//! repo's own "Out-of-Scope Reminders") — only the [`ProductReferenceCard`]
//! projection `anti-corruption-layers.md` §7 names. [`ProductsGateway`] is a
//! thin translation boundary over Products' single outbound call — a
//! read-only catalog query, mirroring [`crate::edu::EduGateway`]'s
//! `request_learning_catalog` shape (a query in DDD terms, no side effect on
//! Products).
//!
//! # No `consultant_id`, unlike Edu/Customer/Execution's reads
//! `anti-corruption-layers.md` §7 names `RequestProductCatalogQuery`'s only
//! field as `filters?` — no `consultant_id`, unlike
//! [`crate::edu::EduGateway::request_learning_catalog`]/
//! [`crate::customer::CustomerGateway::request_assigned_customer_context`]/
//! [`crate::execution::ExecutionGateway::request_assigned_engagements`].
//! This is a deliberate, DDD-doc-traced difference, not an oversight: the
//! approved product catalog is the same for every consultant (it is not
//! permission-scoped per-consultant the way Customer's assigned-context read
//! is) — which is also exactly why this is this repo's single most
//! cacheable read (see the module docs' next section).
//!
//! # The most cacheable, least latency-sensitive gateway in this repo
//! Per this unit's own prompt text and `anti-corruption-layers.md` §7,
//! Products is explicitly the read-only capability that should get the
//! **longest** timeout and **most aggressive** retry budget of all ten
//! ACLs — not merely another "read-mostly" capability sharing Edu's
//! [`crate::timeout::DEFAULT_EXTENDED_READ_TIMEOUT`] tier (PROMPT-35). See
//! [`crate::timeout::DEFAULT_MAX_READ_TIMEOUT`] and
//! [`crate::retry::AGGRESSIVE_MAX_RETRIES`] for the two constants this
//! gateway's construction (`main.rs`) is expected to use, and each
//! constant's own doc comment for the full reasoning.
//!
//! # Read-mostly: no side-effecting command, no two-gateway split
//! Same shape as [`crate::edu::EduGateway`]/[`crate::customer::CustomerGateway`]:
//! Products' `anti-corruption-layers.md` §7 entry lists no outbound command
//! with a side effect — only `RequestProductCatalogQuery`. There is
//! therefore nothing here for the "two `Nexus<Capability>Gateway` instances,
//! one per retry-safety profile" convention (`crate::sales`/`crate::commit`
//! module docs) to split: a single [`NexusProductsGateway`] instance,
//! constructed once over a `RetryingTransport`-wrapped stack, safely serves
//! [`ProductsGateway::request_product_catalog`] — the only method this trait
//! has.
//!
//! # Request path: provisional, matching Edu's `.../v1/...` convention
//! Nexus's real Products contract is not finalized. This gateway assumes:
//! - `GET products/v1/catalog` (repeated `filter=...` query params for
//!   `filters`, if any) — response an envelope
//!   `{"cards": [ProductReferenceCard, ...]}`, matching
//!   [`crate::edu::NexusEduGateway`]'s `LearningCatalogEnvelope` convention
//!   (see that module's doc comment for why an envelope was chosen over a
//!   bare array).
//!
//! Update this once Nexus's actual Products contract is known.
//!
//! # Transport-stack-assembly convention
//! Same convention as [`crate::edu::NexusEduGateway`]/
//! [`crate::customer::NexusCustomerGateway`]: [`NexusProductsGateway::new`]
//! takes an already-fully-decorated `Arc<dyn NexusTransport>` and does not
//! assemble the ADR-016 timeout/retry/circuit-breaker stack itself.

use std::sync::Arc;

use async_trait::async_trait;

use crate::transport::{CapabilityCall, CapabilityCaller, NexusTransport, NexusTransportError};

/// ADR-029 capability id + target repo for this gateway's single call.
const CAPABILITY_CATALOG: &str = "products.catalog";
const TARGET_REPO: &str = "cognitum-products";

/// Products' approved-for-selling Product Reference Card projection
/// (`anti-corruption-layers.md` §7): this repo never becomes a second store
/// of Products' own catalog/pricing data — only this snapshot, refreshed on
/// demand and by `ProductCatalogUpdated` events
/// (`bff_core::event_ingestion`'s classifier).
///
/// `Serialize` (alongside `Deserialize`, used to decode Products' response)
/// is derived so `bff-api` can relay this same shape verbatim to the
/// frontend, matching `LearningSnapshot`'s/`CustomerContextCard`'s "no BFF
/// re-shaping" convention.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProductReferenceCard {
    pub product_id: String,
    pub name: String,
    pub packaging_summary: String,
    pub pricing_guidance: String,
    /// Opaque references (e.g. urls) to approved demo assets. Defaults to
    /// an empty list on deserialization (`#[serde(default)]`) rather than
    /// requiring every fixture/response to spell out an empty array —
    /// `anti-corruption-layers.md` §7 names this field but not whether
    /// Products' real contract always includes it.
    #[serde(default)]
    pub demo_assets: Vec<String>,
}

/// Envelope this gateway expects `GET products/v1/catalog`'s response body
/// to match. See the module docs for why an envelope (vs. a bare array) was
/// chosen (mirrors [`crate::edu::NexusEduGateway`]'s `LearningCatalogEnvelope`
/// rationale).
#[derive(Debug, serde::Deserialize)]
struct ProductCatalogEnvelope {
    cards: Vec<ProductReferenceCard>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProductsGatewayError {
    #[error(transparent)]
    Transport(#[from] NexusTransportError),
    #[error("Products returned a response body that did not match the expected shape: {0}")]
    UnexpectedResponseShape(#[source] serde_json::Error),
}

/// ACL over Products' read-only, approved-for-selling catalog capability. No
/// business policy (e.g. which products are "approved") is decided here —
/// see the module docs.
#[async_trait]
pub trait ProductsGateway: Send + Sync {
    /// Fetches the current approved [`ProductReferenceCard`] set, per
    /// `anti-corruption-layers.md` §7's `RequestProductCatalogQuery`.
    /// `filters`, if non-empty, is passed through to Products untouched —
    /// this repo has no opinion on what a valid filter value is (Products
    /// owns that vocabulary), the same convention
    /// [`crate::edu::EduGateway::request_learning_catalog`]'s `filters`
    /// parameter follows.
    ///
    /// A **query** in DDD terms — reading Products' current catalog state
    /// has no side effect, so retrying it is safe/idempotent, and — per the
    /// module docs — this repo's *most* aggressively retried call. See
    /// [`NexusProductsGateway`]'s doc comment for the transport requirement
    /// this method needs from its caller.
    async fn request_product_catalog(
        &self,
        filters: Option<&[String]>,
    ) -> Result<Vec<ProductReferenceCard>, ProductsGatewayError>;
}

/// [`ProductsGateway`] implementation backed by a [`NexusTransport`]. See
/// the module docs for the required transport decoration.
pub struct NexusProductsGateway {
    caller: CapabilityCaller,
}

impl NexusProductsGateway {
    /// `transport` is expected to already be decorated per the ADR-016
    /// longest-read-timeout + most-aggressive-retry convention (see module
    /// docs) — this constructor does not assemble timeout/retry/
    /// circuit-breaker layers itself. It is wrapped in a [`CapabilityCaller`]
    /// so this gateway issues the ADR-029 capability envelope.
    pub fn new(transport: Arc<dyn NexusTransport>) -> Self {
        Self { caller: CapabilityCaller::new(transport) }
    }
}

#[async_trait]
impl ProductsGateway for NexusProductsGateway {
    async fn request_product_catalog(
        &self,
        filters: Option<&[String]>,
    ) -> Result<Vec<ProductReferenceCard>, ProductsGatewayError> {
        // `filters`, when present, is passed through untouched (this repo has
        // no opinion on the filter vocabulary — see the trait method docs).
        let payload = match filters.filter(|f| !f.is_empty()) {
            Some(filters) => serde_json::json!({ "filters": filters }),
            None => serde_json::json!({}),
        };

        let response_payload = self
            .caller
            .call(CapabilityCall {
                capability_id: CAPABILITY_CATALOG.to_owned(),
                target_repo: TARGET_REPO.to_owned(),
                payload,
            })
            .await?;

        let envelope: ProductCatalogEnvelope =
            serde_json::from_value(response_payload).map_err(ProductsGatewayError::UnexpectedResponseShape)?;
        Ok(envelope.cards)
    }
}
