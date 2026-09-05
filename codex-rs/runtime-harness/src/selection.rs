use crate::types::ProviderId;
use crate::types::RuntimeModelId;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSelection {
    pub model: RuntimeModelId,
    pub account_id: Option<String>,
}

impl RuntimeSelection {
    pub fn new(model: RuntimeModelId) -> Self {
        Self {
            model,
            account_id: None,
        }
    }

    pub const fn provider(&self) -> ProviderId {
        self.model.provider
    }

    /// Select a model without allowing an implicit provider transition.
    pub fn select_model_within_provider(
        &mut self,
        model: RuntimeModelId,
    ) -> Result<(), RuntimeSelectionError> {
        if model.provider != self.provider() {
            return Err(
                RuntimeSelectionError::ProviderTransitionRequiresExplicitSelection {
                    current: self.provider(),
                    requested: model.provider,
                },
            );
        }
        self.model = model;
        Ok(())
    }

    /// Explicit provider selection resets provider-scoped account state.
    pub fn select_provider(&mut self, model: RuntimeModelId) {
        if model.provider != self.provider() {
            self.account_id = None;
        }
        self.model = model;
    }

    /// Account activation is provider-scoped and can never switch providers.
    pub fn select_account(
        &mut self,
        provider: ProviderId,
        account_id: impl Into<String>,
    ) -> Result<(), RuntimeSelectionError> {
        if provider != self.provider() {
            return Err(RuntimeSelectionError::AccountProviderMismatch {
                active: self.provider(),
                requested: provider,
            });
        }
        let account_id = account_id.into();
        if account_id.trim().is_empty() {
            return Err(RuntimeSelectionError::EmptyAccountId);
        }
        self.account_id = Some(account_id);
        Ok(())
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RuntimeSelectionError {
    #[error(
        "model provider transition from {current} to {requested} requires explicit provider selection"
    )]
    ProviderTransitionRequiresExplicitSelection {
        current: ProviderId,
        requested: ProviderId,
    },
    #[error("account belongs to {requested}, but active runtime provider is {active}")]
    AccountProviderMismatch {
        active: ProviderId,
        requested: ProviderId,
    },
    #[error("account id cannot be empty")]
    EmptyAccountId,
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeSelectionStoreError {
    #[error("runtime selection I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("runtime selection JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct RuntimeSelectionStore {
    path: PathBuf,
}

impl RuntimeSelectionStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Result<Option<RuntimeSelection>, RuntimeSelectionStoreError> {
        let raw = match fs::read(&self.path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        Ok(Some(serde_json::from_slice(&raw)?))
    }

    pub fn save(&self, selection: &RuntimeSelection) -> Result<(), RuntimeSelectionStoreError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let encoded = serde_json::to_vec_pretty(selection)?;
        let temporary = temporary_path(&self.path);
        fs::write(&temporary, encoded)?;
        fs::rename(temporary, &self.path)?;
        Ok(())
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}.tmp", std::process::id()));
    PathBuf::from(temporary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(provider: ProviderId, name: &str) -> RuntimeModelId {
        RuntimeModelId::new(provider, name).unwrap()
    }

    #[test]
    fn model_selection_cannot_implicitly_change_provider() {
        let mut selection = RuntimeSelection::new(model(ProviderId::OpenAi, "gpt-5"));
        let error = selection
            .select_model_within_provider(model(ProviderId::Cursor, "auto"))
            .unwrap_err();
        assert!(matches!(
            error,
            RuntimeSelectionError::ProviderTransitionRequiresExplicitSelection { .. }
        ));
        assert_eq!(selection.provider(), ProviderId::OpenAi);
    }

    #[test]
    fn account_selection_never_changes_provider() {
        let mut selection = RuntimeSelection::new(model(ProviderId::Cursor, "auto"));
        selection
            .select_account(ProviderId::Cursor, "cursor-account")
            .unwrap();
        assert_eq!(selection.provider(), ProviderId::Cursor);
        assert_eq!(selection.account_id.as_deref(), Some("cursor-account"));

        assert!(
            selection
                .select_account(ProviderId::OpenAi, "codex-account")
                .is_err()
        );
        assert_eq!(selection.provider(), ProviderId::Cursor);
    }

    #[test]
    fn explicit_provider_change_clears_provider_scoped_account() {
        let mut selection = RuntimeSelection::new(model(ProviderId::Cursor, "auto"));
        selection
            .select_account(ProviderId::Cursor, "cursor-account")
            .unwrap();
        selection.select_provider(model(ProviderId::OpenAi, "gpt-5"));
        assert_eq!(selection.provider(), ProviderId::OpenAi);
        assert_eq!(selection.account_id, None);
    }

    #[test]
    fn store_round_trips_selection_atomically() {
        let directory = std::env::temp_dir().join(format!(
            "codex-runtime-selection-test-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("thread")
        ));
        let path = directory.join("selection.json");
        let store = RuntimeSelectionStore::new(path);
        let mut selection = RuntimeSelection::new(model(ProviderId::Cursor, "auto"));
        selection
            .select_account(ProviderId::Cursor, "cursor-account")
            .unwrap();
        store.save(&selection).unwrap();
        assert_eq!(store.load().unwrap(), Some(selection));
        let _ = fs::remove_dir_all(directory);
    }
}
