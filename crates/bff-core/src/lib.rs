//! bff-core: domain-agnostic aggregation/composition logic and this repo's own
//! aggregates and DTOs (see ADR-004, ../ddd/consultant-experience-context.md).
//! Depends only on nexus-client's trait interfaces, never its concrete
//! implementations. `persistence` depends on `bff-core` (not the reverse) so
//! it can implement the repository traits defined here (e.g.
//! [`ConsultantPreferencesRepository`]) against the ADR-010 datastore.

mod action_queue_entry;
mod consultant_preferences;
mod dashboard_configuration;
mod event_ingestion;
mod notification_item;
mod preference_key;
mod save_outcome;
mod workflow_session;

pub use action_queue_entry::{
    ActionQueueEntry, ActionQueueEntryError, ActionQueueRepository, ActionState,
    ParseActionStateError,
};
pub use consultant_preferences::{
    ConsultantPreferences, ConsultantPreferencesError, ConsultantPreferencesRepository, RepoError,
};
pub use dashboard_configuration::{
    CardPlacement, DashboardConfiguration, DashboardConfigurationError,
    DashboardConfigurationRepository, DEFAULT_CARD_MODULE_IDS,
};
pub use event_ingestion::{
    classify, ingest_events, CapabilityEventReceived, EventBus, EventClassification,
    IngestedEvent, IngestionOutcome, IngestionResult, DEFAULT_ACTION_QUEUE_ENTRY_TTL_HOURS,
    DEFAULT_EVENT_BUS_CAPACITY,
};
pub use notification_item::{
    NotificationItem, NotificationItemError, NotificationRepository, ParseReadStateError,
    ReadState,
};
pub use preference_key::{ParsePreferenceKeyError, PreferenceKey};
pub use save_outcome::SaveOutcome;
pub use workflow_session::{
    CrossCapabilityWorkflowSession, ParseWorkflowSessionStatusError, WorkflowSessionError,
    WorkflowSessionRepository, WorkflowSessionStatus, DEFAULT_WORKFLOW_SESSION_TTL_MINUTES,
};
