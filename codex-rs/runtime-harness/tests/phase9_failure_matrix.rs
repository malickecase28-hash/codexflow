use codex_runtime_harness::AccountBroker;
use codex_runtime_harness::HarnessSession;
use codex_runtime_harness::HarnessEvent;
use codex_runtime_harness::ProviderId;
use codex_runtime_harness::RotationRetryDisposition;
use codex_runtime_harness::RuntimeEvent;
use codex_runtime_harness::RuntimeModelId;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use subswap_core::Account;
use subswap_core::AccountId;
use subswap_core::ClientTarget;
use subswap_core::PolicyConfig;
use subswap_core::PolicyDecision;
use subswap_core::Provider;
use subswap_core::Quota;
use subswap_core::QuotaStatus;
use subswap_core::QuotaWindow;

#[derive(Clone)]
enum QuotaBehavior {
    Healthy { used: u64 },
    Failed(String),
}

struct MatrixProvider {
    id: &'static str,
    accounts: Mutex<Vec<Account>>,
    quotas: HashMap<String, QuotaBehavior>,
    activation_failure: Option<String>,
}

impl MatrixProvider {
    fn new(
        id: &'static str,
        accounts: Vec<Account>,
        quotas: HashMap<String, QuotaBehavior>,
    ) -> Self {
        Self {
            id,
            accounts: Mutex::new(accounts),
            quotas,
            activation_failure: None,
        }
    }

    fn with_activation_failure(mut self, message: impl Into<String>) -> Self {
        self.activation_failure = Some(message.into());
        self
    }
}

#[async_trait::async_trait]
impl Provider for MatrixProvider {
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
        if let Some(message) = self.activation_failure.as_ref() {
            return Err(subswap_core::Error::Provider(message.clone()));
        }
        let mut accounts = self.accounts.lock().unwrap();
        let Some(_) = accounts.iter().find(|account| account.id == *id) else {
            return Err(subswap_core::Error::AccountNotFound {
                provider: self.id.to_string(),
                id: id.0.clone(),
            });
        };
        for account in accounts.iter_mut() {
            account.active = account.id == *id;
        }
        Ok(())
    }

    async fn query_quota(&self, id: &AccountId) -> subswap_core::Result<Vec<Quota>> {
        match self.quotas.get(&id.0) {
            Some(QuotaBehavior::Healthy { used }) => Ok(vec![Quota {
                provider: self.id.to_string(),
                account_id: id.clone(),
                window: QuotaWindow::FiveHour,
                used: *used,
                limit: 100,
                reset_at: None,
                status: if *used >= 100 {
                    QuotaStatus::Exhausted
                } else {
                    QuotaStatus::Ok
                },
                note: None,
            }]),
            Some(QuotaBehavior::Failed(message)) => {
                Err(subswap_core::Error::Provider(message.clone()))
            }
            None => Ok(Vec::new()),
        }
    }
}

fn account(provider: &str, id: &str, active: bool, priority: i32) -> Account {
    Account {
        provider: provider.to_string(),
        id: AccountId(id.to_string()),
        label: id.to_string(),
        active,
        created_at: "2026-01-01T00:00:00Z".parse().unwrap(),
        last_used_at: None,
        priority,
        extra: serde_json::Map::new(),
    }
}

fn policy() -> PolicyConfig {
    PolicyConfig {
        enabled: true,
        threshold: 0.9,
        allow_unknown: false,
        settle_grace_ms: 0,
    }
}

fn broker_with(provider: ProviderId, implementation: MatrixProvider) -> AccountBroker {
    let mut providers: HashMap<ProviderId, Arc<dyn Provider>> = HashMap::new();
    providers.insert(provider, Arc::new(implementation));
    AccountBroker::new(providers)
}

#[tokio::test]
async fn no_accounts_degrades_instead_of_fabricating_a_target() {
    let broker = broker_with(
        ProviderId::Cursor,
        MatrixProvider::new("cursor", Vec::new(), HashMap::new()),
    );

    let decision = broker
        .evaluate_auto_swap(ProviderId::Cursor, &policy())
        .await
        .unwrap();
    assert!(matches!(
        decision,
        PolicyDecision::Degraded { ref reason } if reason.contains("no accounts")
    ));
    assert_eq!(broker.generation(ProviderId::Cursor).unwrap(), 0);
}

#[tokio::test]
async fn fully_exhausted_pool_degrades_without_rotation_loop() {
    let accounts = vec![
        account("cursor", "primary", true, 100),
        account("cursor", "secondary", false, 90),
    ];
    let quotas = HashMap::from([
        ("primary".to_string(), QuotaBehavior::Healthy { used: 100 }),
        (
            "secondary".to_string(),
            QuotaBehavior::Healthy { used: 100 },
        ),
    ]);
    let broker = broker_with(
        ProviderId::Cursor,
        MatrixProvider::new("cursor", accounts, quotas),
    );

    let decision = broker
        .apply_auto_swap(ProviderId::Cursor, &policy())
        .await
        .unwrap();
    assert!(matches!(decision, PolicyDecision::Degraded { .. }));
    assert_eq!(broker.generation(ProviderId::Cursor).unwrap(), 0);
    assert_eq!(
        broker
            .active_account(ProviderId::Cursor)
            .await
            .unwrap()
            .unwrap()
            .id
            .0,
        "primary"
    );
}

#[tokio::test]
async fn expired_active_auth_can_rotate_to_known_healthy_same_provider_account() {
    let accounts = vec![
        account("cursor", "expired", true, 100),
        account("cursor", "healthy", false, 90),
    ];
    let quotas = HashMap::from([
        (
            "expired".to_string(),
            QuotaBehavior::Failed("401 Unauthorized: token expired".to_string()),
        ),
        ("healthy".to_string(), QuotaBehavior::Healthy { used: 5 }),
    ]);
    let broker = broker_with(
        ProviderId::Cursor,
        MatrixProvider::new("cursor", accounts, quotas),
    );

    let decision = broker
        .apply_auto_swap(ProviderId::Cursor, &policy())
        .await
        .unwrap();
    assert!(matches!(
        decision,
        PolicyDecision::Swap { ref to, .. } if to.0 == "healthy"
    ));
    assert_eq!(broker.generation(ProviderId::Cursor).unwrap(), 1);
    assert_eq!(
        broker
            .active_account(ProviderId::Cursor)
            .await
            .unwrap()
            .unwrap()
            .id
            .0,
        "healthy"
    );
}

#[tokio::test]
async fn malformed_credential_activation_failure_does_not_advance_generation() {
    let accounts = vec![
        account("cursor", "primary", true, 100),
        account("cursor", "malformed", false, 90),
    ];
    let broker = broker_with(
        ProviderId::Cursor,
        MatrixProvider::new("cursor", accounts, HashMap::new())
            .with_activation_failure("malformed credential blob"),
    );

    assert!(
        broker
            .activate(ProviderId::Cursor, "malformed")
            .await
            .is_err()
    );
    assert_eq!(broker.generation(ProviderId::Cursor).unwrap(), 0);
    assert_eq!(
        broker
            .active_account(ProviderId::Cursor)
            .await
            .unwrap()
            .unwrap()
            .id
            .0,
        "primary"
    );
}

#[tokio::test]
async fn repeated_explicit_rotations_remain_provider_scoped_and_monotonic() {
    let cursor_accounts = vec![
        account("cursor", "a", true, 100),
        account("cursor", "b", false, 90),
        account("cursor", "c", false, 80),
    ];
    let openai_accounts = vec![account("codex", "openai-a", true, 100)];
    let mut providers: HashMap<ProviderId, Arc<dyn Provider>> = HashMap::new();
    providers.insert(
        ProviderId::Cursor,
        Arc::new(MatrixProvider::new(
            "cursor",
            cursor_accounts,
            HashMap::new(),
        )),
    );
    providers.insert(
        ProviderId::OpenAi,
        Arc::new(MatrixProvider::new(
            "codex",
            openai_accounts,
            HashMap::new(),
        )),
    );
    let broker = AccountBroker::new(providers);

    for (expected_generation, target) in [(1, "b"), (2, "c"), (3, "a"), (4, "b")] {
        let activation = broker
            .activate(ProviderId::Cursor, target)
            .await
            .unwrap();
        assert_eq!(activation.generation, expected_generation);
        assert_eq!(activation.provider, ProviderId::Cursor);
    }

    assert_eq!(broker.generation(ProviderId::Cursor).unwrap(), 4);
    assert_eq!(broker.generation(ProviderId::OpenAi).unwrap(), 0);
    assert_eq!(
        broker
            .active_account(ProviderId::OpenAi)
            .await
            .unwrap()
            .unwrap()
            .id
            .0,
        "openai-a"
    );
}

#[test]
fn active_tool_call_rotation_never_replays_original_turn() {
    let model = RuntimeModelId::new(ProviderId::Cursor, "composer").unwrap();
    let mut session = HarnessSession::new("phase9", PathBuf::from("/tmp/project"), model);
    session.begin_turn("modify repository state");
    session.record_event(HarnessEvent::ToolRequest {
        provider: ProviderId::Cursor,
        tool_call_id: Some("tool-1".to_string()),
        payload: serde_json::json!({"path":"src/lib.rs"}),
    });
    session.observe_runtime_event(&RuntimeEvent::ToolCall {
        raw: serde_json::json!({"toolCallId":"tool-1"}),
    });

    assert_eq!(
        session.rotation_retry_disposition(),
        RotationRetryDisposition::ContinueFromRepositoryState
    );
}
