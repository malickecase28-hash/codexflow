use crate::accounts::AccountBroker;
use crate::accounts::AccountBrokerError;
use crate::types::ProviderId;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use subswap_core::Account;
use subswap_core::Quota;
use subswap_core::QuotaCache;
use subswap_core::paths::AppPaths;
use tokio::sync::Mutex;
use tokio::sync::watch;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaRefreshState {
    Live,
    Cached,
    Stale { error: String },
    Failed { error: String },
}

#[derive(Debug, Clone)]
pub struct AccountQuotaSnapshot {
    pub account: Account,
    pub quotas: Vec<Quota>,
    pub state: QuotaRefreshState,
}

#[derive(Debug, Clone)]
pub struct ProviderQuotaSnapshot {
    pub provider: ProviderId,
    pub accounts: Vec<AccountQuotaSnapshot>,
}

pub trait QuotaUpdateSink: Send + Sync {
    fn publish(&self, snapshot: ProviderQuotaSnapshot);
}

impl<F> QuotaUpdateSink for F
where
    F: Fn(ProviderQuotaSnapshot) + Send + Sync,
{
    fn publish(&self, snapshot: ProviderQuotaSnapshot) {
        self(snapshot);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum QuotaServiceError {
    #[error(transparent)]
    Broker(#[from] AccountBrokerError),
    #[error("subswap quota service setup failed: {0}")]
    Subswap(#[from] subswap_core::Error),
    #[error("quota poll interval must be greater than zero")]
    InvalidPollInterval,
}

pub struct QuotaService {
    broker: Arc<AccountBroker>,
    cache: Mutex<QuotaCache>,
    cache_path: PathBuf,
}

impl QuotaService {
    pub fn from_default_paths(broker: Arc<AccountBroker>) -> Result<Self, QuotaServiceError> {
        let cache_path = AppPaths::resolve()?.quota_cache_file();
        Ok(Self::new(broker, cache_path))
    }

    pub fn new(broker: Arc<AccountBroker>, cache_path: PathBuf) -> Self {
        let cache = QuotaCache::load(&cache_path);
        Self {
            broker,
            cache: Mutex::new(cache),
            cache_path,
        }
    }

    /// Refresh one provider without invoking the standalone subswap UI/daemon.
    ///
    /// Fresh values are read from subswap's shared cache. Failed accounts honor
    /// the cache's exponential backoff and reuse still-valid stale quota windows
    /// where possible. Authentication failures are handled by QuotaCache itself,
    /// which intentionally invalidates stale successful credentials/usage data.
    pub async fn refresh_provider(
        &self,
        provider: ProviderId,
    ) -> Result<ProviderQuotaSnapshot, QuotaServiceError> {
        let settings = subswap_core::settings::current();
        let min_refresh = Duration::from_millis(settings.quota.min_refresh_interval_ms);
        let backoff_cap = Duration::from_millis(settings.quota.failure_backoff_max_ms);
        let accounts = self.broker.list_accounts(provider).await?;
        let mut snapshots = Vec::with_capacity(accounts.len());

        for account in accounts {
            let provider_key = provider.subswap_id();
            let account_key = account.id.0.as_str();

            let cached = {
                let cache = self.cache.lock().await;
                if let Some(entry) = cache.fresh(provider_key, account_key, min_refresh) {
                    Some((entry.quotas, QuotaRefreshState::Cached))
                } else if let Some(failure) = cache.in_failure_backoff(
                    provider_key,
                    account_key,
                    min_refresh,
                    backoff_cap,
                ) {
                    let error = failure.error.clone();
                    match cache.get(provider_key, account_key) {
                        Some(entry) => Some((entry.quotas, QuotaRefreshState::Stale { error })),
                        None => Some((Vec::new(), QuotaRefreshState::Failed { error })),
                    }
                } else {
                    None
                }
            };

            if let Some((quotas, state)) = cached {
                snapshots.push(AccountQuotaSnapshot {
                    account,
                    quotas,
                    state,
                });
                continue;
            }

            match self
                .broker
                .query_quota(provider, account.id.0.clone())
                .await
            {
                Ok(quotas) => {
                    let mut cache = self.cache.lock().await;
                    cache.set(provider_key, account_key, quotas.clone());
                    cache.save(&self.cache_path);
                    drop(cache);
                    snapshots.push(AccountQuotaSnapshot {
                        account,
                        quotas,
                        state: QuotaRefreshState::Live,
                    });
                }
                Err(error) => {
                    let error = error.to_string();
                    let stale = {
                        let mut cache = self.cache.lock().await;
                        cache.record_failure(provider_key, account_key, &error);
                        let stale = cache.get(provider_key, account_key).map(|entry| entry.quotas);
                        cache.save(&self.cache_path);
                        stale
                    };
                    let (quotas, state) = match stale {
                        Some(quotas) => (quotas, QuotaRefreshState::Stale { error }),
                        None => (Vec::new(), QuotaRefreshState::Failed { error }),
                    };
                    snapshots.push(AccountQuotaSnapshot {
                        account,
                        quotas,
                        state,
                    });
                }
            }
        }

        Ok(ProviderQuotaSnapshot {
            provider,
            accounts: snapshots,
        })
    }

    pub async fn refresh_all(&self) -> Result<Vec<ProviderQuotaSnapshot>, QuotaServiceError> {
        let openai = self.refresh_provider(ProviderId::OpenAi).await?;
        let cursor = self.refresh_provider(ProviderId::Cursor).await?;
        Ok(vec![openai, cursor])
    }

    /// Periodically publish quota state until the caller requests shutdown.
    /// The caller owns the task lifecycle, which keeps the service independent
    /// from the Codex TUI runtime and makes it testable in isolation.
    pub async fn run_provider(
        &self,
        provider: ProviderId,
        poll_interval: Duration,
        mut shutdown: watch::Receiver<bool>,
        sink: Arc<dyn QuotaUpdateSink>,
    ) -> Result<(), QuotaServiceError> {
        if poll_interval.is_zero() {
            return Err(QuotaServiceError::InvalidPollInterval);
        }

        loop {
            sink.publish(self.refresh_provider(provider).await?);
            tokio::select! {
                _ = tokio::time::sleep(poll_interval) => {}
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return Ok(());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use subswap_core::AccountId;
    use subswap_core::ClientTarget;
    use subswap_core::Provider;
    use subswap_core::QuotaStatus;
    use subswap_core::QuotaWindow;

    struct CountingProvider {
        queries: Arc<AtomicUsize>,
        account: Account,
    }

    #[async_trait::async_trait]
    impl Provider for CountingProvider {
        fn id(&self) -> &'static str {
            "cursor"
        }

        fn display_name(&self) -> &'static str {
            "Cursor"
        }

        fn client_targets(&self) -> Vec<ClientTarget> {
            Vec::new()
        }

        async fn list_accounts(&self) -> subswap_core::Result<Vec<Account>> {
            Ok(vec![self.account.clone()])
        }

        async fn activate(&self, _id: &AccountId) -> subswap_core::Result<()> {
            Ok(())
        }

        async fn query_quota(&self, id: &AccountId) -> subswap_core::Result<Vec<Quota>> {
            self.queries.fetch_add(1, Ordering::SeqCst);
            Ok(vec![Quota {
                provider: "cursor".to_string(),
                account_id: id.clone(),
                window: QuotaWindow::FirstPartyModels,
                used: 10,
                limit: 100,
                reset_at: None,
                status: QuotaStatus::Ok,
                note: None,
            }])
        }
    }

    fn test_account() -> Account {
        Account {
            provider: "cursor".to_string(),
            id: AccountId("cursor-account".to_string()),
            label: "Cursor account".to_string(),
            active: true,
            created_at: "2026-01-01T00:00:00Z".parse().unwrap(),
            last_used_at: None,
            priority: 100,
            extra: serde_json::Map::new(),
        }
    }

    #[tokio::test]
    async fn fresh_shared_cache_avoids_duplicate_provider_queries() {
        let queries = Arc::new(AtomicUsize::new(0));
        let provider: Arc<dyn Provider> = Arc::new(CountingProvider {
            queries: Arc::clone(&queries),
            account: test_account(),
        });
        let mut providers = HashMap::new();
        providers.insert(ProviderId::Cursor, provider);
        let broker = Arc::new(AccountBroker::new(providers));
        let temp = tempfile::tempdir().unwrap();
        let service = QuotaService::new(broker, temp.path().join("quota-cache.json"));

        let first = service.refresh_provider(ProviderId::Cursor).await.unwrap();
        let second = service.refresh_provider(ProviderId::Cursor).await.unwrap();

        assert_eq!(queries.load(Ordering::SeqCst), 1);
        assert!(matches!(first.accounts[0].state, QuotaRefreshState::Live));
        assert!(matches!(second.accounts[0].state, QuotaRefreshState::Cached));
        assert_eq!(second.accounts[0].quotas[0].used, 10);
    }

    #[tokio::test]
    async fn zero_poll_interval_is_rejected() {
        let providers = HashMap::new();
        let broker = Arc::new(AccountBroker::new(providers));
        let temp = tempfile::tempdir().unwrap();
        let service = QuotaService::new(broker, temp.path().join("quota-cache.json"));
        let (_tx, rx) = watch::channel(false);
        let sink: Arc<dyn QuotaUpdateSink> = Arc::new(|_snapshot: ProviderQuotaSnapshot| {});

        let error = service
            .run_provider(ProviderId::Cursor, Duration::ZERO, rx, sink)
            .await
            .unwrap_err();
        assert!(matches!(error, QuotaServiceError::InvalidPollInterval));
    }
}
