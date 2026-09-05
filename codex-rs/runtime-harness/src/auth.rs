use crate::AccountBroker;
use crate::AccountBrokerError;
use crate::ImportedAccount;
use crate::ProviderId;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum AuthCoordinatorError {
    #[error("provider {0} does not have a harness-managed login command")]
    LoginUnsupported(ProviderId),
    #[error("Cursor CLI agent was not found; install Cursor CLI or set CODEX_CURSOR_AGENT")]
    CursorAgentUnavailable,
    #[error("failed to launch {executable} login: {source}")]
    LoginSpawn {
        executable: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{provider} login exited unsuccessfully with {status}")]
    LoginFailed { provider: ProviderId, status: String },
    #[error(transparent)]
    Account(#[from] AccountBrokerError),
}

/// Coordinates provider-native authentication without implementing OAuth.
///
/// OpenAI login remains owned by the existing Codex UI. Once that flow succeeds,
/// `import_after_native_login(ProviderId::OpenAi, ..)` imports the active Codex
/// credential through embedded subswap. Cursor authentication is delegated to the
/// official CLI (`agent login`) and then imported the same way.
pub struct AuthCoordinator {
    broker: Arc<AccountBroker>,
    cursor_executable: Option<PathBuf>,
}

impl AuthCoordinator {
    pub fn new(broker: Arc<AccountBroker>) -> Self {
        Self {
            broker,
            cursor_executable: None,
        }
    }

    pub fn with_cursor_executable(
        broker: Arc<AccountBroker>,
        executable: impl Into<PathBuf>,
    ) -> Self {
        Self {
            broker,
            cursor_executable: Some(executable.into()),
        }
    }

    pub async fn import_after_native_login(
        &self,
        provider: ProviderId,
        label_hint: Option<String>,
    ) -> Result<ImportedAccount, AuthCoordinatorError> {
        Ok(self.broker.import_active(provider, label_hint).await?)
    }

    pub async fn login_cursor(
        &self,
        label_hint: Option<String>,
    ) -> Result<ImportedAccount, AuthCoordinatorError> {
        let status = self.run_cursor_login().await?;
        if !status.success() {
            return Err(AuthCoordinatorError::LoginFailed {
                provider: ProviderId::Cursor,
                status: status.to_string(),
            });
        }
        self.import_after_native_login(ProviderId::Cursor, label_hint)
            .await
    }

    pub async fn login(
        &self,
        provider: ProviderId,
        label_hint: Option<String>,
    ) -> Result<ImportedAccount, AuthCoordinatorError> {
        match provider {
            ProviderId::Cursor => self.login_cursor(label_hint).await,
            ProviderId::OpenAi => Err(AuthCoordinatorError::LoginUnsupported(provider)),
        }
    }

    async fn run_cursor_login(&self) -> Result<std::process::ExitStatus, AuthCoordinatorError> {
        let mut candidates = Vec::new();
        if let Some(executable) = self.cursor_executable.clone() {
            candidates.push(executable);
        } else if let Some(executable) = std::env::var_os("CODEX_CURSOR_AGENT") {
            candidates.push(PathBuf::from(executable));
        }
        if self.cursor_executable.is_none() {
            candidates.push(PathBuf::from("agent"));
            candidates.push(PathBuf::from("cursor-agent"));
        }

        for executable in candidates {
            let mut command = Command::new(&executable);
            command
                .arg("login")
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
            match command.status().await {
                Ok(status) => return Ok(status),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(source) => {
                    return Err(AuthCoordinatorError::LoginSpawn {
                        executable: executable.display().to_string(),
                        source,
                    });
                }
            }
        }
        Err(AuthCoordinatorError::CursorAgentUnavailable)
    }
}
