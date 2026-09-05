from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one patch anchor, found {count}")
    path.write_text(text.replace(old, new, 1))


account_protocol = Path("codex-rs/app-server-protocol/src/protocol/v2/account.rs")
replace_once(
    account_protocol,
    '''pub struct LogoutAccountResponse {}
''',
    '''pub struct LogoutAccountResponse {}

/// Confirms that app-server reconciled its exact live authentication manager
/// with the credentials currently stored in the native Codex auth store.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ReloadAccountAuthResponse {}
''',
)

common = Path("codex-rs/app-server-protocol/src/protocol/common.rs")
replace_once(
    common,
    '''    LoginAccount => "account/login/start" {
        params: v2::LoginAccountParams,
        inspect_params: true,
        serialization: global("account-auth"),
        response: v2::LoginAccountResponse,
    },
''',
    '''    LoginAccount => "account/login/start" {
        params: v2::LoginAccountParams,
        inspect_params: true,
        serialization: global("account-auth"),
        response: v2::LoginAccountResponse,
    },

    /// Reload the native credential store into the exact process-local AuthManager.
    /// This carries no credentials over JSON-RPC and is serialized with login/logout.
    ReloadAccountAuth => "account/auth/reload" {
        params: #[ts(type = "undefined")] #[serde(skip_serializing_if = "Option::is_none")] Option<()>,
        serialization: global("account-auth"),
        response: v2::ReloadAccountAuthResponse,
    },
''',
)

processor = Path("codex-rs/app-server/src/request_processors/account_processor.rs")
replace_once(
    processor,
    '''    pub(crate) async fn logout_account(
        &self,
        request_id: ConnectionRequestId,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.logout_v2(request_id).await.map(|()| None)
    }
''',
    '''    pub(crate) async fn logout_account(
        &self,
        request_id: ConnectionRequestId,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.logout_v2(request_id).await.map(|()| None)
    }

    pub(crate) async fn reload_account_auth(
        &self,
        request_id: ConnectionRequestId,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        let result = self.reload_account_auth_response().await;
        let account_updated = result
            .as_ref()
            .ok()
            .map(|_| self.current_account_updated_notification());
        self.outgoing.send_result(request_id, result).await;
        if let Some(payload) = account_updated {
            self.outgoing
                .send_server_notification(ServerNotification::AccountUpdated(payload))
                .await;
        }
        Ok(None)
    }
''',
)
replace_once(
    processor,
    '''    async fn logout_common(&self) -> std::result::Result<Option<AuthMode>, JSONRPCErrorError> {
''',
    '''    async fn reload_account_auth_response(
        &self,
    ) -> Result<ReloadAccountAuthResponse, JSONRPCErrorError> {
        if self.auth_manager.is_workload_identity_selected() {
            return Err(self.configured_auth_owned_by_host_error());
        }
        if self.auth_manager.is_external_chatgpt_auth_active() {
            return Err(self.external_auth_active_error());
        }

        // Prevent an in-flight interactive login from overwriting the account
        // selected by the embedded runtime harness after this reload completes.
        self.cancel_active_login().await;
        self.auth_manager.reload().await;

        let auth = self.auth_manager.auth_cached().ok_or_else(|| {
            invalid_request("native auth reload did not produce usable OpenAI credentials")
        })?;

        self.config_manager.clear_cloud_config_bundle_loader();
        if auth.uses_codex_backend() {
            self.config_manager.replace_cloud_config_bundle_loader(
                self.auth_manager.clone(),
                self.config.chatgpt_base_url.clone(),
                self.config.http_client_factory(),
            );
        }
        self.config_manager
            .sync_default_client_residency_requirement()
            .await;
        Self::maybe_refresh_plugin_caches_for_current_config(
            &self.config_manager,
            &self.thread_manager,
            Some(auth),
        )
        .await;

        Ok(ReloadAccountAuthResponse {})
    }

    async fn logout_common(&self) -> std::result::Result<Option<AuthMode>, JSONRPCErrorError> {
''',
)

message_processor = Path("codex-rs/app-server/src/message_processor.rs")
replace_once(
    message_processor,
    '''            ClientRequest::LoginAccount { params, .. } => {
                self.account_processor
                    .login_account(request_id.clone(), params)
                    .await
            }
''',
    '''            ClientRequest::LoginAccount { params, .. } => {
                self.account_processor
                    .login_account(request_id.clone(), params)
                    .await
            }
            ClientRequest::ReloadAccountAuth { .. } => {
                self.account_processor
                    .reload_account_auth(request_id.clone())
                    .await
            }
''',
)

session = Path("codex-rs/tui/src/app_server_session.rs")
replace_once(
    session,
    '''use codex_app_server_protocol::LogoutAccountResponse;
''',
    '''use codex_app_server_protocol::LogoutAccountResponse;
use codex_app_server_protocol::ReloadAccountAuthResponse;
''',
)
replace_once(
    session,
    '''    pub(crate) async fn read_account(&mut self) -> Result<GetAccountResponse> {
''',
    '''    /// Reconcile the embedded app-server's exact live AuthManager with the
    /// native credential store. Runtime account switching uses this narrow RPC
    /// instead of exposing AuthManager to the TUI.
    pub(crate) async fn reload_account_auth(&mut self) -> Result<()> {
        let request_id = self.next_request_id();
        let _: ReloadAccountAuthResponse = self
            .client
            .request_typed(ClientRequest::ReloadAccountAuth {
                request_id,
                params: None,
            })
            .await
            .wrap_err("account/auth/reload failed during runtime account switch")?;
        Ok(())
    }

    pub(crate) async fn read_account(&mut self) -> Result<GetAccountResponse> {
''',
)
