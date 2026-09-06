use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourcePosition {
    /// Zero-based line number.
    pub line: u32,
    /// Zero-based UTF-8 column offset as reported by the provider.
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSpan {
    pub path: PathBuf,
    pub start: SourcePosition,
    pub end: SourcePosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticQueryKind {
    Definition,
    References,
    Implementations,
    WorkspaceSymbols,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticQuery {
    pub kind: SemanticQueryKind,
    pub symbol: String,
    pub path_hint: Option<PathBuf>,
}

impl SemanticQuery {
    pub fn new(kind: SemanticQueryKind, symbol: impl Into<String>) -> Self {
        Self {
            kind,
            symbol: symbol.into(),
            path_hint: None,
        }
    }

    pub fn with_path_hint(mut self, path: impl Into<PathBuf>) -> Self {
        self.path_hint = Some(path.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuralQuery {
    pub language: String,
    pub pattern: String,
    pub paths: Vec<PathBuf>,
}

impl StructuralQuery {
    pub fn new(language: impl Into<String>, pattern: impl Into<String>) -> Self {
        Self {
            language: language.into(),
            pattern: pattern.into(),
            paths: Vec::new(),
        }
    }

    pub fn with_paths(mut self, paths: impl IntoIterator<Item = PathBuf>) -> Self {
        self.paths = paths.into_iter().collect();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuralRewrite {
    pub query: StructuralQuery,
    pub replacement: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeMatch {
    pub span: SourceSpan,
    pub symbol: Option<String>,
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuralEdit {
    pub span: SourceSpan,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuralRewriteResult {
    pub edits: Vec<StructuralEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderQueryError {
    Unsupported(String),
    Failed(String),
}

impl fmt::Display for ProviderQueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(message) => write!(f, "unsupported: {message}"),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ProviderQueryError {}

pub trait CodeIntelligenceProvider: Send + Sync {
    fn provider_id(&self) -> &str;

    fn semantic_query(
        &self,
        _query: &SemanticQuery,
    ) -> Result<Vec<CodeMatch>, ProviderQueryError> {
        Err(ProviderQueryError::Unsupported(
            "semantic query".to_string(),
        ))
    }

    fn structural_search(
        &self,
        _query: &StructuralQuery,
    ) -> Result<Vec<CodeMatch>, ProviderQueryError> {
        Err(ProviderQueryError::Unsupported(
            "structural search".to_string(),
        ))
    }

    fn structural_rewrite(
        &self,
        _rewrite: &StructuralRewrite,
    ) -> Result<StructuralRewriteResult, ProviderQueryError> {
        Err(ProviderQueryError::Unsupported(
            "structural rewrite".to_string(),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderAttemptStatus {
    Succeeded,
    Unsupported,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAttempt {
    pub provider: String,
    pub status: ProviderAttemptStatus,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryEvidence<T> {
    pub provider: String,
    pub attempts: Vec<ProviderAttempt>,
    pub result: T,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeIntelligenceError {
    pub operation: &'static str,
    pub attempts: Vec<ProviderAttempt>,
}

impl fmt::Display for CodeIntelligenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "no code-intelligence provider completed {}; {} provider attempt(s) recorded",
            self.operation,
            self.attempts.len()
        )
    }
}

impl std::error::Error for CodeIntelligenceError {}

#[derive(Default)]
pub struct CodeIntelligenceChain {
    providers: Vec<Box<dyn CodeIntelligenceProvider>>,
}

impl CodeIntelligenceChain {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push<P>(&mut self, provider: P)
    where
        P: CodeIntelligenceProvider + 'static,
    {
        self.providers.push(Box::new(provider));
    }

    pub fn semantic_query(
        &self,
        query: &SemanticQuery,
    ) -> Result<QueryEvidence<Vec<CodeMatch>>, CodeIntelligenceError> {
        let mut attempts = Vec::with_capacity(self.providers.len());
        for provider in &self.providers {
            match provider.semantic_query(query) {
                Ok(result) => {
                    let provider_id = provider.provider_id().to_string();
                    attempts.push(succeeded_attempt(&provider_id));
                    return Ok(QueryEvidence {
                        provider: provider_id,
                        attempts,
                        result,
                    });
                }
                Err(error) => attempts.push(failed_attempt(provider.provider_id(), error)),
            }
        }
        Err(CodeIntelligenceError {
            operation: "semantic query",
            attempts,
        })
    }

    pub fn structural_search(
        &self,
        query: &StructuralQuery,
    ) -> Result<QueryEvidence<Vec<CodeMatch>>, CodeIntelligenceError> {
        let mut attempts = Vec::with_capacity(self.providers.len());
        for provider in &self.providers {
            match provider.structural_search(query) {
                Ok(result) => {
                    let provider_id = provider.provider_id().to_string();
                    attempts.push(succeeded_attempt(&provider_id));
                    return Ok(QueryEvidence {
                        provider: provider_id,
                        attempts,
                        result,
                    });
                }
                Err(error) => attempts.push(failed_attempt(provider.provider_id(), error)),
            }
        }
        Err(CodeIntelligenceError {
            operation: "structural search",
            attempts,
        })
    }

    pub fn structural_rewrite(
        &self,
        rewrite: &StructuralRewrite,
    ) -> Result<QueryEvidence<StructuralRewriteResult>, CodeIntelligenceError> {
        let mut attempts = Vec::with_capacity(self.providers.len());
        for provider in &self.providers {
            match provider.structural_rewrite(rewrite) {
                Ok(result) => {
                    let provider_id = provider.provider_id().to_string();
                    attempts.push(succeeded_attempt(&provider_id));
                    return Ok(QueryEvidence {
                        provider: provider_id,
                        attempts,
                        result,
                    });
                }
                Err(error) => attempts.push(failed_attempt(provider.provider_id(), error)),
            }
        }
        Err(CodeIntelligenceError {
            operation: "structural rewrite",
            attempts,
        })
    }
}

fn succeeded_attempt(provider: &str) -> ProviderAttempt {
    ProviderAttempt {
        provider: provider.to_string(),
        status: ProviderAttemptStatus::Succeeded,
        detail: None,
    }
}

fn failed_attempt(provider: &str, error: ProviderQueryError) -> ProviderAttempt {
    let status = match &error {
        ProviderQueryError::Unsupported(_) => ProviderAttemptStatus::Unsupported,
        ProviderQueryError::Failed(_) => ProviderAttemptStatus::Failed,
    };
    ProviderAttempt {
        provider: provider.to_string(),
        status,
        detail: Some(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct UnsupportedProvider(&'static str);

    impl CodeIntelligenceProvider for UnsupportedProvider {
        fn provider_id(&self) -> &str {
            self.0
        }

        fn semantic_query(
            &self,
            _query: &SemanticQuery,
        ) -> Result<Vec<CodeMatch>, ProviderQueryError> {
            Err(ProviderQueryError::Unsupported(
                "semantic queries are disabled".to_string(),
            ))
        }

        fn structural_search(
            &self,
            _query: &StructuralQuery,
        ) -> Result<Vec<CodeMatch>, ProviderQueryError> {
            Err(ProviderQueryError::Unsupported(
                "structural search is disabled".to_string(),
            ))
        }

        fn structural_rewrite(
            &self,
            _rewrite: &StructuralRewrite,
        ) -> Result<StructuralRewriteResult, ProviderQueryError> {
            Err(ProviderQueryError::Unsupported(
                "structural rewrite is disabled".to_string(),
            ))
        }
    }

    struct WorkingProvider(&'static str);

    impl WorkingProvider {
        fn result() -> CodeMatch {
            CodeMatch {
                span: SourceSpan {
                    path: PathBuf::from("src/lib.rs"),
                    start: SourcePosition { line: 1, column: 2 },
                    end: SourcePosition { line: 1, column: 8 },
                },
                symbol: Some("target".to_string()),
                snippet: Some("target".to_string()),
            }
        }
    }

    impl CodeIntelligenceProvider for WorkingProvider {
        fn provider_id(&self) -> &str {
            self.0
        }

        fn semantic_query(
            &self,
            _query: &SemanticQuery,
        ) -> Result<Vec<CodeMatch>, ProviderQueryError> {
            Ok(vec![Self::result()])
        }

        fn structural_search(
            &self,
            _query: &StructuralQuery,
        ) -> Result<Vec<CodeMatch>, ProviderQueryError> {
            Ok(vec![Self::result()])
        }

        fn structural_rewrite(
            &self,
            _rewrite: &StructuralRewrite,
        ) -> Result<StructuralRewriteResult, ProviderQueryError> {
            Ok(StructuralRewriteResult {
                edits: vec![StructuralEdit {
                    span: Self::result().span,
                    before: "old".to_string(),
                    after: "new".to_string(),
                }],
            })
        }
    }

    #[test]
    fn semantic_query_falls_back_deterministically_and_keeps_evidence() {
        let mut chain = CodeIntelligenceChain::new();
        chain.push(UnsupportedProvider("serena"));
        chain.push(WorkingProvider("lsp"));

        let evidence = chain
            .semantic_query(&SemanticQuery::new(
                SemanticQueryKind::Definition,
                "target",
            ))
            .unwrap();

        assert_eq!(evidence.provider, "lsp");
        assert_eq!(evidence.result.len(), 1);
        assert_eq!(evidence.attempts.len(), 2);
        assert_eq!(
            evidence.attempts[0].status,
            ProviderAttemptStatus::Unsupported
        );
        assert_eq!(
            evidence.attempts[1].status,
            ProviderAttemptStatus::Succeeded
        );
    }

    #[test]
    fn structural_search_uses_same_ordered_fallback_contract() {
        let mut chain = CodeIntelligenceChain::new();
        chain.push(UnsupportedProvider("tree-sitter"));
        chain.push(WorkingProvider("ast-grep"));

        let evidence = chain
            .structural_search(&StructuralQuery::new("rust", "$A.unwrap()"))
            .unwrap();

        assert_eq!(evidence.provider, "ast-grep");
        assert_eq!(evidence.attempts.len(), 2);
    }

    #[test]
    fn structural_rewrite_returns_edit_evidence() {
        let mut chain = CodeIntelligenceChain::new();
        chain.push(WorkingProvider("ast-grep"));

        let evidence = chain
            .structural_rewrite(&StructuralRewrite {
                query: StructuralQuery::new("rust", "$A.unwrap()"),
                replacement: "$A?".to_string(),
            })
            .unwrap();

        assert_eq!(evidence.result.edits.len(), 1);
        assert_eq!(evidence.result.edits[0].before, "old");
        assert_eq!(evidence.result.edits[0].after, "new");
    }

    #[test]
    fn exhausted_chain_returns_all_attempts() {
        let mut chain = CodeIntelligenceChain::new();
        chain.push(UnsupportedProvider("serena"));
        chain.push(UnsupportedProvider("lsp"));

        let error = chain
            .semantic_query(&SemanticQuery::new(
                SemanticQueryKind::References,
                "target",
            ))
            .unwrap_err();

        assert_eq!(error.operation, "semantic query");
        assert_eq!(error.attempts.len(), 2);
        assert!(error
            .attempts
            .iter()
            .all(|attempt| attempt.status == ProviderAttemptStatus::Unsupported));
    }
}
