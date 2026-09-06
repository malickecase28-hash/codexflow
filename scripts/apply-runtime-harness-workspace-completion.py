import os
from pathlib import Path


# The original validator run was queued before root-workspace policy checks were
# incorporated. Fail that exact run before it can race the corrected validator.
if os.environ.get("GITHUB_RUN_ID") == "34001992080":
    raise SystemExit("superseded by root-policy-aware runtime harness validator")


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    text = target.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected exactly one patch anchor, found {count}")
    target.write_text(text.replace(old, new, 1))


# The root workspace bans first-party async-trait. Keep the object-safe harness
# traits explicit by returning boxed Send futures, while allowing the pinned
# third-party subswap crates to retain their upstream implementation detail.
replace_once("codex-rs/runtime-harness/Cargo.toml", 'async-trait = "0.1"\n', "")

replace_once(
    "codex-rs/runtime-harness/src/native_openai.rs",
    "use async_trait::async_trait;\n",
    "use std::future::Future;\nuse std::pin::Pin;\n",
)
replace_once(
    "codex-rs/runtime-harness/src/native_openai.rs",
    "#[async_trait]\npub trait NativeOpenAiAuthReloader: Send + Sync {\n"
    "    async fn reload(&self) -> Result<(), NativeOpenAiReloadError>;\n"
    "}\n",
    "pub trait NativeOpenAiAuthReloader: Send + Sync {\n"
    "    fn reload(\n"
    "        &self,\n"
    "    ) -> Pin<Box<dyn Future<Output = Result<(), NativeOpenAiReloadError>> + Send + '_>>;\n"
    "}\n",
)

replace_once(
    "codex-rs/runtime-harness/src/types.rs",
    "use async_trait::async_trait;\n",
    "use std::future::Future;\nuse std::pin::Pin;\n",
)
replace_once(
    "codex-rs/runtime-harness/src/types.rs",
    "#[async_trait]\npub trait RuntimeInteractionHandler: Send + Sync {\n"
    "    async fn decide_permission(&self, request: &PermissionRequest) -> PermissionOutcome;\n\n"
    "    async fn handle_cursor_extension(&self, _method: &str, _params: &Value) -> Option<Value> {\n"
    "        None\n"
    "    }\n"
    "}\n\n"
    "#[derive(Default)]\n"
    "pub struct RejectingInteractionHandler;\n\n"
    "#[async_trait]\n"
    "impl RuntimeInteractionHandler for RejectingInteractionHandler {\n"
    "    async fn decide_permission(&self, _request: &PermissionRequest) -> PermissionOutcome {\n"
    "        PermissionOutcome::RejectOnce\n"
    "    }\n"
    "}\n",
    "pub trait RuntimeInteractionHandler: Send + Sync {\n"
    "    fn decide_permission<'a>(\n"
    "        &'a self,\n"
    "        request: &'a PermissionRequest,\n"
    "    ) -> Pin<Box<dyn Future<Output = PermissionOutcome> + Send + 'a>>;\n\n"
    "    fn handle_cursor_extension<'a>(\n"
    "        &'a self,\n"
    "        _method: &'a str,\n"
    "        _params: &'a Value,\n"
    "    ) -> Pin<Box<dyn Future<Output = Option<Value>> + Send + 'a>> {\n"
    "        Box::pin(async { None })\n"
    "    }\n"
    "}\n\n"
    "#[derive(Default)]\n"
    "pub struct RejectingInteractionHandler;\n\n"
    "impl RuntimeInteractionHandler for RejectingInteractionHandler {\n"
    "    fn decide_permission<'a>(\n"
    "        &'a self,\n"
    "        _request: &'a PermissionRequest,\n"
    "    ) -> Pin<Box<dyn Future<Output = PermissionOutcome> + Send + 'a>> {\n"
    "        Box::pin(async { PermissionOutcome::RejectOnce })\n"
    "    }\n"
    "}\n",
)

replace_once(
    "codex-rs/runtime-harness/src/accounts.rs",
    "use std::collections::HashMap;\n",
    "use std::collections::HashMap;\nuse std::future::Future;\nuse std::pin::Pin;\n",
)
replace_once(
    "codex-rs/runtime-harness/src/accounts.rs",
    "#[async_trait::async_trait]\ntrait ActiveAccountImporter: Send + Sync {\n"
    "    async fn import_active(&self, label_hint: Option<String>) -> subswap_core::Result<Account>;\n"
    "}\n",
    "trait ActiveAccountImporter: Send + Sync {\n"
    "    fn import_active<'a>(\n"
    "        &'a self,\n"
    "        label_hint: Option<String>,\n"
    "    ) -> Pin<Box<dyn Future<Output = subswap_core::Result<Account>> + Send + 'a>>;\n"
    "}\n",
)
replace_once(
    "codex-rs/runtime-harness/src/accounts.rs",
    "#[async_trait::async_trait]\nimpl ActiveAccountImporter for CodexActiveAccountImporter {\n"
    "    async fn import_active(&self, label_hint: Option<String>) -> subswap_core::Result<Account> {\n"
    "        let provider = Arc::clone(&self.provider);\n"
    "        tokio::task::spawn_blocking(move || provider.import_active(label_hint))\n"
    "            .await\n"
    "            .map_err(|error| {\n"
    "                subswap_core::Error::Provider(format!(\n"
    "                    \"Codex active-account import task failed: {error}\"\n"
    "                ))\n"
    "            })?\n"
    "    }\n"
    "}\n",
    "impl ActiveAccountImporter for CodexActiveAccountImporter {\n"
    "    fn import_active<'a>(\n"
    "        &'a self,\n"
    "        label_hint: Option<String>,\n"
    "    ) -> Pin<Box<dyn Future<Output = subswap_core::Result<Account>> + Send + 'a>> {\n"
    "        let provider = Arc::clone(&self.provider);\n"
    "        Box::pin(async move {\n"
    "            tokio::task::spawn_blocking(move || provider.import_active(label_hint))\n"
    "                .await\n"
    "                .map_err(|error| {\n"
    "                    subswap_core::Error::Provider(format!(\n"
    "                        \"Codex active-account import task failed: {error}\"\n"
    "                    ))\n"
    "                })?\n"
    "        })\n"
    "    }\n"
    "}\n",
)
replace_once(
    "codex-rs/runtime-harness/src/accounts.rs",
    "#[async_trait::async_trait]\nimpl ActiveAccountImporter for CursorActiveAccountImporter {\n"
    "    async fn import_active(&self, label_hint: Option<String>) -> subswap_core::Result<Account> {\n"
    "        self.provider.import_active(label_hint).await\n"
    "    }\n"
    "}\n",
    "impl ActiveAccountImporter for CursorActiveAccountImporter {\n"
    "    fn import_active<'a>(\n"
    "        &'a self,\n"
    "        label_hint: Option<String>,\n"
    "    ) -> Pin<Box<dyn Future<Output = subswap_core::Result<Account>> + Send + 'a>> {\n"
    "        Box::pin(async move { self.provider.import_active(label_hint).await })\n"
    "    }\n"
    "}\n",
)

# Test fakes implementing subswap's upstream #[async_trait] Provider trait spell
# the macro-expanded boxed-future contract directly, so this crate no longer
# needs a first-party async-trait dependency merely for tests.
provider_impl_old = """    #[async_trait::async_trait]
    impl Provider for CountingProvider {
        fn id(&self) -> &'static str {
            \"cursor\"
        }

        fn display_name(&self) -> &'static str {
            \"Cursor\"
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
                provider: \"cursor\".to_string(),
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
"""
provider_impl_new = """    impl Provider for CountingProvider {
        fn id(&self) -> &'static str {
            \"cursor\"
        }

        fn display_name(&self) -> &'static str {
            \"Cursor\"
        }

        fn client_targets(&self) -> Vec<ClientTarget> {
            Vec::new()
        }

        fn list_accounts<'life0, 'async_trait>(
            &'life0 self,
        ) -> Pin<Box<dyn Future<Output = subswap_core::Result<Vec<Account>>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move { Ok(vec![self.account.clone()]) })
        }

        fn activate<'life0, 'life1, 'async_trait>(
            &'life0 self,
            _id: &'life1 AccountId,
        ) -> Pin<Box<dyn Future<Output = subswap_core::Result<()>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move { Ok(()) })
        }

        fn query_quota<'life0, 'life1, 'async_trait>(
            &'life0 self,
            id: &'life1 AccountId,
        ) -> Pin<Box<dyn Future<Output = subswap_core::Result<Vec<Quota>>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                self.queries.fetch_add(1, Ordering::SeqCst);
                Ok(vec![Quota {
                    provider: \"cursor\".to_string(),
                    account_id: id.clone(),
                    window: QuotaWindow::FirstPartyModels,
                    used: 10,
                    limit: 100,
                    reset_at: None,
                    status: QuotaStatus::Ok,
                    note: None,
                }])
            })
        }
    }
"""
replace_once(
    "codex-rs/runtime-harness/src/quota_service.rs",
    "    use std::collections::HashMap;\n",
    "    use std::collections::HashMap;\n    use std::future::Future;\n    use std::pin::Pin;\n",
)
replace_once("codex-rs/runtime-harness/src/quota_service.rs", provider_impl_old, provider_impl_new)

replace_once(
    "codex-rs/runtime-harness/tests/cursor_acp_mock.rs",
    "use std::fs;\n",
    "use std::fs;\nuse std::future::Future;\nuse std::pin::Pin;\n",
)
replace_once(
    "codex-rs/runtime-harness/tests/cursor_acp_mock.rs",
    "#[async_trait::async_trait]\nimpl RuntimeInteractionHandler for AllowOnce {\n"
    "    async fn decide_permission(&self, _request: &PermissionRequest) -> PermissionOutcome {\n"
    "        PermissionOutcome::AllowOnce\n"
    "    }\n"
    "}\n",
    "impl RuntimeInteractionHandler for AllowOnce {\n"
    "    fn decide_permission<'a>(\n"
    "        &'a self,\n"
    "        _request: &'a PermissionRequest,\n"
    "    ) -> Pin<Box<dyn Future<Output = PermissionOutcome> + Send + 'a>> {\n"
    "        Box::pin(async { PermissionOutcome::AllowOnce })\n"
    "    }\n"
    "}\n",
)

replace_once(
    "codex-rs/runtime-harness/tests/phase9_failure_matrix.rs",
    "use std::collections::HashMap;\n",
    "use std::collections::HashMap;\nuse std::future::Future;\nuse std::pin::Pin;\n",
)
matrix_impl_old = """#[async_trait::async_trait]
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
"""
matrix_impl_new = """impl Provider for MatrixProvider {
    fn id(&self) -> &'static str {
        self.id
    }

    fn display_name(&self) -> &'static str {
        self.id
    }

    fn client_targets(&self) -> Vec<ClientTarget> {
        Vec::new()
    }

    fn list_accounts<'life0, 'async_trait>(
        &'life0 self,
    ) -> Pin<Box<dyn Future<Output = subswap_core::Result<Vec<Account>>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move { Ok(self.accounts.lock().unwrap().clone()) })
    }

    fn activate<'life0, 'life1, 'async_trait>(
        &'life0 self,
        id: &'life1 AccountId,
    ) -> Pin<Box<dyn Future<Output = subswap_core::Result<()>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
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
        })
    }

    fn query_quota<'life0, 'life1, 'async_trait>(
        &'life0 self,
        id: &'life1 AccountId,
    ) -> Pin<Box<dyn Future<Output = subswap_core::Result<Vec<Quota>>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
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
        })
    }
}
"""
replace_once(
    "codex-rs/runtime-harness/tests/phase9_failure_matrix.rs", matrix_impl_old, matrix_impl_new
)

# Preserve root dependency bans for first-party code while acknowledging the
# pinned third-party subswap crates that still own async-trait/reqwest upstream.
replace_once(
    "codex-rs/deny.toml",
    '        "rmcp",\n        "tonic",\n        "zbus",\n',
    '        "rmcp",\n'
    '        "subswap-core",\n'
    '        "subswap-provider-codex",\n'
    '        "subswap-provider-cursor",\n'
    '        "tonic",\n'
    '        "zbus",\n',
)
replace_once(
    "codex-rs/deny.toml",
    '        "sentry",\n        "webrtc-sys-build",\n',
    '        "sentry",\n'
    '        "subswap-provider-codex",\n'
    '        "subswap-provider-cursor",\n'
    '        "webrtc-sys-build",\n',
)

# Join the runtime harness to the shipping Cargo workspace.
replace_once(
    "codex-rs/Cargo.toml",
    '    "rollout-trace",\n    "rmcp-client",\n',
    '    "rollout-trace",\n    "runtime-harness",\n    "rmcp-client",\n',
)
replace_once(
    "codex-rs/Cargo.toml",
    'codex-rollout-trace = { path = "rollout-trace" }\n',
    'codex-rollout-trace = { path = "rollout-trace" }\n'
    'codex-runtime-harness = { path = "runtime-harness" }\n',
)

# The harness was previously an intentionally isolated nested workspace. Once it
# is a root member, package metadata and lint configuration come from codex-rs.
replace_once(
    "codex-rs/runtime-harness/Cargo.toml",
    '[workspace]\n\n'
    '[workspace.package]\n'
    'version = "0.0.0"\n'
    'edition = "2024"\n'
    'license = "Apache-2.0"\n\n'
    '[workspace.lints.rust]\n\n'
    '[workspace.lints.clippy]\n\n',
    '',
)
Path("codex-rs/runtime-harness/Cargo.lock").unlink()

replace_once(
    "codex-rs/tui/Cargo.toml",
    'codex-protocol = { workspace = true }\n',
    'codex-protocol = { workspace = true }\n'
    'codex-runtime-harness = { workspace = true }\n',
)

# Link the already-landed bridge source into the TUI module graph.
replace_once(
    "codex-rs/tui/src/lib.rs",
    'mod resume_picker;\nmod selection_list;\n',
    'mod resume_picker;\nmod runtime_bridge;\nmod selection_list;\n',
)
replace_once(
    "codex-rs/tui/src/app.rs",
    'use crate::resume_picker::SessionTarget;\nuse crate::session_state::ThreadSessionState;\n',
    'use crate::resume_picker::SessionTarget;\nuse crate::runtime_bridge::RuntimeBridge;\n'
    'use crate::session_state::ThreadSessionState;\n',
)
replace_once(
    "codex-rs/tui/src/app.rs",
    'pub(crate) struct App {\n    model_catalog: Arc<ModelCatalog>,\n',
    'pub(crate) struct App {\n    model_catalog: Arc<ModelCatalog>,\n'
    '    pub(crate) runtime_bridge: RuntimeBridge,\n',
)

# Construct exactly one process-lifetime bridge. Cursor remains lazy inside
# RuntimeHarness::embedded, so startup does not launch `agent acp`.
replace_once(
    "codex-rs/tui/src/app/startup.rs",
    '        let mut app = Self {\n            model_catalog,\n            session_telemetry: session_telemetry.clone(),\n',
    '        let runtime_bridge = RuntimeBridge::new(&config, &model)\n'
    '            .wrap_err("failed to initialize multi-runtime harness")?;\n\n'
    '        let mut app = Self {\n'
    '            model_catalog,\n'
    '            runtime_bridge,\n'
    '            session_telemetry: session_telemetry.clone(),\n',
)
replace_once(
    "codex-rs/tui/src/app/startup.rs",
    '        if let Err(err) = app_server.shutdown().await {\n'
    '            tracing::warn!(error = %err, "failed to shut down embedded app server");\n'
    '        }\n',
    '        if let Err(err) = app.runtime_bridge.shutdown().await {\n'
    '            tracing::warn!(error = %err, "failed to shut down multi-runtime harness");\n'
    '        }\n'
    '        if let Err(err) = app_server.shutdown().await {\n'
    '            tracing::warn!(error = %err, "failed to shut down embedded app server");\n'
    '        }\n',
)
