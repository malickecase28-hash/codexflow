use crate::types::ProviderId;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use subswap_core::Account;
use subswap_core::AccountId;
use subswap_core::AccountRegistry;
use subswap_core::AccountWithQuotas;
use subswap_core::CredentialStore;
use subswap_core::FileStore;
use subswap_core::KeyringStore;
use subswap_core::PolicyConfig;
use subswap_core::PolicyDecision;
use subswap_core::Provider;
use subswap_core::ProviderSnapshot;
use subswap_core::Quota;
use subswap_core::QuotaFetchState;
use subswap_core::auto_decide;
use subswap_core::paths::AppPaths;
use tokio::sync::Semaphore;

#[derive(Debug, thiserror::Error)]
pub enum AccountBrokerError {
    #[error("subswap error: {0}")]
    Subswap(#[from] subswap_core::Error),
    #[error("provider {0} is not registered with the account broker")]
    ProviderUnavailable(ProviderId),
    #[error("provider {0} account swap coordinator is unavailable")]
    SwapCoordinatorUnavailable(ProviderId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activation {
    pub provider: ProviderId,
    pub account_id: String,
    pub generation: u64,
}

struct ProviderAccountState {
    swap_permit: Arc<Semaphore>,
    generation: AtomicU64,
}

impl ProviderAccountState {
    fn new() -> Self {
        Self {
            swap_permit: Arc::new(Semaphore::new(1)),
            generation: AtomicU64::new(0),
        }
    }
}

pub struct AccountBroker {
    providers: HashMap<ProviderId, Arc<dyn Provider>>,
    states: HashMap<ProviderId, Arc<ProviderAccountState>>,
}

impl AccountBroker {
    pub fn new(providers: HashMap<ProviderId, Arc<dyn Provider>>) -> Self {
        let states = providers
            .keys()
            .copied()
            .map(|provider| (provider, Arc::new(ProviderAccountState::new())))
            .collect();
        Self { providers, states }
    }

    /// Build the account controller directly from pinned subswap crates.
    ///
    /// This deliberately embeds the providers instead of invoking the `subswap`
    /// executable. Credentials stay in subswap's CredentialStore and metadata
    /// stays in AccountRegistry.
    pub fn embedded_default() -> Result<Self, AccountBrokerError> {
        let paths = AppPaths::resolve()?;
        let store: Arc<dyn CredentialStore> = Arc::new(FileStore::with_legacy_keyring(
            paths.credentials_file(),
            KeyringStore::new(),
        ));
        let registry = Arc::new(AccountRegistry::new(paths.registry_file()));

        let codex: Arc<dyn Provider> = Arc::new(subswap_provider_codex::new(
            Arc::clone(&store),
            Arc::clone(&registry),
        ));
        let cursor: Arc<dyn Provider> = Arc::new(subswap_provider_cursor::CursorProvider::new(
            Arc::clone(&store),
            Arc::clone(&registry),
        )?);

        let mut providers = HashMap::new();
        providers.insert(ProviderId::OpenAi, codex);
        providers.insert(ProviderId::Cursor, cursor);
        Ok(Self::new(providers))
    }

    fn provider(&self, provider: ProviderId) -> Result<&Arc<dyn Provider>, AccountBrokerError> {
        self.providers
            .get(&provider)
            .ok_or(AccountBrokerError::ProviderUnavailable(provider))
    }

    fn state(&self, provider: ProviderId) -> Result<&Arc<ProviderAccountState>, AccountBrokerError> {
        self.states
            .get(&provider)
            .ok_or(AccountBrokerError::ProviderUnavailable(provider))
    }

    pub fn generation(&self, provider: ProviderId) -> Result<u64, AccountBrokerError> {
        Ok(self.state(provider)?.generation.load(Ordering::Acquire))
    }

    pub async fn list_accounts(
        &self,
        provider: ProviderId,
    ) -> Result<Vec<Account>, AccountBrokerError> {
        Ok(self.provider(provider)?.list_accounts().await?)
    }

    /// Activate an account under a provider-specific serialization permit and
    /// increment the provider's account generation only after a successful,
    /// transactional subswap activation.
    pub async fn activate(
        &self,
        provider: ProviderId,
        account_id: impl Into<String>,
    ) -> Result<Activation, AccountBrokerError> {
        let account_id = account_id.into();
        let id = AccountId(account_id.clone());
        let state = Arc::clone(self.state(provider)?);
        let _permit = Arc::clone(&state.swap_permit)
            .acquire_owned()
            .await
            .map_err(|_| AccountBrokerError::SwapCoordinatorUnavailable(provider))?;
        self.provider(provider)?.activate(&id).await?;
        let generation = state.generation.fetch_add(1, Ordering::AcqRel) + 1;
        Ok(Activation {
            provider,
            account_id,
            generation,
        })
    }

    pub async fn query_quota(
        &self,
        provider: ProviderId,
        account_id: impl Into<String>,
    ) -> Result<Vec<Quota>, AccountBrokerError> {
        let id = AccountId(account_id.into());
        Ok(self.provider(provider)?.query_quota(&id).await?)
    }

    pub async fn active_account(
        &self,
        provider: ProviderId,
    ) -> Result<Option<Account>, AccountBrokerError> {
        Ok(self
            .list_accounts(provider)
            .await?
            .into_iter()
            .find(|account| account.active))
    }

    /// Evaluate subswap's autoswap policy for exactly one runtime provider.
    /// Query failures are represented as failed quota states so the policy can
    /// make its documented degraded/fallback decision instead of accidentally
    /// crossing provider boundaries.
    pub async fn evaluate_auto_swap(
        &self,
        provider: ProviderId,
        config: &PolicyConfig,
    ) -> Result<PolicyDecision, AccountBrokerError> {
        let implementation = self.provider(provider)?;
        let accounts = implementation.list_accounts().await?;
        let mut accounts_with_quotas = Vec::with_capacity(accounts.len());
        for account in accounts {
            let (quotas, fetch_state) = match implementation.query_quota(&account.id).await {
                Ok(quotas) => (quotas, QuotaFetchState::Ready),
                Err(error) => (Vec::new(), QuotaFetchState::Failed(error.to_string())),
            };
            accounts_with_quotas.push(AccountWithQuotas {
                account,
                quotas,
                fetch_state,
            });
        }
        let snapshot = ProviderSnapshot {
            provider: provider.subswap_id().to_string(),
            accounts: accounts_with_quotas,
        };
        Ok(auto_decide(&snapshot, config))
    }

    /// Evaluate and, only when subswap chooses a same-provider target, activate it.
    pub async fn apply_auto_swap(
        &self,
        provider: ProviderId,
        config: &PolicyConfig,
    ) -> Result<PolicyDecision, AccountBrokerError> {
        let decision = self.evaluate_auto_swap(provider, config).await?;
        if let PolicyDecision::Swap { to, .. } = &decision {
            self.activate(provider, to.0.clone()).await?;
        }
        Ok(decision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use subswap_core::ClientTarget;
    use subswap_core::QuotaStatus;
    use subswap_core::QuotaWindow;

    struct FakeProvider {
        id: &'static str,
        accounts: Mutex<Vec<Account>>,
    }

    impl FakeProvider {
        fn new(id: &'static str, accounts: Vec<Account>) -> Self {
            Self {
                id,
                accounts: Mutex::new(accounts),
            }
        }
    }

    #[async_trait::async_trait]
    impl Provider for FakeProvider {
        fn id(&self) -> &'static str {
            self.id
        }

        fn display_name(&self) -> &'static str {
            self.id
        }

        fn client_targets(&self) -> Vec<ClientTarget> {
            Vec::new()
        }

        async fn list_accounts(&self) -> subswap_core::Result<Vec<Account>> {
            Ok(self.accounts.lock().unwrap().clone())
        }

        async fn activate(&self, id: &AccountId) -> subswap_core::Result<()> {
            let mut accounts = self.accounts.lock().unwrap();
            for account in accounts.iter_mut() {
                account.active = account.id == *id;
            }
            Ok(())
        }

        async fn query_quota(&self, id: &AccountId) -> subswap_core::Result<Vec<Quota>> {
            let used = if id.0 == "primary" { 95 } else { 10 };
            Ok(vec![Quota {
                provider: self.id.to_string(),
                account_id: id.clone(),
                window: QuotaWindow::FiveHour,
                used,
                limit: 100,
                reset_at: None,
                status: if used >= 95 {
                    QuotaStatus::Exhausted
                } else {
                    QuotaStatus::Ok
                },
                note: None,
            }])
        }
    }

    fn account(provider: &str, id: &str, active: bool) -> Account {
        Account {
            provider: provider.to_string(),
            id: AccountId(id.to_string()),
            label: id.to_string(),
            active,
            created_at: "2026-01-01T00:00:00Z".parse().unwrap(),
            last_used_at: None,
            priority: 100,
            extra: serde_json::Map::new(),
        }
    }

    fn broker() -> AccountBroker {
        let cursor = Arc::new(FakeProvider::new(
            "cursor",
            vec![
                account("cursor", "primary", true),
                account("cursor", "secondary", false),
            ],
        ));
        let openai = Arc::new(FakeProvider::new(
            "codex",
            vec![account("codex", "codex-primary", true)],
        ));
        let mut providers: HashMap<ProviderId, Arc<dyn Provider>> = HashMap::new();
        providers.insert(ProviderId::Cursor, cursor);
        providers.insert(ProviderId::OpenAi, openai);
        AccountBroker::new(providers)
    }

    #[tokio::test]
    async fn account_activation_is_scoped_to_requested_provider() {
        let broker = broker();
        let activation = broker
            .activate(ProviderId::Cursor, "secondary")
            .await
            .unwrap();
        assert_eq!(activation.provider, ProviderId::Cursor);
        assert_eq!(activation.account_id, "secondary");
        assert_eq!(activation.generation, 1);
        assert_eq!(broker.generation(ProviderId::Cursor).unwrap(), 1);
        assert_eq!(broker.generation(ProviderId::OpenAi).unwrap(), 0);
        assert_eq!(
            broker
                .active_account(ProviderId::Cursor)
                .await
                .unwrap()
                .unwrap()
                .id
                .0,
            "secondary"
        );
        assert_eq!(
            broker
                .active_account(ProviderId::OpenAi)
                .await
                .unwrap()
                .unwrap()
                .id
                .0,
            "codex-primary"
        );
    }

    #[tokio::test]
    async fn successful_activations_increment_only_that_provider_generation() {
        let broker = broker();
        let first = broker
            .activate(ProviderId::Cursor, "secondary")
            .await
            .unwrap();
        let second = broker
            .activate(ProviderId::Cursor, "primary")
            .await
            .unwrap();
        assert_eq!(first.generation, 1);
        assert_eq!(second.generation, 2);
        assert_eq!(broker.generation(ProviderId::Cursor).unwrap(), 2);
        assert_eq!(broker.generation(ProviderId::OpenAi).unwrap(), 0);
    }

    #[tokio::test]
    async fn autoswap_never_crosses_provider_boundary() {
        let broker = broker();
        let config = PolicyConfig {
            enabled: true,
            threshold: 0.9,
            allow_unknown: false,
            settle_grace_ms: 0,
        };

        let decision = broker
            .apply_auto_swap(ProviderId::Cursor, &config)
            .await
            .unwrap();
        assert!(matches!(
            decision,
            PolicyDecision::Swap { ref to, .. } if to.0 == "secondary"
        ));
        assert_eq!(broker.generation(ProviderId::Cursor).unwrap(), 1);
        assert_eq!(broker.generation(ProviderId::OpenAi).unwrap(), 0);
        assert_eq!(
            broker
                .active_account(ProviderId::Cursor)
                .await
                .unwrap()
                .unwrap()
                .id
                .0,
            "secondary"
        );
        assert_eq!(
            broker
                .active_account(ProviderId::OpenAi)
                .await
                .unwrap()
                .unwrap()
                .id
                .0,
            "codex-primary"
        );
    }
}
