//! Provider-neutral runtime harness for CodexFlow.
//!
//! The native OpenAI/Codex execution path remains owned by Codex. External
//! runtimes implement [`AgentBackend`] and are selected only through an explicit
//! [`ProviderId`] carried by [`RuntimeModelId`]. Cursor is integrated over ACP
//! JSON-RPC stdio, while account and quota operations embed pinned subswap crates.

pub mod accounts;
pub mod capabilities;
pub mod cursor_acp;
pub mod quota;
pub mod router;
pub mod selection;
pub mod supervisor;
pub mod types;

pub use accounts::AccountBroker;
pub use accounts::AccountBrokerError;
pub use accounts::Activation;
pub use capabilities::ProviderCapabilities;
pub use cursor_acp::CursorAcpBackend;
pub use cursor_acp::CursorAcpConfig;
pub use cursor_acp::CursorAcpError;
pub use cursor_acp::normalize_session_update;
pub use cursor_acp::parse_permission_request;
pub use quota::NormalizedQuota;
pub use quota::QuotaNormalizationError;
pub use quota::normalize_quotas;
pub use router::AgentBackend;
pub use router::BackendFuture;
pub use router::RuntimeRoute;
pub use router::RuntimeRouter;
pub use router::RuntimeRouterError;
pub use selection::RuntimeSelection;
pub use selection::RuntimeSelectionError;
pub use selection::RuntimeSelectionStore;
pub use selection::RuntimeSelectionStoreError;
pub use supervisor::RuntimeSessionSupervisor;
pub use supervisor::SupervisedSession;
pub use types::ModelIdError;
pub use types::PermissionOption;
pub use types::PermissionOutcome;
pub use types::PermissionRequest;
pub use types::ProviderId;
pub use types::ProviderParseError;
pub use types::RejectingInteractionHandler;
pub use types::RuntimeEvent;
pub use types::RuntimeEventSink;
pub use types::RuntimeInteractionHandler;
pub use types::RuntimeModelId;
pub use types::RuntimeSessionId;

pub const SUBSWAP_REVISION: &str = "c839bd4de69397612d09fdc9312e03cf6e9c9e05";
pub const ACP_PROTOCOL_VERSION: u32 = 1;
