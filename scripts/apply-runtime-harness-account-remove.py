from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one patch anchor, found {count}")
    path.write_text(text.replace(old, new, 1))


accounts = Path("codex-rs/runtime-harness/src/accounts.rs")
controller = Path("codex-rs/runtime-harness/src/controller.rs")

replace_once(
    accounts,
    """    #[error("provider {0} does not support importing the active native login")]
    ActiveImportUnavailable(ProviderId),
}""",
    """    #[error("provider {0} does not support importing the active native login")]
    ActiveImportUnavailable(ProviderId),
    #[error("provider {0} account removal storage is unavailable")]
    RemovalUnavailable(ProviderId),
    #[error("cannot remove active {provider} account {account_id}; activate another account first")]
    ActiveAccountRemoval {
        provider: ProviderId,
        account_id: String,
    },
    #[error("failed to remove {provider} account {account_id}: {message}; rollback: {rollback}")]
    AccountRemovalFailed {
        provider: ProviderId,
        account_id: String,
        message: String,
        rollback: String,
    },
}""",
)

replace_once(
    accounts,
    """pub struct AccountBroker {
    providers: HashMap<ProviderId, Arc<dyn Provider>>,
    importers: HashMap<ProviderId, Arc<dyn ActiveAccountImporter>>,
    states: HashMap<ProviderId, Arc<ProviderAccountState>>,
}

impl AccountBroker {
    pub fn new(providers: HashMap<ProviderId, Arc<dyn Provider>>) -> Self {
        Self::with_importers(providers, HashMap::new())
    }

    fn with_importers(
        providers: HashMap<ProviderId, Arc<dyn Provider>>,
        importers: HashMap<ProviderId, Arc<dyn ActiveAccountImporter>>,
    ) -> Self {
        let states = providers
            .keys()
            .copied()
            .map(|provider| (provider, Arc::new(ProviderAccountState::new())))
            .collect();
        Self {
            providers,
            importers,
            states,
        }
    }
""",
    """struct AccountRemovalStorage {
    registry: Arc<AccountRegistry>,
    store: Arc<dyn CredentialStore>,
}

pub struct AccountBroker {
    providers: HashMap<ProviderId, Arc<dyn Provider>>,
    importers: HashMap<ProviderId, Arc<dyn ActiveAccountImporter>>,
    states: HashMap<ProviderId, Arc<ProviderAccountState>>,
    removal_storage: Option<AccountRemovalStorage>,
}

impl AccountBroker {
    pub fn new(providers: HashMap<ProviderId, Arc<dyn Provider>>) -> Self {
        Self::with_components(providers, HashMap::new(), None)
    }

    fn with_importers(
        providers: HashMap<ProviderId, Arc<dyn Provider>>,
        importers: HashMap<ProviderId, Arc<dyn ActiveAccountImporter>>,
    ) -> Self {
        Self::with_components(providers, importers, None)
    }

    fn with_components(
        providers: HashMap<ProviderId, Arc<dyn Provider>>,
        importers: HashMap<ProviderId, Arc<dyn ActiveAccountImporter>>,
        removal_storage: Option<AccountRemovalStorage>,
    ) -> Self {
        let states = providers
            .keys()
            .copied()
            .map(|provider| (provider, Arc::new(ProviderAccountState::new())))
            .collect();
        Self {
            providers,
            importers,
            states,
            removal_storage,
        }
    }
""",
)

replace_once(
    accounts,
    """        Ok(Self::with_importers(providers, importers))
    }
""",
    """        Ok(Self::with_components(
            providers,
            importers,
            Some(AccountRemovalStorage { registry, store }),
        ))
    }
""",
)

replace_once(
    accounts,
    """    pub async fn query_quota(
        &self,
        provider: ProviderId,
""",
    """    /// Remove a non-active account from embedded subswap metadata and credentials.
    ///
    /// Active accounts are deliberately protected: the caller must activate a
    /// replacement first so the native client and runtime cannot be left signed
    /// into credentials that the registry no longer knows about. Credential
    /// values are snapshotted and restored if cleanup or registry persistence
    /// fails, keeping removal transactional from the harness' point of view.
    pub async fn remove_account(
        &self,
        provider: ProviderId,
        account_id: impl Into<String>,
    ) -> Result<u64, AccountBrokerError> {
        let account_id = account_id.into();
        let id = AccountId(account_id.clone());
        let state = Arc::clone(self.state(provider)?);
        let storage = self
            .removal_storage
            .as_ref()
            .ok_or(AccountBrokerError::RemovalUnavailable(provider))?;
        let _permit = Arc::clone(&state.swap_permit)
            .acquire_owned()
            .await
            .map_err(|_| AccountBrokerError::SwapCoordinatorUnavailable(provider))?;

        let account = self
            .provider(provider)?
            .list_accounts()
            .await?
            .into_iter()
            .find(|account| account.id == id)
            .ok_or_else(|| {
                AccountBrokerError::Subswap(subswap_core::Error::AccountNotFound {
                    provider: provider.subswap_id().to_string(),
                    id: account_id.clone(),
                })
            })?;
        if account.active {
            return Err(AccountBrokerError::ActiveAccountRemoval {
                provider,
                account_id,
            });
        }

        let provider_id = provider.subswap_id();
        let fields: &[&str] = match provider {
            ProviderId::OpenAi => &["auth_json"],
            ProviderId::Cursor => &["blob"],
        };
        let mut credential_snapshot = Vec::with_capacity(fields.len());
        for field in fields {
            credential_snapshot.push((
                *field,
                storage.store.get(provider_id, &account_id, field)?,
            ));
        }

        for (field, _) in &credential_snapshot {
            if let Err(error) = storage.store.delete(provider_id, &account_id, field) {
                let rollback = restore_credentials(
                    storage,
                    provider_id,
                    &account_id,
                    &credential_snapshot,
                );
                return Err(AccountBrokerError::AccountRemovalFailed {
                    provider,
                    account_id,
                    message: format!("credential cleanup failed for {field}: {error}"),
                    rollback,
                });
            }
        }

        if let Err(error) = storage.registry.remove(provider_id, &id) {
            let rollback = restore_credentials(
                storage,
                provider_id,
                &account_id,
                &credential_snapshot,
            );
            return Err(AccountBrokerError::AccountRemovalFailed {
                provider,
                account_id,
                message: format!("registry removal failed: {error}"),
                rollback,
            });
        }

        Ok(state.generation.fetch_add(1, Ordering::AcqRel) + 1)
    }

    pub async fn query_quota(
        &self,
        provider: ProviderId,
""",
)

replace_once(
    accounts,
    """}

#[cfg(test)]
mod tests {
""",
    """}

fn restore_credentials(
    storage: &AccountRemovalStorage,
    provider: &str,
    account_id: &str,
    snapshot: &[(&str, Option<String>)],
) -> String {
    let mut failures = Vec::new();
    for (field, value) in snapshot {
        if let Some(value) = value
            && let Err(error) = storage.store.set(provider, account_id, field, value)
        {
            failures.push(format!("{field}: {error}"));
        }
    }
    if failures.is_empty() {
        "restored credential snapshot".to_string()
    } else {
        format!("credential restore failures: {}", failures.join(", "))
    }
}

#[cfg(test)]
mod tests {
""",
)

replace_once(
    accounts,
    """    #[tokio::test]
    async fn autoswap_never_crosses_provider_boundary() {
""",
    """    #[tokio::test]
    async fn removing_inactive_account_cleans_registry_and_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let registry = Arc::new(AccountRegistry::new(directory.path().join("registry.toml")));
        let store: Arc<dyn CredentialStore> =
            Arc::new(FileStore::new(directory.path().join("credentials.json")));
        let primary = account("cursor", "primary", true);
        let secondary = account("cursor", "secondary", false);
        registry.upsert(primary.clone()).unwrap();
        registry.upsert(secondary.clone()).unwrap();
        store.set("cursor", "secondary", "blob", "secret").unwrap();

        let cursor = Arc::new(FakeProvider::new(
            "cursor",
            vec![primary, secondary],
        ));
        let mut providers: HashMap<ProviderId, Arc<dyn Provider>> = HashMap::new();
        providers.insert(ProviderId::Cursor, cursor);
        let broker = AccountBroker::with_components(
            providers,
            HashMap::new(),
            Some(AccountRemovalStorage {
                registry: Arc::clone(&registry),
                store: Arc::clone(&store),
            }),
        );

        let generation = broker
            .remove_account(ProviderId::Cursor, "secondary")
            .await
            .unwrap();
        assert_eq!(generation, 1);
        assert_eq!(broker.generation(ProviderId::Cursor).unwrap(), 1);
        assert!(registry.find("cursor", &AccountId("secondary".into())).unwrap().is_none());
        assert_eq!(store.get("cursor", "secondary", "blob").unwrap(), None);
    }

    #[tokio::test]
    async fn removing_active_account_is_rejected_without_mutation() {
        let directory = tempfile::tempdir().unwrap();
        let registry = Arc::new(AccountRegistry::new(directory.path().join("registry.toml")));
        let store: Arc<dyn CredentialStore> =
            Arc::new(FileStore::new(directory.path().join("credentials.json")));
        let primary = account("cursor", "primary", true);
        registry.upsert(primary.clone()).unwrap();
        store.set("cursor", "primary", "blob", "secret").unwrap();

        let cursor = Arc::new(FakeProvider::new("cursor", vec![primary]));
        let mut providers: HashMap<ProviderId, Arc<dyn Provider>> = HashMap::new();
        providers.insert(ProviderId::Cursor, cursor);
        let broker = AccountBroker::with_components(
            providers,
            HashMap::new(),
            Some(AccountRemovalStorage {
                registry: Arc::clone(&registry),
                store: Arc::clone(&store),
            }),
        );

        let error = broker
            .remove_account(ProviderId::Cursor, "primary")
            .await
            .unwrap_err();
        assert!(matches!(error, AccountBrokerError::ActiveAccountRemoval { .. }));
        assert_eq!(broker.generation(ProviderId::Cursor).unwrap(), 0);
        assert!(registry.find("cursor", &AccountId("primary".into())).unwrap().is_some());
        assert_eq!(
            store.get("cursor", "primary", "blob").unwrap().as_deref(),
            Some("secret")
        );
    }

    #[tokio::test]
    async fn autoswap_never_crosses_provider_boundary() {
""",
)

replace_once(
    controller,
    """    pub async fn quota_snapshot(
        &self,
        provider: ProviderId,
""",
    """    /// Remove a non-active account while serializing against provider/model
    /// transitions. Active-account removal is rejected by the broker so the
    /// running native client can never be orphaned from its credential metadata.
    pub async fn remove_account(
        &self,
        provider: ProviderId,
        account_id: impl Into<String>,
    ) -> Result<u64, RuntimeHarnessError> {
        let _transition = self.acquire_transition().await?;
        Ok(self.broker.remove_account(provider, account_id).await?)
    }

    pub async fn quota_snapshot(
        &self,
        provider: ProviderId,
""",
)
