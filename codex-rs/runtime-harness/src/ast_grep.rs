use crate::CodeIntelligenceProvider;
use crate::CodeMatch;
use crate::ProviderQueryError;
use crate::SourcePosition;
use crate::SourceSpan;
use crate::StructuralEdit;
use crate::StructuralQuery;
use crate::StructuralRewrite;
use crate::StructuralRewriteResult;
use serde::Deserialize;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct AstGrepCliProvider {
    executable: PathBuf,
    working_directory: PathBuf,
}

impl AstGrepCliProvider {
    pub fn new(executable: impl Into<PathBuf>, working_directory: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            working_directory: working_directory.into(),
        }
    }

    fn execute(
        &self,
        query: &StructuralQuery,
        replacement: Option<&str>,
    ) -> Result<Vec<AstGrepMatch>, ProviderQueryError> {
        let args = ast_grep_args(query, replacement);
        let output = Command::new(&self.executable)
            .args(&args)
            .current_dir(&self.working_directory)
            .output()
            .map_err(|error| {
                ProviderQueryError::Failed(format!(
                    "failed to execute ast-grep at {}: {error}",
                    self.executable.display()
                ))
            })?;

        if !output.status.success() {
            return Err(ProviderQueryError::Failed(format!(
                "ast-grep exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }

        serde_json::from_slice(&output.stdout).map_err(|error| {
            ProviderQueryError::Failed(format!("invalid ast-grep JSON output: {error}"))
        })
    }
}

impl CodeIntelligenceProvider for AstGrepCliProvider {
    fn provider_id(&self) -> &str {
        "ast-grep"
    }

    fn structural_search(
        &self,
        query: &StructuralQuery,
    ) -> Result<Vec<CodeMatch>, ProviderQueryError> {
        self.execute(query, None)?
            .into_iter()
            .map(AstGrepMatch::into_code_match)
            .collect()
    }

    /// Produce deterministic edit proposals without mutating the repository.
    ///
    /// ast-grep only applies rewrites when update/interactive flags are used.
    /// This adapter intentionally omits those flags so the workflow can inspect,
    /// policy-check, checkpoint, and verify edits before a separate apply step.
    fn structural_rewrite(
        &self,
        rewrite: &StructuralRewrite,
    ) -> Result<StructuralRewriteResult, ProviderQueryError> {
        let matches = self.execute(&rewrite.query, Some(&rewrite.replacement))?;
        let mut edits = Vec::with_capacity(matches.len());
        for matched in matches {
            let after = matched.replacement.clone().ok_or_else(|| {
                ProviderQueryError::Failed(
                    "ast-grep rewrite result omitted replacement text".to_string(),
                )
            })?;
            edits.push(StructuralEdit {
                span: matched.span(),
                before: matched.text,
                after,
            });
        }
        Ok(StructuralRewriteResult { edits })
    }
}

fn ast_grep_args(query: &StructuralQuery, replacement: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "run".to_string(),
        "--pattern".to_string(),
        query.pattern.clone(),
        "--lang".to_string(),
        query.language.clone(),
        "--json=compact".to_string(),
        "--color=never".to_string(),
        "--threads=1".to_string(),
    ];
    if let Some(replacement) = replacement {
        args.push("--rewrite".to_string());
        args.push(replacement.to_string());
    }
    if query.paths.is_empty() {
        args.push(".".to_string());
    } else {
        args.extend(
            query
                .paths
                .iter()
                .map(|path| path.to_string_lossy().into_owned()),
        );
    }
    args
}

#[derive(Debug, Deserialize)]
struct AstGrepMatch {
    text: String,
    file: PathBuf,
    range: AstGrepRange,
    replacement: Option<String>,
}

impl AstGrepMatch {
    fn span(&self) -> SourceSpan {
        SourceSpan {
            path: self.file.clone(),
            start: SourcePosition {
                line: self.range.start.line,
                column: self.range.start.column,
            },
            end: SourcePosition {
                line: self.range.end.line,
                column: self.range.end.column,
            },
        }
    }

    fn into_code_match(self) -> Result<CodeMatch, ProviderQueryError> {
        let span = self.span();
        Ok(CodeMatch {
            span,
            symbol: None,
            snippet: Some(self.text),
        })
    }
}

#[derive(Debug, Deserialize)]
struct AstGrepRange {
    start: AstGrepPosition,
    end: AstGrepPosition,
}

#[derive(Debug, Deserialize)]
struct AstGrepPosition {
    line: u32,
    column: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_is_non_mutating_and_machine_readable() {
        let query = StructuralQuery::new("rust", "Some($A)")
            .with_paths([PathBuf::from("src"), PathBuf::from("tests")]);
        let args = ast_grep_args(&query, Some("None"));

        assert_eq!(args[0], "run");
        assert!(args.contains(&"--json=compact".to_string()));
        assert!(args.contains(&"--rewrite".to_string()));
        assert!(!args.contains(&"--update-all".to_string()));
        assert!(!args.contains(&"-U".to_string()));
        assert_eq!(&args[args.len() - 2..], &["src", "tests"]);
    }

    #[test]
    fn parses_current_ast_grep_zero_based_json_ranges() {
        let parsed: Vec<AstGrepMatch> = serde_json::from_str(
            r#"[{"text":"Some(matched)","range":{"byteOffset":{"start":10828,"end":10841},"start":{"line":303,"column":2},"end":{"line":303,"column":15}},"file":"crates/config/src/rule/mod.rs","replacement":"None","language":"Rust"}]"#,
        )
        .unwrap();

        let matched = &parsed[0];
        assert_eq!(matched.range.start.line, 303);
        assert_eq!(matched.range.start.column, 2);
        assert_eq!(matched.range.end.column, 15);
        assert_eq!(matched.replacement.as_deref(), Some("None"));
    }
}
