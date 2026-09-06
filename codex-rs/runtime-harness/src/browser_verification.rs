use crate::VerificationEvidence;
use crate::VerificationKind;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserStep {
    /// Playwright CLI arguments after the session selector, for example
    /// `["click", "e12"]` or `["fill", "e5", "hello"]`.
    pub args: Vec<String>,
}

impl BrowserStep {
    pub fn new(args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            args: args.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SnapshotAssertion {
    Contains { text: String },
    Excludes { text: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserVerificationPlan {
    pub session: String,
    pub url: String,
    pub steps: Vec<BrowserStep>,
    pub assertions: Vec<SnapshotAssertion>,
    pub capture_screenshot: bool,
}

impl BrowserVerificationPlan {
    pub fn new(session: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            session: session.into(),
            url: url.into(),
            steps: Vec::new(),
            assertions: Vec::new(),
            capture_screenshot: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotAssertionResult {
    pub assertion: SnapshotAssertion,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserCommandEvidence {
    pub args: Vec<String>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserVerificationEvidence {
    pub passed: bool,
    pub snapshot_path: PathBuf,
    pub screenshot_path: Option<PathBuf>,
    pub assertions: Vec<SnapshotAssertionResult>,
    pub commands: Vec<BrowserCommandEvidence>,
}

impl BrowserVerificationEvidence {
    pub fn workflow_evidence(&self) -> VerificationEvidence {
        let failed = self.assertions.iter().filter(|result| !result.passed).count();
        VerificationEvidence {
            verifier: "playwright-cli".to_string(),
            kind: VerificationKind::Browser,
            passed: self.passed,
            detail: format!(
                "{} browser assertion(s), {failed} failed",
                self.assertions.len()
            ),
            artifact: self
                .screenshot_path
                .clone()
                .or_else(|| Some(self.snapshot_path.clone())),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BrowserVerificationError {
    #[error("invalid Playwright session name '{0}'")]
    InvalidSession(String),
    #[error("browser verification plan has an empty URL")]
    EmptyUrl,
    #[error("browser verification plan requires at least one snapshot assertion")]
    EmptyAssertions,
    #[error("Playwright command failed: {args:?}; stderr: {stderr}")]
    CommandFailed { args: Vec<String>, stderr: String },
    #[error("failed to execute Playwright command {args:?}: {source}")]
    CommandIo {
        args: Vec<String>,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read browser artifact {path}: {source}")]
    ArtifactIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct PlaywrightCliVerifier {
    executable: PathBuf,
    working_directory: PathBuf,
    artifact_directory: PathBuf,
}

impl PlaywrightCliVerifier {
    pub fn new(
        executable: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
        artifact_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            executable: executable.into(),
            working_directory: working_directory.into(),
            artifact_directory: artifact_directory.into(),
        }
    }

    pub async fn verify(
        &self,
        plan: &BrowserVerificationPlan,
    ) -> Result<BrowserVerificationEvidence, BrowserVerificationError> {
        validate_plan(plan)?;
        fs::create_dir_all(&self.artifact_directory).map_err(|source| {
            BrowserVerificationError::ArtifactIo {
                path: self.artifact_directory.clone(),
                source,
            }
        })?;

        let snapshot_path = self
            .artifact_directory
            .join(format!("{}-snapshot.yml", plan.session));
        let screenshot_path = plan
            .capture_screenshot
            .then(|| self.artifact_directory.join(format!("{}-screenshot.png", plan.session)));
        let mut commands = Vec::new();

        let result = async {
            commands.push(
                self.run(&plan.session, vec!["open".to_string(), plan.url.clone()])
                    .await?,
            );
            for step in &plan.steps {
                commands.push(self.run(&plan.session, step.args.clone()).await?);
            }
            commands.push(
                self.run(
                    &plan.session,
                    vec![
                        "snapshot".to_string(),
                        format!("--filename={}", snapshot_path.display()),
                    ],
                )
                .await?,
            );
            if let Some(path) = &screenshot_path {
                commands.push(
                    self.run(
                        &plan.session,
                        vec![
                            "screenshot".to_string(),
                            "--full-page".to_string(),
                            format!("--filename={}", path.display()),
                        ],
                    )
                    .await?,
                );
            }

            let snapshot = fs::read_to_string(&snapshot_path).map_err(|source| {
                BrowserVerificationError::ArtifactIo {
                    path: snapshot_path.clone(),
                    source,
                }
            })?;
            let assertions = plan
                .assertions
                .iter()
                .cloned()
                .map(|assertion| {
                    let passed = match &assertion {
                        SnapshotAssertion::Contains { text } => snapshot.contains(text),
                        SnapshotAssertion::Excludes { text } => !snapshot.contains(text),
                    };
                    SnapshotAssertionResult { assertion, passed }
                })
                .collect::<Vec<_>>();
            let passed = assertions.iter().all(|result| result.passed);

            Ok(BrowserVerificationEvidence {
                passed,
                snapshot_path,
                screenshot_path,
                assertions,
                commands,
            })
        }
        .await;

        let _ = self
            .run(&plan.session, vec!["close".to_string()])
            .await;
        result
    }

    async fn run(
        &self,
        session: &str,
        args: Vec<String>,
    ) -> Result<BrowserCommandEvidence, BrowserVerificationError> {
        let mut full_args = vec![format!("-s={session}")];
        full_args.extend(args);
        let output = Command::new(&self.executable)
            .args(&full_args)
            .current_dir(&self.working_directory)
            .output()
            .await
            .map_err(|source| BrowserVerificationError::CommandIo {
                args: full_args.clone(),
                source,
            })?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if !output.status.success() {
            return Err(BrowserVerificationError::CommandFailed {
                args: full_args,
                stderr,
            });
        }
        Ok(BrowserCommandEvidence {
            args: full_args,
            stdout,
            stderr,
        })
    }
}

fn validate_plan(plan: &BrowserVerificationPlan) -> Result<(), BrowserVerificationError> {
    if plan.url.trim().is_empty() {
        return Err(BrowserVerificationError::EmptyUrl);
    }
    if plan.assertions.is_empty() {
        return Err(BrowserVerificationError::EmptyAssertions);
    }
    if plan.session.is_empty()
        || !plan
            .session
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(BrowserVerificationError::InvalidSession(plan.session.clone()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_rejects_invalid_session_names() {
        let mut plan = BrowserVerificationPlan::new("bad session", "https://example.com");
        plan.assertions.push(SnapshotAssertion::Contains {
            text: "Example".to_string(),
        });
        assert!(matches!(
            validate_plan(&plan),
            Err(BrowserVerificationError::InvalidSession(_))
        ));
    }

    #[test]
    fn plan_rejects_empty_urls() {
        let mut plan = BrowserVerificationPlan::new("test", "  ");
        plan.assertions.push(SnapshotAssertion::Contains {
            text: "Example".to_string(),
        });
        assert!(matches!(
            validate_plan(&plan),
            Err(BrowserVerificationError::EmptyUrl)
        ));
    }

    #[test]
    fn plan_rejects_vacuous_verification() {
        let plan = BrowserVerificationPlan::new("test", "https://example.com");
        assert!(matches!(
            validate_plan(&plan),
            Err(BrowserVerificationError::EmptyAssertions)
        ));
    }

    #[test]
    fn snapshot_assertions_are_serializable_for_evidence_logs() {
        let assertions = vec![
            SnapshotAssertion::Contains {
                text: "Save".to_string(),
            },
            SnapshotAssertion::Excludes {
                text: "Fatal error".to_string(),
            },
        ];
        let json = serde_json::to_string(&assertions).unwrap();
        let restored: Vec<SnapshotAssertion> = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, assertions);
    }

    #[test]
    fn browser_evidence_maps_into_workflow_verification() {
        let evidence = BrowserVerificationEvidence {
            passed: true,
            snapshot_path: PathBuf::from("snapshot.yml"),
            screenshot_path: Some(PathBuf::from("page.png")),
            assertions: vec![SnapshotAssertionResult {
                assertion: SnapshotAssertion::Contains {
                    text: "Save".to_string(),
                },
                passed: true,
            }],
            commands: Vec::new(),
        };
        let workflow = evidence.workflow_evidence();
        assert!(workflow.passed);
        assert_eq!(workflow.kind, VerificationKind::Browser);
        assert_eq!(workflow.artifact, Some(PathBuf::from("page.png")));
    }

    #[test]
    fn configured_artifact_paths_remain_inside_artifact_directory() {
        let verifier = PlaywrightCliVerifier::new(
            "playwright-cli",
            Path::new("."),
            Path::new("artifacts/browser"),
        );
        let path = verifier.artifact_directory.join("session-snapshot.yml");
        assert!(path.starts_with(&verifier.artifact_directory));
    }
}
