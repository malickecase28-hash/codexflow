use crate::capabilities::ProviderCapabilities;
use crate::types::ProviderId;
use crate::types::RuntimeModelId;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelParameterDescriptor {
    pub id: String,
    pub display_name: String,
    pub current_value: Option<Value>,
    pub allowed_values: Vec<Value>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    pub id: RuntimeModelId,
    pub display_name: String,
    pub capabilities: ProviderCapabilities,
    pub parameters: Vec<ModelParameterDescriptor>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ModelCatalogError {
    #[error("model {model} was returned for {actual}, expected {expected}")]
    ProviderMismatch {
        model: RuntimeModelId,
        expected: ProviderId,
        actual: ProviderId,
    },
    #[error("provider {provider} returned duplicate model id {model}")]
    DuplicateModel {
        provider: ProviderId,
        model: RuntimeModelId,
    },
}

/// Unified provider-aware model catalog.
///
/// The catalog owns no provider availability assumptions. Backends replace their
/// portion of the catalog with dynamically discovered descriptors, which is
/// important for Cursor where available models depend on the authenticated plan.
#[derive(Debug, Clone, Default)]
pub struct ModelCatalog {
    models: BTreeMap<String, ModelDescriptor>,
}

impl ModelCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn replace_provider(
        &mut self,
        provider: ProviderId,
        models: Vec<ModelDescriptor>,
    ) -> Result<(), ModelCatalogError> {
        let mut replacement = BTreeMap::new();
        for model in models {
            if model.id.provider != provider {
                return Err(ModelCatalogError::ProviderMismatch {
                    model: model.id.clone(),
                    expected: provider,
                    actual: model.id.provider,
                });
            }
            let key = model.id.qualified();
            if replacement.insert(key, model.clone()).is_some() {
                return Err(ModelCatalogError::DuplicateModel {
                    provider,
                    model: model.id,
                });
            }
        }

        self.models.retain(|_, model| model.id.provider != provider);
        self.models.extend(replacement);
        Ok(())
    }

    pub fn get(&self, id: &RuntimeModelId) -> Option<&ModelDescriptor> {
        self.models.get(&id.qualified())
    }

    pub fn models_for_provider(&self, provider: ProviderId) -> Vec<&ModelDescriptor> {
        self.models
            .values()
            .filter(|model| model.id.provider == provider)
            .collect()
    }

    pub fn all(&self) -> Vec<&ModelDescriptor> {
        self.models.values().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(provider: ProviderId, id: &str) -> ModelDescriptor {
        ModelDescriptor {
            id: RuntimeModelId::new(provider, id).unwrap(),
            display_name: id.to_string(),
            capabilities: ProviderCapabilities::for_provider(provider),
            parameters: Vec::new(),
            metadata: Value::Null,
        }
    }

    #[test]
    fn same_model_name_can_exist_under_different_providers() {
        let mut catalog = ModelCatalog::new();
        catalog
            .replace_provider(
                ProviderId::OpenAi,
                vec![descriptor(ProviderId::OpenAi, "gpt-5")],
            )
            .unwrap();
        catalog
            .replace_provider(
                ProviderId::Cursor,
                vec![descriptor(ProviderId::Cursor, "gpt-5")],
            )
            .unwrap();

        assert_eq!(catalog.all().len(), 2);
        assert!(
            catalog
                .get(&RuntimeModelId::new(ProviderId::OpenAi, "gpt-5").unwrap())
                .is_some()
        );
        assert!(
            catalog
                .get(&RuntimeModelId::new(ProviderId::Cursor, "gpt-5").unwrap())
                .is_some()
        );
    }

    #[test]
    fn provider_refresh_replaces_only_that_provider_models() {
        let mut catalog = ModelCatalog::new();
        catalog
            .replace_provider(
                ProviderId::OpenAi,
                vec![descriptor(ProviderId::OpenAi, "gpt-a")],
            )
            .unwrap();
        catalog
            .replace_provider(
                ProviderId::Cursor,
                vec![descriptor(ProviderId::Cursor, "old")],
            )
            .unwrap();
        catalog
            .replace_provider(
                ProviderId::Cursor,
                vec![descriptor(ProviderId::Cursor, "new")],
            )
            .unwrap();

        assert!(
            catalog
                .get(&RuntimeModelId::new(ProviderId::OpenAi, "gpt-a").unwrap())
                .is_some()
        );
        assert!(
            catalog
                .get(&RuntimeModelId::new(ProviderId::Cursor, "old").unwrap())
                .is_none()
        );
        assert!(
            catalog
                .get(&RuntimeModelId::new(ProviderId::Cursor, "new").unwrap())
                .is_some()
        );
    }

    #[test]
    fn provider_cannot_publish_model_under_another_namespace() {
        let mut catalog = ModelCatalog::new();
        let error = catalog
            .replace_provider(
                ProviderId::Cursor,
                vec![descriptor(ProviderId::OpenAi, "gpt-5")],
            )
            .unwrap_err();
        assert!(matches!(error, ModelCatalogError::ProviderMismatch { .. }));
    }
}
