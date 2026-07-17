//! `tracing-subscriber` initialization (ADR-012).
//!
//! Log format is controlled by the `LOG_FORMAT` env var: `pretty` (default,
//! human-readable, for local dev) or `json` (structured, for deployed
//! environments). ADR-012 mandates the human-vs-JSON split but does not pin
//! an exact env var name for the switch; `LOG_FORMAT` is this crate's choice.
//! Verbosity is controlled the standard `tracing-subscriber` way via
//! `RUST_LOG` (defaults to `info` if unset).
//!
//! Also wires a `tracing-opentelemetry` layer so spans carry W3C Trace
//! Context-compatible identifiers per ADR-012. No exporter/collector backend
//! is configured yet — provisioning one is ADR-014's concern — so spans are
//! produced in-process (enabling correlation-ID-style propagation today) but
//! are not shipped to a collector until that backend is chosen.

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

const LOG_FORMAT_ENV: &str = "LOG_FORMAT";

/// Initializes the global `tracing` subscriber. Must be called once at
/// process startup, before any spans/events are recorded.
pub fn init() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // No span exporter attached yet (ADR-014 will provision a backend); the
    // provider still lets tracing-opentelemetry stamp spans with W3C trace
    // context so nothing needs re-instrumenting once a backend exists.
    let tracer_provider = SdkTracerProvider::builder().build();
    let tracer = tracer_provider.tracer("bff-api");
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    let registry = Registry::default().with(env_filter).with(otel_layer);

    let json_format = std::env::var(LOG_FORMAT_ENV)
        .map(|value| value.eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    if json_format {
        registry.with(tracing_subscriber::fmt::layer().json()).init();
    } else {
        registry.with(tracing_subscriber::fmt::layer().pretty()).init();
    }
}
