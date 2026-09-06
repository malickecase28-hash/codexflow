use serde::Deserialize;
use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct RepositoryMapRequest {
    pub roots: Vec<PathBuf>,
    pub focus_terms: Vec<String>,
    pub changed_files: BTreeSet<PathBuf>,
    pub max_entries: usize,
    pub max_bytes: usize,
    pub include_members: bool,
}

impl Default for RepositoryMapRequest {
    fn default() -> Self {
        Self {
            roots: vec![PathBuf::from(".")],
            focus_terms: Vec::new(),
            changed_files: BTreeSet::new(),
            max_entries: 200,
            max_bytes: 24 * 1024,
            include_members: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryMapEntry {
    pub path: PathBuf,
    pub language: String,
    pub name: String,
    pub symbol_type: String,
    pub signature: String,
    pub line: u32,
    pub parent: Option<String>,
    pub score: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryMap {
    pub rendered: String,
    pub entries: Vec<RepositoryMapEntry>,
    pub discovered_entries: usize,
    pub omitted_entries: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum RepositoryMapError {
    #[error("failed to execute ast-grep outline at {executable}: {source}")]
    Execute {
        executable: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("ast-grep outline exited with {status}: {stderr}")]
    CommandFailed { status: String, stderr: String },
    #[error("invalid ast-grep outline JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct AstGrepRepositoryMapper {
    executable: PathBuf,
    working_directory: PathBuf,
}

impl AstGrepRepositoryMapper {
    pub fn new(executable: impl Into<PathBuf>, working_directory: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            working_directory: working_directory.into(),
        }
    }

    pub fn build(&self, request: &RepositoryMapRequest) -> Result<RepositoryMap, RepositoryMapError> {
        let args = outline_args(request);
        let output = Command::new(&self.executable)
            .args(&args)
            .current_dir(&self.working_directory)
            .output()
            .map_err(|source| RepositoryMapError::Execute {
                executable: self.executable.clone(),
                source,
            })?;
        if !output.status.success() {
            return Err(RepositoryMapError::CommandFailed {
                status: output.status.to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }

        let files: Vec<OutlineFile> = serde_json::from_slice(&output.stdout)?;
        Ok(build_repository_map(files, request))
    }
}

fn outline_args(request: &RepositoryMapRequest) -> Vec<String> {
    let mut args = vec![
        "outline".to_string(),
        "--items=structure".to_string(),
        "--view=digest".to_string(),
        "--pub-members".to_string(),
        "--json=compact".to_string(),
        "--color=never".to_string(),
        "--threads=1".to_string(),
    ];
    if request.roots.is_empty() {
        args.push(".".to_string());
    } else {
        args.extend(
            request
                .roots
                .iter()
                .map(|path| path.to_string_lossy().into_owned()),
        );
    }
    args
}

fn build_repository_map(files: Vec<OutlineFile>, request: &RepositoryMapRequest) -> RepositoryMap {
    let focus_terms = request
        .focus_terms
        .iter()
        .map(|term| term.to_ascii_lowercase())
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>();
    let mut entries = Vec::new();

    for file in files {
        let changed = request.changed_files.contains(&file.path);
        for item in file.items {
            let top_score = score_entry(
                &file.path,
                &item.name,
                &item.signature,
                &item.symbol_type,
                item.is_exported,
                changed,
                &focus_terms,
            );
            entries.push(RepositoryMapEntry {
                path: file.path.clone(),
                language: file.language.clone(),
                name: item.name.clone(),
                symbol_type: item.symbol_type.clone(),
                signature: item.signature.clone(),
                line: item.range.start.line,
                parent: None,
                score: top_score,
            });

            if request.include_members {
                for member in item.members {
                    let member_score = score_entry(
                        &file.path,
                        &member.name,
                        &member.signature,
                        &member.symbol_type,
                        member.is_public || item.is_exported,
                        changed,
                        &focus_terms,
                    ) - 2;
                    entries.push(RepositoryMapEntry {
                        path: file.path.clone(),
                        language: file.language.clone(),
                        name: member.name,
                        symbol_type: member.symbol_type,
                        signature: member.signature,
                        line: member.range.start.line,
                        parent: Some(item.name.clone()),
                        score: member_score,
                    });
                }
            }
        }
    }

    entries.sort_by_key(|entry| {
        (
            Reverse(entry.score),
            entry.path.clone(),
            entry.line,
            entry.name.clone(),
        )
    });
    let discovered_entries = entries.len();
    let mut selected = Vec::new();
    let mut rendered_lines = Vec::new();
    let mut used_bytes = 0usize;

    for entry in entries {
        if selected.len() >= request.max_entries {
            break;
        }
        let line = render_entry(&entry);
        let separator = usize::from(!rendered_lines.is_empty());
        if used_bytes.saturating_add(separator).saturating_add(line.len()) > request.max_bytes {
            continue;
        }
        used_bytes += separator + line.len();
        rendered_lines.push(line);
        selected.push(entry);
    }

    RepositoryMap {
        rendered: rendered_lines.join("\n"),
        omitted_entries: discovered_entries.saturating_sub(selected.len()),
        entries: selected,
        discovered_entries,
    }
}

fn score_entry(
    path: &PathBuf,
    name: &str,
    signature: &str,
    symbol_type: &str,
    externally_visible: bool,
    changed: bool,
    focus_terms: &[String],
) -> i32 {
    let mut score = match symbol_type {
        "class" | "struct" | "interface" | "trait" | "enum" | "module" => 18,
        "function" | "method" | "constructor" => 14,
        "typeAlias" | "constant" | "static" => 10,
        _ => 6,
    };
    if externally_visible {
        score += 12;
    }
    if changed {
        score += 30;
    }

    let haystack = format!("{} {name} {signature}", path.display()).to_ascii_lowercase();
    for term in focus_terms {
        if haystack.contains(term) {
            score += 20;
        }
    }
    score
}

fn render_entry(entry: &RepositoryMapEntry) -> String {
    let parent = entry
        .parent
        .as_ref()
        .map(|parent| format!("{parent}::"))
        .unwrap_or_default();
    format!(
        "{}:{} [{}] {}{} — {}",
        entry.path.display(),
        entry.line.saturating_add(1),
        entry.symbol_type,
        parent,
        entry.name,
        entry.signature
    )
}

#[derive(Debug, Deserialize)]
struct OutlineFile {
    path: PathBuf,
    language: String,
    #[serde(default)]
    items: Vec<OutlineItem>,
}

#[derive(Debug, Deserialize)]
struct OutlineItem {
    name: String,
    #[serde(rename = "symbolType")]
    symbol_type: String,
    range: OutlineRange,
    signature: String,
    #[serde(rename = "isExported", default)]
    is_exported: bool,
    #[serde(default)]
    members: Vec<OutlineMember>,
}

#[derive(Debug, Deserialize)]
struct OutlineMember {
    name: String,
    #[serde(rename = "symbolType")]
    symbol_type: String,
    range: OutlineRange,
    signature: String,
    #[serde(rename = "isPublic", default)]
    is_public: bool,
}

#[derive(Debug, Deserialize)]
struct OutlineRange {
    start: OutlinePosition,
}

#[derive(Debug, Deserialize)]
struct OutlinePosition {
    line: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<OutlineFile> {
        serde_json::from_str(
            r#"[{"path":"src/parser.ts","language":"TypeScript","items":[{"name":"Parser","symbolType":"class","role":"item","isImport":false,"isExported":true,"range":{"byteOffset":{"start":1200,"end":2500},"start":{"line":39,"column":0},"end":{"line":97,"column":1}},"signature":"export class Parser","astKind":"class_declaration","members":[{"name":"parse","symbolType":"method","role":"member","isPublic":true,"range":{"byteOffset":{"start":1300,"end":1900},"start":{"line":43,"column":2},"end":{"line":71,"column":3}},"signature":"parse(...) ","astKind":"method_definition"}]}]}]"#,
        )
        .unwrap()
    }

    #[test]
    fn outline_command_is_compact_and_non_source-dumping() {
        let args = outline_args(&RepositoryMapRequest::default());
        assert!(args.contains(&"outline".to_string()));
        assert!(args.contains(&"--view=digest".to_string()));
        assert!(args.contains(&"--json=compact".to_string()));
        assert!(args.contains(&"--pub-members".to_string()));
    }

    #[test]
    fn changed_and_focus_relevance_drive_ranking() {
        let mut request = RepositoryMapRequest::default();
        request.focus_terms.push("parse".to_string());
        request.changed_files.insert(PathBuf::from("src/parser.ts"));
        let map = build_repository_map(sample(), &request);

        assert_eq!(map.discovered_entries, 2);
        assert_eq!(map.entries[0].name, "Parser");
        assert!(map.entries.iter().all(|entry| entry.score >= 30));
        assert!(map.rendered.contains("src/parser.ts:40"));
        assert!(map.rendered.contains("Parser::parse"));
    }

    #[test]
    fn byte_and_entry_budgets_are_hard_caps() {
        let request = RepositoryMapRequest {
            max_entries: 1,
            max_bytes: 10_000,
            ..Default::default()
        };
        let map = build_repository_map(sample(), &request);
        assert_eq!(map.entries.len(), 1);
        assert_eq!(map.omitted_entries, 1);

        let tiny = RepositoryMapRequest {
            max_entries: 10,
            max_bytes: 1,
            ..Default::default()
        };
        let map = build_repository_map(sample(), &tiny);
        assert!(map.entries.is_empty());
        assert_eq!(map.omitted_entries, 2);
    }
}
