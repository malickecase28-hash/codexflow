use serde::Deserialize;
use serde::Serialize;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerenaMcpLaunch {
    pub server_name: String,
    pub command: String,
    pub args: Vec<String>,
    pub project: PathBuf,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SerenaMcpConfigError {
    #[error("Serena project path must be absolute: {0}")]
    ProjectPathNotAbsolute(PathBuf),
    #[error("Serena source cannot be empty")]
    EmptySource,
}

impl SerenaMcpLaunch {
    /// Build the stdio launch registration consumed by Codex's existing MCP
    /// server configuration/runtime. This type does not implement an MCP client.
    ///
    /// `source` should normally be a pinned uv-compatible source such as a git
    /// URL with a tag/revision. Keeping the source explicit makes upgrades and
    /// security review deliberate rather than silently tracking latest.
    pub fn uvx(
        project: impl Into<PathBuf>,
        source: impl Into<String>,
    ) -> Result<Self, SerenaMcpConfigError> {
        let project = project.into();
        if !project.is_absolute() {
            return Err(SerenaMcpConfigError::ProjectPathNotAbsolute(project));
        }
        let source = source.into();
        if source.trim().is_empty() {
            return Err(SerenaMcpConfigError::EmptySource);
        }
        let project_arg = project.to_string_lossy().into_owned();
        Ok(Self {
            server_name: "serena".to_string(),
            command: "uvx".to_string(),
            args: vec![
                "--from".to_string(),
                source.clone(),
                "serena".to_string(),
                "start-mcp-server".to_string(),
                "--transport".to_string(),
                "stdio".to_string(),
                "--context".to_string(),
                "ide-assistant".to_string(),
                "--project".to_string(),
                project_arg,
            ],
            project,
            source,
        })
    }

    pub fn project(&self) -> &Path {
        &self.project
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_uses_codex_compatible_stdio_server_shape() {
        let launch = SerenaMcpLaunch::uvx(
            PathBuf::from("/workspace/repo"),
            "git+https://github.com/oraios/serena@v1.0.52",
        )
        .unwrap();

        assert_eq!(launch.server_name, "serena");
        assert_eq!(launch.command, "uvx");
        assert!(launch.args.windows(2).any(|pair| pair == ["--transport", "stdio"]));
        assert!(launch
            .args
            .windows(2)
            .any(|pair| pair == ["--context", "ide-assistant"]));
        assert!(launch
            .args
            .windows(2)
            .any(|pair| pair == ["--project", "/workspace/repo"]));
    }

    #[test]
    fn relative_projects_are_rejected() {
        assert!(matches!(
            SerenaMcpLaunch::uvx(PathBuf::from("repo"), "git+https://github.com/oraios/serena"),
            Err(SerenaMcpConfigError::ProjectPathNotAbsolute(_))
        ));
    }

    #[test]
    fn source_must_be_explicit() {
        assert_eq!(
            SerenaMcpLaunch::uvx(PathBuf::from("/workspace/repo"), "").unwrap_err(),
            SerenaMcpConfigError::EmptySource
        );
    }
}
