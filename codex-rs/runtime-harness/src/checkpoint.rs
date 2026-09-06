use crate::HarnessEvent;
use crate::HarnessSession;
use crate::RuntimeModelId;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

pub const CHECKPOINT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarnessCheckpoint {
    pub format_version: u32,
    pub session_id: String,
    pub working_directory: PathBuf,
    pub model: RuntimeModelId,
    pub account: Option<String>,
    pub transcript: Vec<HarnessEvent>,
    /// Generation of the disposable backend at snapshot time. This is evidence
    /// only; restored sessions always start with no backend binding.
    pub source_backend_generation: u64,
}

impl HarnessCheckpoint {
    pub fn capture(session: &HarnessSession) -> Result<Self, CheckpointError> {
        if session.is_turn_active() {
            return Err(CheckpointError::TurnActive);
        }
        Ok(Self {
            format_version: CHECKPOINT_FORMAT_VERSION,
            session_id: session.id.clone(),
            working_directory: session.working_directory.clone(),
            model: session.model.clone(),
            account: session.account.clone(),
            transcript: session.transcript.clone(),
            source_backend_generation: session.backend_generation,
        })
    }

    pub fn restore(self) -> Result<HarnessSession, CheckpointError> {
        if self.format_version != CHECKPOINT_FORMAT_VERSION {
            return Err(CheckpointError::UnsupportedVersion {
                found: self.format_version,
                supported: CHECKPOINT_FORMAT_VERSION,
            });
        }
        let mut session = HarnessSession::new(
            self.session_id,
            self.working_directory,
            self.model,
        );
        session.account = self.account;
        session.transcript = self.transcript;
        session.invalidate_backend();
        Ok(session)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    #[error("cannot checkpoint a runtime session while a turn is active")]
    TurnActive,
    #[error("unsupported checkpoint format version {found}; supported version is {supported}")]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("checkpoint I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("checkpoint serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct CheckpointStore {
    path: PathBuf,
}

impl CheckpointStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn save_session(&self, session: &HarnessSession) -> Result<HarnessCheckpoint, CheckpointError> {
        let checkpoint = HarnessCheckpoint::capture(session)?;
        self.save(&checkpoint)?;
        Ok(checkpoint)
    }

    pub fn save(&self, checkpoint: &HarnessCheckpoint) -> Result<(), CheckpointError> {
        if checkpoint.format_version != CHECKPOINT_FORMAT_VERSION {
            return Err(CheckpointError::UnsupportedVersion {
                found: checkpoint.format_version,
                supported: CHECKPOINT_FORMAT_VERSION,
            });
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| CheckpointError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let bytes = serde_json::to_vec_pretty(checkpoint)?;
        let temporary = temporary_path(&self.path);
        let mut file = File::create(&temporary).map_err(|source| CheckpointError::Io {
            path: temporary.clone(),
            source,
        })?;
        file.write_all(&bytes).map_err(|source| CheckpointError::Io {
            path: temporary.clone(),
            source,
        })?;
        file.write_all(b"\n").map_err(|source| CheckpointError::Io {
            path: temporary.clone(),
            source,
        })?;
        file.sync_all().map_err(|source| CheckpointError::Io {
            path: temporary.clone(),
            source,
        })?;
        drop(file);

        replace_file(&temporary, &self.path)
    }

    pub fn load(&self) -> Result<Option<HarnessCheckpoint>, CheckpointError> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(CheckpointError::Io {
                    path: self.path.clone(),
                    source,
                });
            }
        };
        let checkpoint: HarnessCheckpoint = serde_json::from_slice(&bytes)?;
        if checkpoint.format_version != CHECKPOINT_FORMAT_VERSION {
            return Err(CheckpointError::UnsupportedVersion {
                found: checkpoint.format_version,
                supported: CHECKPOINT_FORMAT_VERSION,
            });
        }
        Ok(Some(checkpoint))
    }

    pub fn restore_session(&self) -> Result<Option<HarnessSession>, CheckpointError> {
        self.load()?.map(HarnessCheckpoint::restore).transpose()
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("checkpoint.json");
    path.with_file_name(format!(".{file_name}.tmp"))
}

fn replace_file(temporary: &Path, target: &Path) -> Result<(), CheckpointError> {
    match fs::rename(temporary, target) {
        Ok(()) => Ok(()),
        Err(first_error)
            if target.exists()
                && matches!(
                    first_error.kind(),
                    std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::PermissionDenied
                ) =>
        {
            fs::remove_file(target).map_err(|source| CheckpointError::Io {
                path: target.to_path_buf(),
                source,
            })?;
            fs::rename(temporary, target).map_err(|source| CheckpointError::Io {
                path: target.to_path_buf(),
                source,
            })
        }
        Err(source) => Err(CheckpointError::Io {
            path: target.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProviderId;
    use tempfile::tempdir;

    fn session() -> HarnessSession {
        let mut session = HarnessSession::new(
            "checkpoint-session",
            PathBuf::from("/tmp/project"),
            RuntimeModelId::new(ProviderId::Cursor, "auto").unwrap(),
        );
        session.account = Some("cursor-account".to_string());
        session.record_event(HarnessEvent::UserMessage {
            text: "inspect the parser".to_string(),
        });
        session.record_event(HarnessEvent::AssistantMessage {
            text: "parser inspected".to_string(),
        });
        session.bind_backend_session(crate::RuntimeSessionId("disposable".to_string()), 7);
        session
    }

    #[test]
    fn checkpoint_round_trip_preserves_canonical_state_not_backend_identity() {
        let original = session();
        let checkpoint = HarnessCheckpoint::capture(&original).unwrap();
        let restored = checkpoint.restore().unwrap();

        assert_eq!(restored.id, original.id);
        assert_eq!(restored.working_directory, original.working_directory);
        assert_eq!(restored.model, original.model);
        assert_eq!(restored.account, original.account);
        assert_eq!(restored.transcript, original.transcript);
        assert!(restored.backend_session.is_none());
        assert_eq!(restored.backend_generation, 0);
        assert!(!restored.is_turn_active());
    }

    #[test]
    fn active_turn_checkpoint_is_rejected() {
        let mut active = session();
        active.begin_turn("make a change");

        assert!(matches!(
            HarnessCheckpoint::capture(&active),
            Err(CheckpointError::TurnActive)
        ));
    }

    #[test]
    fn store_persists_and_restores_checkpoint() {
        let directory = tempdir().unwrap();
        let store = CheckpointStore::new(directory.path().join("state/session.json"));
        let original = session();

        store.save_session(&original).unwrap();
        let restored = store.restore_session().unwrap().unwrap();

        assert_eq!(restored.transcript, original.transcript);
        assert_eq!(restored.account, original.account);
        assert!(restored.backend_session.is_none());
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let mut checkpoint = HarnessCheckpoint::capture(&session()).unwrap();
        checkpoint.format_version = CHECKPOINT_FORMAT_VERSION + 1;

        assert!(matches!(
            checkpoint.restore(),
            Err(CheckpointError::UnsupportedVersion { .. })
        ));
    }
}
