use crate::types::ProviderId;
use serde::Deserialize;
use serde::Serialize;
use subswap_core::Quota;
use subswap_core::QuotaStatus;
use subswap_core::QuotaWindow;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedQuota {
    pub provider: ProviderId,
    pub account_id: String,
    pub window: QuotaWindow,
    pub used: u64,
    pub limit: u64,
    pub usage_ratio: Option<f64>,
    pub status: QuotaStatus,
    pub note: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum QuotaNormalizationError {
    #[error("quota provider '{0}' is not supported by the runtime harness")]
    UnsupportedProvider(String),
}

impl NormalizedQuota {
    pub fn from_subswap(quota: &Quota) -> Result<Self, QuotaNormalizationError> {
        let provider = match quota.provider.as_str() {
            "codex" | "openai" => ProviderId::OpenAi,
            "cursor" => ProviderId::Cursor,
            other => {
                return Err(QuotaNormalizationError::UnsupportedProvider(
                    other.to_string(),
                ));
            }
        };
        Ok(Self {
            provider,
            account_id: quota.account_id.0.clone(),
            window: quota.window,
            used: quota.used,
            limit: quota.limit,
            usage_ratio: quota.usage_ratio(),
            status: quota.status,
            note: quota.note.clone(),
        })
    }
}

pub fn normalize_quotas(quotas: &[Quota]) -> Result<Vec<NormalizedQuota>, QuotaNormalizationError> {
    quotas.iter().map(NormalizedQuota::from_subswap).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use subswap_core::AccountId;

    #[test]
    fn quota_normalization_preserves_provider_account_and_ratio() {
        let quota = Quota {
            provider: "cursor".into(),
            account_id: AccountId("acct-1".into()),
            window: QuotaWindow::FirstPartyModels,
            used: 25,
            limit: 100,
            reset_at: None,
            status: QuotaStatus::Ok,
            note: Some("healthy".into()),
        };
        let normalized = NormalizedQuota::from_subswap(&quota).unwrap();
        assert_eq!(normalized.provider, ProviderId::Cursor);
        assert_eq!(normalized.account_id, "acct-1");
        assert_eq!(normalized.usage_ratio, Some(0.25));
        assert_eq!(normalized.note.as_deref(), Some("healthy"));
    }

    #[test]
    fn unknown_provider_is_not_implicitly_mapped() {
        let quota = Quota {
            provider: "other".into(),
            account_id: AccountId("acct".into()),
            window: QuotaWindow::Custom,
            used: 0,
            limit: 0,
            reset_at: None,
            status: QuotaStatus::Unknown,
            note: None,
        };
        assert!(matches!(
            NormalizedQuota::from_subswap(&quota),
            Err(QuotaNormalizationError::UnsupportedProvider(provider)) if provider == "other"
        ));
    }
}
