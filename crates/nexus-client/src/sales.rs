//! Sales ACL gateway (ADR-007, ADR-016, PROMPT-24).
//!
//! Sales owns the lead-conflict/account-claim *decision*; this repo never
//! re-derives or overrides `creation_allowed` or any other part of Sales'
//! verdict (`../../.plans/ddd/anti-corruption-layers.md` §1). [`SalesGateway`]
//! is a thin translation boundary over three outbound calls: a claim check
//! (query-shaped command) and two commands with side effects.
//!
//! # Request paths: provisional, matching Armor's `armor/v1/...` convention
//! Nexus's real Sales contract is not finalized. This gateway assumes:
//! - `POST sales/v1/account-claims` — body [`CheckAccountClaimCommand`],
//!   response an [`AccountClaimResult`].
//! - `POST sales/v1/collaboration-requests` — body
//!   [`RequestCollaborationCommand`].
//! - `POST sales/v1/referrals` — body [`SubmitReferralCommand`].
//!
//! Update these once Nexus's actual Sales contract is known.
//!
//! # Ack response shape for `request_collaboration` / `submit_referral`
//! The anti-corruption-layers doc lists inbound
//! `CollaborationRequestAcknowledged` / `ReferralSubmitted` events, but does
//! not spell out their field shape (unlike the worked `AccountClaimResult`
//! example). Rather than invent DTO fields with no worked example to match,
//! this gateway treats both calls as fire-and-confirm: `Ok(())` on any
//! success status, [`SalesGatewayError`] otherwise. This is deliberately
//! narrower than `ArmorGateway`'s pattern (which fully decodes a documented
//! shape) — widen to a real ack DTO once Nexus's actual
//! `CollaborationRequestAcknowledged` / `ReferralSubmitted` body shape is
//! known.
//!
//! # Transport-stack-assembly convention
//! Same convention as [`crate::armor::NexusArmorGateway`]:
//! [`NexusSalesGateway::new`] takes an already-fully-decorated
//! `Arc<dyn NexusTransport>` and does not assemble the ADR-016
//! timeout/retry/circuit-breaker stack itself. See
//! [`NexusSalesGateway`]'s doc comment for this gateway's specific
//! per-method timeout and retry-safety requirements.

use std::sync::Arc;

use async_trait::async_trait;

use crate::transport::{CapabilityCall, CapabilityCaller, NexusTransport, NexusTransportError};

/// ADR-029 capability ids + target repo for this gateway's three calls.
const CAPABILITY_ACCOUNT_CLAIMS: &str = "sales.account_claims";
const CAPABILITY_COLLABORATION_REQUESTS: &str = "sales.collaboration_requests";
const CAPABILITY_REFERRALS: &str = "sales.referrals";
const TARGET_REPO: &str = "cognitum-sales";

/// Sales' opaque verdict on a company/lead claim check. This repo never
/// models Sales' internal Company/Lead/Contact/Opportunity graph — only
/// this verdict plus the short list of actions it's allowed to render
/// (`anti-corruption-layers.md` §1).
///
/// `Serialize` (alongside `Deserialize`, used to decode Sales' response
/// body) is derived so `bff-api` can relay this same shape verbatim to the
/// frontend in PROMPT-25 without a parallel DTO — per
/// `anti-corruption-layers.md` §1 step 5, the BFF must not re-adjudicate
/// `creation_allowed`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AccountClaimResult {
    pub match_status: String,
    pub creation_allowed: bool,
    pub display_message: String,
    pub permitted_actions: Vec<String>,
}

/// Outbound query-shaped command: asks Sales to evaluate a company name for
/// existing ownership/conflict, it does not assert any fact
/// (`anti-corruption-layers.md` §1 step 2).
#[derive(Debug, Clone, serde::Serialize)]
struct CheckAccountClaimCommand<'a> {
    company_name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    normalized_domain: Option<&'a str>,
    consultant_id: &'a str,
}

/// Outbound command: request collaboration on a company already claimed by
/// another consultant. Has a side effect (creates a collaboration request
/// record in Sales) — never idempotent-safe to blindly retry.
#[derive(Debug, Clone, serde::Serialize)]
struct RequestCollaborationCommand<'a> {
    company_reference: &'a str,
    consultant_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'a str>,
}

/// Outbound command: submit a referral for a company this consultant will
/// not pursue directly. Has a side effect (creates a referral record in
/// Sales) — never idempotent-safe to blindly retry.
#[derive(Debug, Clone, serde::Serialize)]
struct SubmitReferralCommand<'a> {
    company_reference: &'a str,
    consultant_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<&'a str>,
}

#[derive(Debug, thiserror::Error)]
pub enum SalesGatewayError {
    #[error(transparent)]
    Transport(#[from] NexusTransportError),
    #[error("Sales returned a response body that did not match the expected shape: {0}")]
    UnexpectedResponseShape(#[source] serde_json::Error),
}

/// ACL over Sales' account-claim and collaboration/referral capabilities.
/// No re-adjudication of Sales' verdicts happens on this trait — see the
/// module docs.
#[async_trait]
pub trait SalesGateway: Send + Sync {
    /// Checks whether `company_name` is already owned/worked by another
    /// consultant, per `anti-corruption-layers.md` §1's worked example.
    ///
    /// This is a **query** in DDD terms — reading Sales' current claim
    /// state has no side effect, so retrying it is safe/idempotent in that
    /// sense — but it is also the *user-blocking synchronous call* ADR-016
    /// requires the shorter write timeout budget for, because the
    /// consultant is actively waiting on it in the UI. See
    /// [`NexusSalesGateway`]'s doc comment for the exact transport
    /// requirement this method needs from its caller.
    async fn check_account_claim(
        &self,
        company_name: &str,
        consultant_id: &str,
    ) -> Result<AccountClaimResult, SalesGatewayError>;

    /// Requests collaboration on a company already claimed by another
    /// consultant. **Not idempotent-safe to retry** — a retry against an
    /// unknown-outcome prior attempt risks creating a duplicate
    /// collaboration request record in Sales.
    async fn request_collaboration(
        &self,
        company_reference: &str,
        consultant_id: &str,
        message: Option<&str>,
    ) -> Result<(), SalesGatewayError>;

    /// Submits a referral for a company this consultant will not pursue
    /// directly. **Not idempotent-safe to retry** — a retry against an
    /// unknown-outcome prior attempt risks creating a duplicate referral
    /// record in Sales.
    async fn submit_referral(
        &self,
        company_reference: &str,
        consultant_id: &str,
        notes: Option<&str>,
    ) -> Result<(), SalesGatewayError>;
}

/// [`SalesGateway`] implementation backed by a [`NexusTransport`].
///
/// # Construction requirement: timeout budget (ADR-016 / PROMPT-13)
/// `transport` is expected to already be decorated per the ADR-016 **write**
/// timeout convention (3s, [`crate::timeout::DEFAULT_WRITE_TIMEOUT`]), NOT
/// the 5s read convention Armor uses — even though `check_account_claim` is
/// a query in DDD terms, it is a user-blocking synchronous call the
/// consultant is actively waiting on in the UI, so it needs the *shorter*
/// budget. This constructor does not assemble timeout/retry/circuit-breaker
/// layers itself; that composition happens once at the call site (same
/// convention as [`crate::armor::NexusArmorGateway`]).
///
/// # Construction requirement: per-method retry safety (ADR-016)
/// `check_account_claim` reads Sales' current claim state and has no side
/// effect, so — like Armor's `fetch_assertions` — it MAY be issued over a
/// transport wrapped in [`crate::retry::RetryingTransport`], e.g.
/// `RetryingTransport::with_default_retries(Arc::new(TimeoutTransport::new(base, DEFAULT_WRITE_TIMEOUT)))`.
/// `request_collaboration` and `submit_referral` both create a record in
/// Sales as a side effect and are NOT idempotent-safe to retry: never pass
/// a `RetryingTransport`-wrapped stack to a caller path that only ever
/// calls these two methods. Because this gateway's single `transport`
/// field is shared across all three methods, a caller that needs
/// `check_account_claim`'s retry benefit alongside safe
/// `request_collaboration` / `submit_referral` calls must either accept
/// the same non-retrying stack for all three (safe default) or construct
/// two `NexusSalesGateway` instances, one per timeout/retry profile.
pub struct NexusSalesGateway {
    caller: CapabilityCaller,
}

impl NexusSalesGateway {
    /// See the struct doc comment for the required transport decoration
    /// (write timeout budget; retry only safe for `check_account_claim`).
    /// The `transport` is wrapped in a [`CapabilityCaller`] so each method
    /// issues the ADR-029 capability envelope.
    pub fn new(transport: Arc<dyn NexusTransport>) -> Self {
        Self { caller: CapabilityCaller::new(transport) }
    }

    async fn call(
        &self,
        capability_id: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, SalesGatewayError> {
        Ok(self
            .caller
            .call(CapabilityCall { capability_id: capability_id.to_owned(), target_repo: TARGET_REPO.to_owned(), payload })
            .await?)
    }
}

#[async_trait]
impl SalesGateway for NexusSalesGateway {
    async fn check_account_claim(
        &self,
        company_name: &str,
        consultant_id: &str,
    ) -> Result<AccountClaimResult, SalesGatewayError> {
        let command = CheckAccountClaimCommand { company_name, normalized_domain: None, consultant_id };
        let payload = self
            .call(CAPABILITY_ACCOUNT_CLAIMS, serde_json::to_value(command).expect("command always serializes"))
            .await?;
        serde_json::from_value(payload).map_err(SalesGatewayError::UnexpectedResponseShape)
    }

    async fn request_collaboration(
        &self,
        company_reference: &str,
        consultant_id: &str,
        message: Option<&str>,
    ) -> Result<(), SalesGatewayError> {
        let command = RequestCollaborationCommand { company_reference, consultant_id, message };
        self.call(CAPABILITY_COLLABORATION_REQUESTS, serde_json::to_value(command).expect("command always serializes"))
            .await?;
        Ok(())
    }

    async fn submit_referral(
        &self,
        company_reference: &str,
        consultant_id: &str,
        notes: Option<&str>,
    ) -> Result<(), SalesGatewayError> {
        let command = SubmitReferralCommand { company_reference, consultant_id, notes };
        self.call(CAPABILITY_REFERRALS, serde_json::to_value(command).expect("command always serializes")).await?;
        Ok(())
    }
}
