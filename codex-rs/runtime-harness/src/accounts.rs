use crate::types::ProviderId;
use std::collections::HashMap;
use std::sync::Arc;
use subswap_core::Account;
use subswap_core::AccountId;
use subswap_core::AccountRegistry;
use subswap_core::CredentialStore;
use subswap_core::FileStore;
use subswap_core::KeyringStore;
use subswap_core::Provider;
use subswap_core::Quota;
use subswap_core::paths::AppPaths;

#[derive(Debug, thiserror::Error)]
pub enum AccountBrokerError {
    #[error("subswap error: {0}")]
    Subswap(#[from] subswap_core::Error),
    #[error("provider {0} is not registered with the account broker")]
    ProviderUnavailable(ProviderId),
}

pub struct AccountBroker {
    providers: HashMap<ProviderId, Arc<dyn Provider>>,
}

impl AccountBroker {
    pub fn new(providers: HashMap<ProviderId, Arc<dyn Provider>>) -> Self {
        Self { providers }
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

    pub async fn list_accounts(
        &self,
        provider: ProviderId,
    ) -> Result<Vec<Account>, AccountBrokerError> {
        Ok(self.provider(provider)?.list_accounts().await?)
    }

    pub async fn activate(
        &self,
        provider: ProviderId,
        account_id: impl Into<String>,
    ) -> Result<(), AccountBrokerError> {
        let id = AccountId(account_id.into());
        self.provider(provider)?.activate(&id).await?;
        Ok(())
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;
    use subswap_core::ClientTarget;

    struct FakeProvider {
        accounts: Vec<Account>,
    }

    #[async_trait::async_trait]
    impl Provider for FakeProvider {
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
            Ok(self.accounts.clone())
        }

        async fn activate(&self, _id: &AccountId) -> subswap_core::Result<()> {
            Ok(())
        }

        async fn query_quota(&self, _id: &AccountId) -> subswap_core::Result<Vec<Quota>> {
            Ok(Vec::new())
        }
    }

    // Keep a compile-time assertion that the embedded broker stores trait objects
    // rather than reaching out to a CLI subprocess.
    #[allow(dead_code)]
    fn provider_future_is_send<T>(future: Pin<Box<dyn Future<Output = T> + Send>>) -> Pin<Box<dyn Future<Output = T> + Send>> {
        future
    }
}
