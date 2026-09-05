//! Provider-neutral runtime harness for CodexFlow.
//!
//! The native OpenAI/Codex execution path remains owned by Codex. External
//! runtimes implement [`AgentBackend`] and are selected only through an explicit
//! [`ProviderId`] carried by [`RuntimeModelId`]. Cursor is integrated over ACP
//! JSON-RPC stdio, while account and quota operations embed pinned subswap crates.

pub mod accounts;
pub mod auth;
pub mod capabilities;
pub mod controller;
pub mod cursor_acp;
pub mod cursor_models;
pub mod lazy_cursor;
pub mod model_catalog;
pub mod native_openai;
pub mod quota;
pub mod quota_service;
pub mod router;
pub mod selection;
pub mod session;
pub mod supervisor;
pub mod types;

pub use accounts::AccountBroker;
pub use accounts::AccountBrokerError;
pub use accounts::Activation;
pub use accounts::ImportedAccount;
pub use auth::AuthCoordinator;
pub use auth::AuthCoordinatorError;
pub use capabilities::ProviderCapabilities;
pub use controller::RuntimeHarness;
pub use controller::RuntimeHarnessError;
pub use cursor_acp::CursorAcpBackend;
pub use cursor_acp::CursorAcpConfig;
pub use cursor_acp::CursorAcpError;
pub use cursor_acp::normalize_session_update;
pub use cursor_acp::parse_permission_request;
pub use cursor_models::CursorModelDiscoveryError;
pub use cursor_models::discover_cursor_models;
pub use cursor_models::parse_cursor_model_list;
pub use lazy_cursor::LazyCursorBackend;
pub use model_catalog::ModelCatalog;
pub use model_catalog::ModelCatalogError;
pub use model_catalog::ModelDescriptor;
pub use model_catalog::ModelParameterDescriptor;
pub use native_openai::NativeOpenAiAuthReloader;
pub use native_openai::NativeOpenAiReloadError;
pub use quota::NormalizedQuota;
pub use quota::QuotaNormalizationError;
pub use quota::normalize_quotas;
pub use quota_service::AccountQuotaSnapshot;
pub use quota_service::ProviderQuotaSnapshot;
pub use quota_service::QuotaRefreshState;
pub use quota_service::QuotaService;
pub use quota_service::QuotaServiceError;
pub use quota_service::QuotaUpdateSink;
pub use router::AgentBackend;
pub use router::BackendFuture;
pub use router::RuntimeRoute;
pub use router::RuntimeRouter;
pub use router::RuntimeRouterError;
pub use selection::RuntimeSelection;
pub use selection::RuntimeSelectionError;
pub use selection::RuntimeSelectionStore;
pub use selection::RuntimeSelectionStoreError;
pub use session::HarnessEvent;
pub use session::HarnessSession;
pub use session::HarnessSessionError;
pub use session::RotationRetryDisposition;
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
