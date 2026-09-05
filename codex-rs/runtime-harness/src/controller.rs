use crate::AccountBroker;
use crate::AccountBrokerError;
use crate::Activation;
use crate::AuthCoordinator;
use crate::AuthCoordinatorError;
use crate::CursorAcpConfig;
use crate::CursorModelDiscoveryError;
use crate::HarnessSession;
use crate::ImportedAccount;
use crate::ModelCatalog;
use crate::ModelCatalogError;
use crate::ModelDescriptor;
use crate::ProviderId;
use crate::ProviderQuotaSnapshot;
use crate::QuotaService;
use crate::QuotaServiceError;
use crate::RuntimeModelId;
use crate::RuntimeRoute;
use crate::RuntimeRouter;
use crate::RuntimeRouterError;
use crate::RuntimeSelection;
use crate::RuntimeSelectionError;
use crate::RuntimeSelectionStore;
use crate::RuntimeSelectionStoreError;
use crate::RuntimeSessionSupervisor;
use crate::discover_cursor_models;
use std::path::PathBuf;
use std::sync::Arc;
use subswap_core::PolicyConfig;
use subswap_core::PolicyDecision;
use tokio::sync::RwLock;
use tokio::sync::Semaphore;

#[derive(Debug, thiserror::Error)]
pub enum RuntimeHarnessError {
    #[error(transparent)]
    Account(#[from] AccountBrokerError),
    #[error(transparent)]
    Auth(#[from] AuthCoordinatorError),
    #[error(transparent)]
    Router(#[from] RuntimeRouterError),
    #[error(transparent)]
    Selection(#[from] RuntimeSelectionError),
    #[error(transparent)]
    SelectionStore(#[from] RuntimeSelectionStoreError),
    #[error(transparent)]
    ModelCatalog(#[from] ModelCatalogError),
    #[error(transparent)]
    CursorModels(#[from] CursorModelDiscoveryError),
    #[error(transparent)]
    Quota(#[from] QuotaServiceError),
    #[error("operation targets provider {requested}, but active provider is {active}")]
    ProviderMismatch {
        active: ProviderId,
        requested: ProviderId,
    },
    #[error("runtime selection transition coordinator is unavailable")]
    TransitionCoordinatorUnavailable,
}

/// Composition root for the multi-runtime harness.
///
/// This object keeps provider/model/account selection, embedded subswap state,
/// quota service, runtime routing, and child-process invalidation coherent. It is
/// deliberately UI-agnostic so the Codex TUI can remain the presentation layer.
pub struct RuntimeHarness {
    broker: Arc<AccountBroker>,
    auth: Arc<AuthCoordinator>,
    router: Arc<RuntimeRouter>,
    quota: Arc<QuotaService>,
    catalog: RwLock<ModelCatalog>,
    selection: RwLock<RuntimeSelection>,
    selection_store: RuntimeSelectionStore,
    supervisor: RuntimeSessionSupervisor,
    cursor_config: CursorAcpConfig,
    transition_permit: Arc<Semaphore>,
}

impl RuntimeHarness {
    /// Build the production harness without launching Cursor ACP.
    ///
    /// The embedded broker initializes subswap storage/providers immediately,
    /// while `RuntimeRouter::with_lazy_cursor` defers `agent acp` until Cursor is
    /// actually used.
    pub fn embedded(
        default_model: RuntimeModelId,
        selection_path: PathBuf,
        cursor_config: CursorAcpConfig,
    ) -> Result<Self, RuntimeHarnessError> {
        let broker = Arc::new(AccountBroker::embedded_default()?);
        let selection_store = RuntimeSelectionStore::new(selection_path);
        let selection = selection_store
            .load()?
            .unwrap_or_else(|| RuntimeSelection::new(default_model));
        let auth = Arc::new(AuthCoordinator::new(Arc::clone(&broker)));
        let router = Arc::new(RuntimeRouter::with_lazy_cursor(cursor_config.clone()));
        let quota = Arc::new(QuotaService::from_default_paths(Arc::clone(&broker))?);
        Ok(Self {
            broker,
            auth,
            router,
            quota,
            catalog: RwLock::new(ModelCatalog::new()),
            selection: RwLock::new(selection),
            selection_store,
            supervisor: RuntimeSessionSupervisor::new(),
            cursor_config,
            transition_permit: Arc::new(Semaphore::new(1)),
        })
    }

    pub fn broker(&self) -> &Arc<AccountBroker> {
        &self.broker
    }

    pub fn auth(&self) -> &Arc<AuthCoordinator> {
        &self.auth
    }

    pub fn router(&self) -> &Arc<RuntimeRouter> {
        &self.router
    }

    pub fn quota_service(&self) -> &Arc<QuotaService> {
        &self.quota
    }

    pub fn supervisor(&self) -> &RuntimeSessionSupervisor {
        &self.supervisor
    }

    pub async fn selection(&self) -> RuntimeSelection {
        self.selection.read().await.clone()
    }

    pub async fn catalog(&self) -> ModelCatalog {
        self.catalog.read().await.clone()
    }

    async fn acquire_transition(&self) -> Result<tokio::sync::OwnedSemaphorePermit, RuntimeHarnessError> {
        Arc::clone(&self.transition_permit)
            .acquire_owned()
            .await
            .map_err(|_| RuntimeHarnessError::TransitionCoordinatorUnavailable)
    }

    pub async fn replace_openai_models(
        &self,
        models: Vec<ModelDescriptor>,
    ) -> Result<(), RuntimeHarnessError> {
        self.catalog
            .write()
            .await
            .replace_provider(ProviderId::OpenAi, models)?;
        Ok(())
    }

    /// Refresh Cursor's account-dependent model catalog without starting ACP.
    pub async fn refresh_cursor_models(
        &self,
    ) -> Result<Vec<ModelDescriptor>, RuntimeHarnessError> {
        let models = discover_cursor_models(&self.cursor_config).await?;
        self.catalog
            .write()
            .await
            .replace_provider(ProviderId::Cursor, models.clone())?;
        Ok(models)
    }

    /// Explicitly change providers by supplying a fully namespaced model for the
    /// destination provider. Switching away from Cursor terminates its ACP child.
    pub async fn select_provider(
        &self,
        model: RuntimeModelId,
    ) -> Result<RuntimeSelection, RuntimeHarnessError> {
        let _transition = self.acquire_transition().await?;
        let previous = self.selection.read().await.clone();
        let previous_provider = previous.provider();
        let mut next = previous.clone();
        next.select_provider(model);
        self.selection_store.save(&next)?;

        if previous_provider != next.provider()
            && let Err(error) = self.router.shutdown_provider(previous_provider).await
        {
            let _ = self.selection_store.save(&previous);
            return Err(error.into());
        }

        *self.selection.write().await = next.clone();
        self.supervisor.invalidate();
        Ok(next)
    }

    /// Change models within the active provider only. A provider transition must
    /// go through [`select_provider`](Self::select_provider).
    pub async fn select_model(
        &self,
        model: RuntimeModelId,
    ) -> Result<RuntimeSelection, RuntimeHarnessError> {
        let _transition = self.acquire_transition().await?;
        let current = self.selection.read().await.clone();
        let mut next = current;
        next.select_model_within_provider(model)?;
        self.selection_store.save(&next)?;
        *self.selection.write().await = next.clone();
        self.supervisor.invalidate();
        Ok(next)
    }

    /// Activate an account for the active provider. Account selection cannot
    /// implicitly switch runtimes. Cursor ACP is terminated after a successful
    /// activation so the next operation authenticates a fresh child.
    pub async fn activate_account(
        &self,
        provider: ProviderId,
        account_id: impl Into<String>,
    ) -> Result<Activation, RuntimeHarnessError> {
        let _transition = self.acquire_transition().await?;
        let current = self.selection.read().await.clone();
        if current.provider() != provider {
            return Err(RuntimeHarnessError::ProviderMismatch {
                active: current.provider(),
                requested: provider,
            });
        }

        let activation = self.broker.activate(provider, account_id).await?;
        let mut next = current;
        next.select_account(provider, activation.account_id.clone())?;
        self.selection_store.save(&next)?;
        *self.selection.write().await = next;
        self.supervisor.invalidate();
        self.router.shutdown_provider(provider).await?;
        Ok(activation)
    }

    /// Run Cursor's official browser login and import the resulting native
    /// credential into embedded subswap. Login does not switch providers.
    pub async fn login_cursor(
        &self,
        label_hint: Option<String>,
    ) -> Result<ImportedAccount, RuntimeHarnessError> {
        let imported = self.auth.login_cursor(label_hint).await?;
        self.after_import(ProviderId::Cursor, &imported).await?;
        Ok(imported)
    }

    /// Import credentials after the existing Codex/OpenAI login UI succeeds.
    pub async fn import_after_native_login(
        &self,
        provider: ProviderId,
        label_hint: Option<String>,
    ) -> Result<ImportedAccount, RuntimeHarnessError> {
        let imported = self
            .auth
            .import_after_native_login(provider, label_hint)
            .await?;
        self.after_import(provider, &imported).await?;
        Ok(imported)
    }

    async fn after_import(
        &self,
        provider: ProviderId,
        imported: &ImportedAccount,
    ) -> Result<(), RuntimeHarnessError> {
        let _transition = self.acquire_transition().await?;
        let current = self.selection.read().await.clone();
        if current.provider() != provider {
            return Ok(());
        }
        let mut next = current;
        next.select_account(provider, imported.account.id.0.clone())?;
        self.selection_store.save(&next)?;
        *self.selection.write().await = next;
        self.supervisor.invalidate();
        self.router.shutdown_provider(provider).await?;
        Ok(())
    }

    pub async fn quota_snapshot(
        &self,
        provider: ProviderId,
    ) -> Result<ProviderQuotaSnapshot, RuntimeHarnessError> {
        Ok(self.quota.refresh_provider(provider).await?)
    }

    /// Evaluate the embedded subswap policy for the active provider only and
    /// apply a same-provider rotation when selected by policy.
    pub async fn auto_swap_current(
        &self,
        policy: &PolicyConfig,
    ) -> Result<PolicyDecision, RuntimeHarnessError> {
        let provider = self.selection.read().await.provider();
        let decision = self.broker.evaluate_auto_swap(provider, policy).await?;
        if let PolicyDecision::Swap { to, .. } = &decision {
            self.activate_account(provider, to.0.clone()).await?;
        }
        Ok(decision)
    }

    pub async fn route_current(&self) -> Result<RuntimeRoute, RuntimeHarnessError> {
        let model = self.selection.read().await.model.clone();
        Ok(self.router.route(&model)?)
    }

    pub async fn new_session(
        &self,
        id: impl Into<String>,
        working_directory: PathBuf,
    ) -> HarnessSession {
        let selection = self.selection.read().await.clone();
        let mut session = HarnessSession::new(id, working_directory, selection.model);
        session.account = selection.account_id;
        session
    }

    pub async fn shutdown(&self) -> Result<(), RuntimeHarnessError> {
        self.supervisor.invalidate();
        self.router.shutdown_provider(ProviderId::Cursor).await?;
        Ok(())
    }
}
