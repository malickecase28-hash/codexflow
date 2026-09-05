use anyhow::Context;
use anyhow::Result;
use clap::Args;
use clap::Subcommand;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

const SNAPSHOT_SCHEMA: &str = "codexflow.project-snapshot.v1";
const CONTEXT_SCHEMA: &str = "codexflow.context.v1";
const CACHE_TTL: Duration = Duration::from_secs(30);
const MAX_SCAN_FILES: usize = 50_000;
const MAX_DIR_DEPTH: usize = 12;
const MAX_CHANGED_PATHS: usize = 200;
const MAX_SKILL_DEPTH: usize = 4;
const MAX_PROJECT_DOC_CHARS: usize = 4_000;

#[derive(Debug, Args)]
pub struct ContextArgs {
    #[arg(long)]
    pub project: Option<String>,
    #[command(subcommand)]
    command: ContextCommand,
}

#[derive(Debug, Subcommand)]
enum ContextCommand {
    /// Build or reuse the deterministic project snapshot.
    Snapshot {
        #[arg(long)]
        refresh: bool,
    },
    /// Render a bounded repository map without reading file bodies.
    Map {
        #[arg(long, default_value_t = 120)]
        max_entries: usize,
        #[arg(long)]
        refresh: bool,
    },
    /// Assemble the bounded model-visible context envelope for a task.
    Assemble {
        #[arg(long)]
        task: Option<String>,
        #[arg(long, default_value_t = 16_000)]
        max_chars: usize,
        #[arg(long)]
        refresh: bool,
    },
    /// Rank lazily discoverable skills for the supplied task.
    Skills {
        #[arg(long)]
        task: Option<String>,
        #[arg(long, default_value_t = 12)]
        max: usize,
    },
    /// Remove the project snapshot cache.
    Invalidate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProjectSnapshot {
    schema: String,
    root: String,
    generated_at: String,
    git_branch: Option<String>,
    git_head: Option<String>,
    dirty_paths: Vec<String>,
    top_level: Vec<String>,
    manifests: Vec<String>,
    languages: BTreeMap<String, usize>,
    file_count: usize,
    truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ProjectMap {
    schema: &'static str,
    root: String,
    entries: Vec<String>,
    truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ContextEnvelope {
    schema: &'static str,
    snapshot: ProjectSnapshot,
    project_docs: Vec<String>,
    selected_skills: Vec<SkillCard>,
    prompt_chars: usize,
    prompt: String,
    truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
struct SkillCard {
    name: String,
    description: String,
    path: String,
    score: usize,
}

#[derive(Debug, Default)]
struct ScanSummary {
    top_level: BTreeSet<String>,
    manifests: BTreeSet<String>,
    languages: BTreeMap<String, usize>,
    file_count: usize,
    truncated: bool,
}

pub fn handle(project_root: &Path, args: ContextArgs) -> Result<()> {
    match args.command {
        ContextCommand::Snapshot { refresh } => {
            let snapshot = load_snapshot(project_root, refresh)?;
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
            Ok(())
        }
        ContextCommand::Map {
            max_entries,
            refresh,
        } => {
            let _ = load_snapshot(project_root, refresh)?;
            let map = build_project_map(project_root, max_entries)?;
            println!("{}", serde_json::to_string_pretty(&map)?);
            Ok(())
        }
        ContextCommand::Assemble {
            task,
            max_chars,
            refresh,
        } => {
            let envelope = assemble(project_root, task.as_deref(), max_chars, refresh)?;
            println!("{}", serde_json::to_string_pretty(&envelope)?);
            Ok(())
        }
        ContextCommand::Skills { task, max } => {
            let cards = rank_skills(project_root, task.as_deref(), max)?;
            println!("{}", serde_json::to_string_pretty(&cards)?);
            Ok(())
        }
        ContextCommand::Invalidate => {
            let path = snapshot_path(project_root);
            if path.exists() {
                fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
            }
            println!("{}", path.display());
            Ok(())
        }
    }
}

pub fn assemble_run_instructions(
    project_root: &Path,
    base: &str,
    task: Option<&str>,
    max_context_chars: usize,
) -> Result<String> {
    let envelope = assemble(project_root, task, max_context_chars, false)?;
    let mut output = String::with_capacity(base.len() + envelope.prompt.len() + 2);
    output.push_str(base);
    if !envelope.prompt.is_empty() {
        output.push_str("\n\n");
        output.push_str(&envelope.prompt);
    }
    Ok(output)
}

fn assemble(
    project_root: &Path,
    task: Option<&str>,
    max_chars: usize,
    refresh: bool,
) -> Result<ContextEnvelope> {
    let max_chars = max_chars.clamp(2_000, 64_000);
    let snapshot = load_snapshot(project_root, refresh)?;
    let project_docs = discover_project_docs(project_root);
    let selected_skills = if task.is_some() {
        rank_skills(project_root, task, 4)?
    } else {
        Vec::new()
    };

    let mut prompt = String::new();
    push_bounded(&mut prompt, &render_snapshot(&snapshot), max_chars);

    for relative in &project_docs {
        if prompt.len() >= max_chars {
            break;
        }
        let path = project_root.join(relative);
        let text = fs::read_to_string(&path).unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        let body = truncate_chars(&text, MAX_PROJECT_DOC_CHARS);
        let section = format!("\n\n[Project reference: {relative}]\n{body}");
        push_bounded(&mut prompt, &section, max_chars);
    }

    if !selected_skills.is_empty() && prompt.len() < max_chars {
        let cards = selected_skills
            .iter()
            .map(|card| format!("- {}: {} ({})", card.name, card.description, card.path))
            .collect::<Vec<_>>()
            .join("\n");
        let section = format!(
            "\n\n[Lazy specialist modules selected]\n{cards}\nLoad full skill instructions only through the native skill runtime when the task actually needs them."
        );
        push_bounded(&mut prompt, &section, max_chars);
    }

    let truncated = prompt.len() >= max_chars;
    Ok(ContextEnvelope {
        schema: CONTEXT_SCHEMA,
        snapshot,
        project_docs,
        selected_skills,
        prompt_chars: prompt.len(),
        prompt,
        truncated,
    })
}

fn render_snapshot(snapshot: &ProjectSnapshot) -> String {
    let languages = snapshot
        .languages
        .iter()
        .take(12)
        .map(|(name, count)| format!("{name}:{count}"))
        .collect::<Vec<_>>()
        .join(", ");
    let manifests = snapshot
        .manifests
        .iter()
        .take(24)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let changed = snapshot
        .dirty_paths
        .iter()
        .take(40)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let top_level = snapshot
        .top_level
        .iter()
        .take(40)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "[CodexFlow deterministic project context]\n\
         Root: {root}\n\
         Git branch: {branch}\n\
         Git head: {head}\n\
         Files scanned: {files}{scan_suffix}\n\
         Languages: {languages}\n\
         Manifests: {manifests}\n\
         Top-level map: {top_level}\n\
         Changed paths: {changed}\n\
         Policy: search before broad reads; read authoritative targets before editing; keep edits bounded; verify before completion.\
         ",
        root = snapshot.root,
        branch = snapshot.git_branch.as_deref().unwrap_or("<unknown>"),
        head = snapshot.git_head.as_deref().unwrap_or("<unknown>"),
        files = snapshot.file_count,
        scan_suffix = if snapshot.truncated {
            " (scan capped)"
        } else {
            ""
        },
        languages = if languages.is_empty() {
            "<none>"
        } else {
            &languages
        },
        manifests = if manifests.is_empty() {
            "<none>"
        } else {
            &manifests
        },
        top_level = if top_level.is_empty() {
            "<none>"
        } else {
            &top_level
        },
        changed = if changed.is_empty() {
            "<clean or unavailable>"
        } else {
            &changed
        },
    )
}

fn load_snapshot(project_root: &Path, refresh: bool) -> Result<ProjectSnapshot> {
    let path = snapshot_path(project_root);
    if !refresh && cache_is_fresh(&path) {
        if let Ok(data) = fs::read_to_string(&path)
            && let Ok(snapshot) = serde_json::from_str::<ProjectSnapshot>(&data)
            && snapshot.schema == SNAPSHOT_SCHEMA
            && snapshot.root == project_root.display().to_string()
        {
            return Ok(snapshot);
        }
    }

    let snapshot = scan_project(project_root)?;
    fs::create_dir_all(path.parent().context("snapshot cache parent")?)?;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&tmp, serde_json::to_vec_pretty(&snapshot)?)
        .with_context(|| format!("write {}", tmp.display()))?;
    replace_file(&tmp, &path)?;
    Ok(snapshot)
}

fn scan_project(project_root: &Path) -> Result<ProjectSnapshot> {
    let summary = if let Some((paths, truncated)) = git_project_paths(project_root) {
        summarize_paths(paths.iter().map(String::as_str), truncated)
    } else {
        scan_filesystem(project_root)?
    };

    Ok(ProjectSnapshot {
        schema: SNAPSHOT_SCHEMA.to_string(),
        root: project_root.display().to_string(),
        generated_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        git_branch: git_output(project_root, &["branch", "--show-current"]),
        git_head: git_output(project_root, &["rev-parse", "HEAD"]),
        dirty_paths: git_dirty_paths(project_root),
        top_level: summary.top_level.into_iter().collect(),
        manifests: summary.manifests.into_iter().collect(),
        languages: summary.languages,
        file_count: summary.file_count,
        truncated: summary.truncated,
    })
}

fn git_project_paths(project_root: &Path) -> Option<(Vec<String>, bool)> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let mut paths = Vec::with_capacity(output.stdout.len().min(MAX_SCAN_FILES));
    let mut truncated = false;
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        if paths.len() >= MAX_SCAN_FILES {
            truncated = true;
            break;
        }
        let relative = String::from_utf8_lossy(raw).replace('\\', "/");
        if relative.is_empty() || should_skip_relative_path(&relative) {
            continue;
        }
        paths.push(relative);
    }
    paths.sort_unstable();
    paths.dedup();
    Some((paths, truncated))
}

fn scan_filesystem(project_root: &Path) -> Result<ScanSummary> {
    let mut summary = ScanSummary::default();
    let mut stack = vec![(project_root.to_path_buf(), 0usize)];

    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_DIR_DEPTH {
            continue;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) if dir != project_root => continue,
            Err(err) => return Err(err).with_context(|| format!("read {}", dir.display())),
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(_) => continue,
            };
            let path = entry.path();
            if kind.is_dir() {
                if should_skip_dir(entry.file_name().as_os_str()) {
                    continue;
                }
                stack.push((path, depth + 1));
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            if summary.file_count >= MAX_SCAN_FILES {
                summary.truncated = true;
                break;
            }
            let relative = match path.strip_prefix(project_root) {
                Ok(value) => normalize_relative(value),
                Err(_) => continue,
            };
            record_path(&mut summary, &relative);
        }
        if summary.truncated {
            break;
        }
    }
    Ok(summary)
}

fn summarize_paths<'a>(paths: impl Iterator<Item = &'a str>, truncated: bool) -> ScanSummary {
    let mut summary = ScanSummary {
        truncated,
        ..ScanSummary::default()
    };
    for relative in paths {
        record_path(&mut summary, relative);
    }
    summary
}

fn record_path(summary: &mut ScanSummary, relative: &str) {
    summary.file_count = summary.file_count.saturating_add(1);
    if let Some((first, _)) = relative.split_once('/') {
        summary.top_level.insert(format!("{first}/"));
    } else {
        summary.top_level.insert(relative.to_string());
    }
    let path = Path::new(relative);
    if is_manifest(path) {
        summary.manifests.insert(relative.to_string());
    }
    if let Some(language) = language_for(path) {
        *summary.languages.entry(language.to_string()).or_insert(0) += 1;
    }
}

fn build_project_map(project_root: &Path, max_entries: usize) -> Result<ProjectMap> {
    let limit = max_entries.clamp(20, 2_000);
    if let Some((paths, scan_truncated)) = git_project_paths(project_root) {
        return Ok(project_map_from_paths(
            project_root,
            &paths,
            limit,
            scan_truncated,
        ));
    }
    build_project_map_filesystem(project_root, limit)
}

fn project_map_from_paths(
    project_root: &Path,
    paths: &[String],
    limit: usize,
    scan_truncated: bool,
) -> ProjectMap {
    let mut entries = BTreeSet::new();
    for relative in paths {
        let components = relative.split('/').collect::<Vec<_>>();
        let parent_depth = components.len().saturating_sub(1);
        for end in 1..=parent_depth.min(5) {
            entries.insert(format!("{}/", components[..end].join("/")));
        }
        if important_map_file(Path::new(relative), parent_depth) {
            entries.insert(relative.clone());
        }
    }

    let truncated = scan_truncated || entries.len() > limit;
    ProjectMap {
        schema: "codexflow.project-map.v1",
        root: project_root.display().to_string(),
        entries: entries.into_iter().take(limit).collect(),
        truncated,
    }
}

fn build_project_map_filesystem(project_root: &Path, limit: usize) -> Result<ProjectMap> {
    let mut entries = BTreeSet::new();
    let mut scan_truncated = false;
    let mut stack = vec![(project_root.to_path_buf(), 0usize)];

    while let Some((dir, depth)) = stack.pop() {
        if depth > 4 {
            scan_truncated = true;
            continue;
        }
        let read = match fs::read_dir(&dir) {
            Ok(read) => read,
            Err(_) if dir != project_root => continue,
            Err(err) => return Err(err).with_context(|| format!("read {}", dir.display())),
        };
        for entry in read {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(_) => continue,
            };
            let path = entry.path();
            let relative = match path.strip_prefix(project_root) {
                Ok(value) => normalize_relative(value),
                Err(_) => continue,
            };
            if kind.is_dir() {
                if should_skip_dir(entry.file_name().as_os_str()) {
                    continue;
                }
                entries.insert(format!("{relative}/"));
                stack.push((path, depth + 1));
            } else if kind.is_file() && important_map_file(&path, depth) {
                entries.insert(relative);
            }
        }
    }

    let truncated = scan_truncated || entries.len() > limit;
    Ok(ProjectMap {
        schema: "codexflow.project-map.v1",
        root: project_root.display().to_string(),
        entries: entries.into_iter().take(limit).collect(),
        truncated,
    })
}

fn rank_skills(project_root: &Path, task: Option<&str>, max: usize) -> Result<Vec<SkillCard>> {
    let tokens = task.map(task_tokens).unwrap_or_default();
    let mut cards = discover_skills(project_root)?;
    for card in &mut cards {
        card.score = score_skill(card, &tokens);
    }
    if task.is_some() {
        cards.retain(|card| card.score > 0);
        cards.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.name.cmp(&right.name))
        });
    } else {
        cards.sort_by(|left, right| left.name.cmp(&right.name));
    }
    cards.truncate(max.clamp(1, 100));
    Ok(cards)
}

fn discover_skills(project_root: &Path) -> Result<Vec<SkillCard>> {
    let mut roots = vec![
        project_root.join(".codex").join("skills"),
        project_root.join(".claude").join("skills"),
        project_root.join("skills"),
    ];
    if let Some(home) = std::env::var_os("CODEX_HOME") {
        roots.push(PathBuf::from(home).join("skills"));
    }

    let mut cards_by_name = BTreeMap::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        collect_skill_cards(&root, 0, &mut cards_by_name)?;
    }
    Ok(cards_by_name.into_values().collect())
}

fn collect_skill_cards(
    root: &Path,
    depth: usize,
    cards: &mut BTreeMap<String, SkillCard>,
) -> Result<()> {
    if depth > MAX_SKILL_DEPTH {
        return Ok(());
    }
    for entry in fs::read_dir(root).with_context(|| format!("read {}", root.display()))? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_skill_cards(&path, depth + 1, cards)?;
            continue;
        }
        if entry.file_name().to_string_lossy() != "SKILL.md" {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap_or_default();
        let (name, description) = parse_skill_frontmatter(&text, &path);
        cards.entry(name.clone()).or_insert(SkillCard {
            name,
            description,
            path: path.display().to_string(),
            score: 0,
        });
    }
    Ok(())
}

fn parse_skill_frontmatter(text: &str, path: &Path) -> (String, String) {
    let mut name = None;
    let mut description = None;
    let mut in_frontmatter = false;
    for line in text.lines().take(40) {
        let trimmed = line.trim();
        if trimmed == "---" {
            if in_frontmatter {
                break;
            }
            in_frontmatter = true;
            continue;
        }
        if !in_frontmatter {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("name:") {
            name = Some(clean_yaml_scalar(value));
        } else if let Some(value) = trimmed.strip_prefix("description:") {
            description = Some(clean_yaml_scalar(value));
        }
    }
    let fallback = path
        .parent()
        .and_then(Path::file_name)
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unnamed-skill".to_string());
    (
        name.filter(|value| !value.is_empty()).unwrap_or(fallback),
        description.unwrap_or_default(),
    )
}

fn clean_yaml_scalar(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string()
}

fn task_tokens(task: &str) -> BTreeSet<String> {
    task.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-')
        .map(str::to_ascii_lowercase)
        .filter(|value| value.len() >= 3)
        .collect()
}

fn score_skill(card: &SkillCard, tokens: &BTreeSet<String>) -> usize {
    if tokens.is_empty() {
        return 0;
    }
    let name = card.name.to_ascii_lowercase();
    let description = card.description.to_ascii_lowercase();
    let path = card.path.to_ascii_lowercase();
    let mut score = 0usize;
    for token in tokens {
        if name.as_str() == token.as_str() {
            score += 20;
        } else if name.contains(token) {
            score += 8;
        }
        if description.contains(token) {
            score += 3;
        }
        if path.contains(token) {
            score += 1;
        }
    }
    score
}

fn discover_project_docs(project_root: &Path) -> Vec<String> {
    let candidates = [
        "CONTEXT.md",
        "docs/architecture/README.md",
        "docs/ARCHITECTURE.md",
    ];
    let mut found = Vec::new();
    for relative in candidates {
        if project_root.join(relative).is_file() {
            found.push(relative.to_string());
        }
    }
    found
}

fn snapshot_path(project_root: &Path) -> PathBuf {
    project_root
        .join(".codexflow")
        .join("cache")
        .join("project-snapshot-v1.json")
}

fn cache_is_fresh(path: &Path) -> bool {
    path.metadata()
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|elapsed| elapsed < CACHE_TTL)
}

fn replace_file(tmp: &Path, destination: &Path) -> Result<()> {
    if cfg!(windows) && destination.exists() {
        fs::remove_file(destination)
            .with_context(|| format!("remove {}", destination.display()))?;
    }
    fs::rename(tmp, destination)
        .with_context(|| format!("replace {}", destination.display()))
}

fn git_output(project_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn git_dirty_paths(project_root: &Path) -> Vec<String> {
    let Some(output) = git_output(
        project_root,
        &["status", "--porcelain=v1", "--untracked-files=normal"],
    ) else {
        return Vec::new();
    };
    output
        .lines()
        .take(MAX_CHANGED_PATHS)
        .filter_map(|line| line.get(3..))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn is_manifest(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(OsStr::to_str),
        Some(
            "Cargo.toml"
                | "package.json"
                | "pyproject.toml"
                | "go.mod"
                | "pom.xml"
                | "build.gradle"
                | "build.gradle.kts"
                | "MODULE.bazel"
                | "WORKSPACE"
                | "WORKSPACE.bazel"
                | "Makefile"
                | "justfile"
        )
    )
}

fn important_map_file(path: &Path, depth: usize) -> bool {
    if depth <= 1 {
        return true;
    }
    if is_manifest(path) {
        return true;
    }
    matches!(
        path.file_name().and_then(OsStr::to_str),
        Some(
            "README.md"
                | "AGENTS.md"
                | "CONTEXT.md"
                | "CLAUDE.md"
                | "BUILD"
                | "BUILD.bazel"
                | "mod.rs"
                | "lib.rs"
                | "main.rs"
        )
    )
}

fn language_for(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(OsStr::to_str)?
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => Some("rust"),
        "ts" | "tsx" => Some("typescript"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "py" => Some("python"),
        "go" => Some("go"),
        "java" => Some("java"),
        "kt" | "kts" => Some("kotlin"),
        "c" | "h" => Some("c"),
        "cc" | "cpp" | "cxx" | "hpp" | "hxx" => Some("cpp"),
        "cs" => Some("csharp"),
        "swift" => Some("swift"),
        "rb" => Some("ruby"),
        "php" => Some("php"),
        "scala" => Some("scala"),
        "sh" | "bash" | "zsh" => Some("shell"),
        "md" => Some("markdown"),
        "toml" => Some("toml"),
        "yaml" | "yml" => Some("yaml"),
        "json" => Some("json"),
        _ => None,
    }
}

fn normalize_relative(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn should_skip_dir(name: &OsStr) -> bool {
    let name = name.to_string_lossy();
    should_skip_name(&name)
}

fn should_skip_relative_path(path: &str) -> bool {
    path.split('/').any(should_skip_name)
}

fn should_skip_name(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".hg"
            | ".svn"
            | ".codexflow"
            | "target"
            | "node_modules"
            | ".next"
            | ".turbo"
            | ".cache"
            | "dist"
            | "build"
            | "coverage"
            | "__pycache__"
            | ".venv"
            | "venv"
    )
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}

fn push_bounded(target: &mut String, addition: &str, max_chars: usize) {
    if target.len() >= max_chars {
        return;
    }
    let remaining = max_chars - target.len();
    if addition.len() <= remaining {
        target.push_str(addition);
        return;
    }
    let mut end = remaining.min(addition.len());
    while !addition.is_char_boundary(end) {
        end -= 1;
    }
    target.push_str(&addition[..end]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_context_seam_discovers_project_and_lazy_skill() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(temp.path().join("Cargo.toml"), "[package]\nname='demo'\n").expect("manifest");
        fs::write(
            temp.path().join("CONTEXT.md"),
            "Event ledger is canonical state.",
        )
        .expect("context");
        let skill_dir = temp.path().join(".codex/skills/debugging");
        fs::create_dir_all(&skill_dir).expect("skill dir");
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: diagnosing-bugs\ndescription: Debug failing and slow software with a tight feedback loop.\n---\nUse a red-capable reproduction first.",
        )
        .expect("skill");

        let envelope = assemble(
            temp.path(),
            Some("debug a failing rust command"),
            12_000,
            true,
        )
        .expect("assemble");
        assert_eq!(envelope.snapshot.languages.get("toml"), Some(&1));
        assert!(envelope.prompt.contains("Event ledger is canonical state."));
        assert!(envelope.prompt.contains("diagnosing-bugs"));
        assert_eq!(envelope.selected_skills.len(), 1);
    }

    #[test]
    fn project_map_skips_build_surfaces() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("src")).expect("src");
        fs::create_dir_all(temp.path().join("target/generated")).expect("target");
        fs::write(temp.path().join("src/lib.rs"), "pub fn run() {}").expect("source");
        fs::write(temp.path().join("target/generated/junk.rs"), "fn junk() {}").expect("junk");

        let map = build_project_map(temp.path(), 100).expect("map");
        assert!(map.entries.iter().any(|entry| entry == "src/"));
        assert!(
            !map.entries
                .iter()
                .any(|entry| entry.starts_with("target/"))
        );
    }

    #[test]
    fn git_index_scan_includes_untracked_and_excludes_ignored_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        run_git(temp.path(), &["init", "-q"]);
        fs::create_dir_all(temp.path().join("src")).expect("src");
        fs::create_dir_all(temp.path().join("target/generated")).expect("target");
        fs::write(temp.path().join(".gitignore"), "target/\n").expect("gitignore");
        fs::write(temp.path().join("src/lib.rs"), "pub fn run() {}").expect("source");
        fs::write(temp.path().join("scratch.py"), "print('hi')\n").expect("scratch");
        fs::write(temp.path().join("target/generated/junk.rs"), "fn junk() {}").expect("junk");
        run_git(temp.path(), &["add", ".gitignore", "src/lib.rs"]);

        let snapshot = scan_project(temp.path()).expect("snapshot");
        assert_eq!(snapshot.languages.get("rust"), Some(&1));
        assert_eq!(snapshot.languages.get("python"), Some(&1));
        assert_eq!(snapshot.file_count, 3);
        assert!(snapshot.top_level.iter().any(|entry| entry == "src/"));
        assert!(snapshot.dirty_paths.iter().any(|entry| entry == "scratch.py"));

        let map = build_project_map(temp.path(), 100).expect("map");
        assert!(map.entries.iter().any(|entry| entry == "scratch.py"));
        assert!(
            !map.entries
                .iter()
                .any(|entry| entry.starts_with("target/"))
        );
    }

    #[test]
    fn skill_scoring_prefers_semantic_matches() {
        let tokens = task_tokens("debug a failing performance regression");
        let strong = SkillCard {
            name: "diagnosing-bugs".to_string(),
            description: "Debug failing software and performance regressions".to_string(),
            path: "/skills/diagnosing-bugs/SKILL.md".to_string(),
            score: 0,
        };
        let weak = SkillCard {
            name: "writing".to_string(),
            description: "Improve prose".to_string(),
            path: "/skills/writing/SKILL.md".to_string(),
            score: 0,
        };
        assert!(score_skill(&strong, &tokens) > score_skill(&weak, &tokens));
    }

    fn run_git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .expect("git command");
        assert!(status.success(), "git command failed: {args:?}");
    }
}
