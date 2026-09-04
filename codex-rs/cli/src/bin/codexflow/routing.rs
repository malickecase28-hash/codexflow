mod recovery_ledger;
mod recovery_replay;
mod recovery_report;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use clap::Args;
use clap::Subcommand;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

const ROUTING_SCHEMA: &str = "codexflow.routing.v1";
const PROFILE_FAST: &str = "fast";
const PROFILE_BALANCED: &str = "balanced";
const PROFILE_DEEP: &str = "deep";
const PROFILE_CRITICAL: &str = "critical";

const FAILURE_RETRIEVAL: &str = "retrieval";
const FAILURE_TOOL_SELECTION: &str = "tool_selection";
const FAILURE_INVALID_ARGUMENTS: &str = "invalid_arguments";
const FAILURE_MISSING_DEPENDENCY: &str = "missing_dependency";
const FAILURE_CONTEXT_INSUFFICIENCY: &str = "context_insufficiency";
const FAILURE_REASONING: &str = "reasoning";
const FAILURE_TEST: &str = "test";
const FAILURE_PERMISSION: &str = "permission";
const FAILURE_TIMEOUT: &str = "timeout";
const FAILURE_AMBIGUOUS_REQUIREMENT: &str = "ambiguous_requirement";

#[derive(Debug, Args)]
pub struct RoutingArgs {
    #[arg(long)]
    pub project: Option<String>,
    #[command(subcommand)]
    command: RoutingCommand,
}

#[derive(Debug, Subcommand)]
enum RoutingCommand {
    /// Create a project-local adaptive routing policy.
    Init {
        #[arg(long)]
        force: bool,
    },
    /// Show the effective routing policy.
    Show,
    /// Classify a task and show the selected model/budget profile.
    Classify {
        #[arg(long)]
        task: String,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Select and durably record a deterministic recovery path for a failed attempt.
    Recover {
        /// Failure class: retrieval, tool_selection, invalid_arguments,
        /// missing_dependency, context_insufficiency, reasoning, test,
        /// permission, timeout, or ambiguous_requirement.
        #[arg(long)]
        failure: String,
        /// One-based number of the failed attempt for the same objective.
        #[arg(long, default_value_t = 1)]
        attempt: u32,
        /// Current adaptive route profile. Defaults to balanced.
        #[arg(long)]
        profile: Option<String>,
        /// Optional short diagnostic preserved in the recovery record.
        #[arg(long)]
        detail: Option<String>,
    },
    /// Show the most recent durable recovery records.
    History {
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Summarize durable recovery trajectories as deterministic metrics.
    Report {
        #[arg(long, default_value_t = 1000)]
        limit: usize,
    },
    /// Recompute a stored recovery event under the current routing policy.
    Replay {
        /// Zero selects the newest record, one selects the prior record.
        #[arg(long, default_value_t = 0)]
        offset: usize,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct RoutingPolicy {
    schema: String,
    enabled: bool,
    fast: RouteProfile,
    balanced: RouteProfile,
    deep: RouteProfile,
    critical: RouteProfile,
    escalation_failure_threshold: u32,
}

impl Default for RoutingPolicy {
    fn default() -> Self {
        Self {
            schema: ROUTING_SCHEMA.to_string(),
            enabled: true,
            fast: RouteProfile {
                model: None,
                reasoning_effort: "low".to_string(),
                max_context_chars: 10_000,
                max_tool_calls: 12,
                max_retries: 1,
                candidate_count: 1,
                verification_depth: "focused".to_string(),
                human_approval: false,
            },
            balanced: RouteProfile {
                model: None,
                reasoning_effort: "medium".to_string(),
                max_context_chars: 16_000,
                max_tool_calls: 24,
                max_retries: 2,
                candidate_count: 1,
                verification_depth: "standard".to_string(),
                human_approval: false,
            },
            deep: RouteProfile {
                model: None,
                reasoning_effort: "high".to_string(),
                max_context_chars: 24_000,
                max_tool_calls: 48,
                max_retries: 3,
                candidate_count: 2,
                verification_depth: "deep".to_string(),
                human_approval: false,
            },
            critical: RouteProfile {
                model: None,
                reasoning_effort: "xhigh".to_string(),
                max_context_chars: 32_000,
                max_tool_calls: 64,
                max_retries: 4,
                candidate_count: 3,
                verification_depth: "exhaustive".to_string(),
                human_approval: true,
            },
            escalation_failure_threshold: 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct RouteProfile {
    /// Optional backend model name. None inherits the Codex profile/default model.
    model: Option<String>,
    reasoning_effort: String,
    max_context_chars: usize,
    max_tool_calls: u32,
    max_retries: u32,
    candidate_count: u32,
    verification_depth: String,
    human_approval: bool,
}

impl Default for RouteProfile {
    fn default() -> Self {
        RoutingPolicy::default().balanced
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RouteDecision {
    pub schema: &'static str,
    pub profile: String,
    pub difficulty_score: u32,
    pub factors: Vec<String>,
    pub model: Option<String>,
    pub reasoning_effort: String,
    pub max_context_chars: usize,
    pub max_tool_calls: u32,
    pub max_retries: u32,
    pub candidate_count: u32,
    pub verification_depth: String,
    pub human_approval: bool,
    pub escalation_failure_threshold: u32,
}

#[derive(Debug, Clone, Serialize)]
struct RecoveryDecision {
    schema: &'static str,
    failure_class: String,
    attempt: u32,
    detail: Option<String>,
    current_profile: String,
    next_profile: String,
    action: String,
    retry_allowed: bool,
    strategy_change: bool,
    additional_retrieval: bool,
    rollback_recommended: bool,
    human_approval: bool,
    preserve_failure_evidence: bool,
    verification_depth: String,
}

pub fn handle(project_root: &Path, args: RoutingArgs) -> Result<()> {
    match args.command {
        RoutingCommand::Init { force } => init(project_root, force),
        RoutingCommand::Show => {
            println!(
                "{}",
                serde_json::to_string_pretty(&load_policy(project_root)?)?
            );
            Ok(())
        }
        RoutingCommand::Classify { task, profile } => {
            let decision = resolve_route(project_root, Some(&task), profile.as_deref())?;
            println!("{}", serde_json::to_string_pretty(&decision)?);
            Ok(())
        }
        RoutingCommand::Recover {
            failure,
            attempt,
            profile,
            detail,
        } => {
            let decision = resolve_recovery(
                project_root,
                &failure,
                attempt,
                profile.as_deref(),
                detail,
            )?;
            recovery_ledger::append(project_root, &decision)?;
            println!("{}", serde_json::to_string_pretty(&decision)?);
            Ok(())
        }
        RoutingCommand::History { limit } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&recovery_ledger::history(project_root, limit)?)?
            );
            Ok(())
        }
        RoutingCommand::Report { limit } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&recovery_report::build(project_root, limit)?)?
            );
            Ok(())
        }
        RoutingCommand::Replay { offset } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&recovery_replay::replay(project_root, offset)?)?
            );
            Ok(())
        }
    }
}

pub fn resolve_route(
    project_root: &Path,
    task: Option<&str>,
    explicit_profile: Option<&str>,
) -> Result<RouteDecision> {
    let policy = load_policy(project_root)?;
    validate_policy(&policy)?;

    let (difficulty_score, factors) = task
        .map(classify_task)
        .unwrap_or_else(|| (2, vec!["interactive session default".to_string()]));
    let profile_name = if !policy.enabled {
        PROFILE_BALANCED
    } else if let Some(profile) = explicit_profile {
        validate_profile_name(profile)?;
        profile
    } else {
        profile_for_score(difficulty_score)
    };
    let profile = policy_profile(&policy, profile_name);

    Ok(RouteDecision {
        schema: "codexflow.route-decision.v1",
        profile: profile_name.to_string(),
        difficulty_score,
        factors,
        model: profile.model.clone().filter(|value| !value.trim().is_empty()),
        reasoning_effort: profile.reasoning_effort.clone(),
        max_context_chars: profile.max_context_chars.clamp(2_000, 64_000),
        max_tool_calls: profile.max_tool_calls.max(1),
        max_retries: profile.max_retries,
        candidate_count: profile.candidate_count.max(1),
        verification_depth: profile.verification_depth.clone(),
        human_approval: profile.human_approval,
        escalation_failure_threshold: policy.escalation_failure_threshold.max(1),
    })
}

fn resolve_recovery(
    project_root: &Path,
    failure: &str,
    attempt: u32,
    current_profile: Option<&str>,
    detail: Option<String>,
) -> Result<RecoveryDecision> {
    if attempt == 0 {
        bail!("attempt must be at least 1");
    }
    let policy = load_policy(project_root)?;
    validate_policy(&policy)?;
    validate_failure_class(failure)?;
    let current_profile = current_profile.unwrap_or(PROFILE_BALANCED);
    validate_profile_name(current_profile)?;

    let threshold = policy.escalation_failure_threshold.max(1);
    let repeated = attempt >= threshold;
    let mut next_profile = current_profile.to_string();
    let mut retry_allowed = true;
    let mut strategy_change = repeated;
    let mut additional_retrieval = false;
    let mut rollback_recommended = false;
    let mut human_approval = false;

    let action = match failure {
        FAILURE_RETRIEVAL => {
            additional_retrieval = true;
            "broaden or rerank retrieval sources, preserve citations, then retry with targeted context"
        }
        FAILURE_TOOL_SELECTION => {
            "select a narrower or alternate tool from capability metadata; do not repeat the identical call"
        }
        FAILURE_INVALID_ARGUMENTS => {
            strategy_change = false;
            "repair arguments against the tool schema and retry without increasing model depth"
        }
        FAILURE_MISSING_DEPENDENCY => {
            retry_allowed = false;
            "resolve, install, or explicitly block on the missing dependency before another model attempt"
        }
        FAILURE_CONTEXT_INSUFFICIENCY => {
            additional_retrieval = true;
            "retrieve the missing authoritative files or sources and rebuild a bounded context envelope"
        }
        FAILURE_REASONING => {
            "replan from verified facts, isolate assumptions, and use a different reasoning strategy"
        }
        FAILURE_TEST => {
            rollback_recommended = repeated;
            "feed the exact failing test or compiler evidence into a targeted repair, then re-run verification"
        }
        FAILURE_PERMISSION => {
            retry_allowed = false;
            human_approval = true;
            strategy_change = false;
            "request the required permission or human approval; never bypass the boundary by retrying"
        }
        FAILURE_TIMEOUT => {
            "narrow the scope, reduce unnecessary parallelism, and retry the smallest independently verifiable step"
        }
        FAILURE_AMBIGUOUS_REQUIREMENT => {
            retry_allowed = false;
            human_approval = true;
            strategy_change = false;
            "obtain a concrete requirement or acceptance criterion before implementation continues"
        }
        _ => unreachable!("validated failure class"),
    };

    if repeated && retry_allowed {
        next_profile = escalate_profile(current_profile).to_string();
    }
    if next_profile == PROFILE_CRITICAL && repeated {
        human_approval |= policy.critical.human_approval;
    }
    let verification_depth = policy_profile(&policy, &next_profile)
        .verification_depth
        .clone();

    Ok(RecoveryDecision {
        schema: "codexflow.recovery-decision.v1",
        failure_class: failure.to_string(),
        attempt,
        detail: detail.filter(|value| !value.trim().is_empty()),
        current_profile: current_profile.to_string(),
        next_profile,
        action: action.to_string(),
        retry_allowed,
        strategy_change,
        additional_retrieval,
        rollback_recommended,
        human_approval,
        preserve_failure_evidence: true,
        verification_depth,
    })
}

pub fn render_route_instructions(decision: &RouteDecision) -> String {
    format!(
        "[CodexFlow adaptive route]\n\
         Profile: {profile}\n\
         Difficulty score: {score}\n\
         Tool-call budget: {tools}\n\
         Retry budget: {retries}\n\
         Candidate budget: {candidates}\n\
         Verification depth: {verification}\n\
         Human approval required before irreversible/high-impact completion: {approval}\n\
         Escalate or change strategy after {failures} repeated failures.\n\
         These are harness budgets, not permission to skip acceptance criteria or deterministic verification.",
        profile = decision.profile,
        score = decision.difficulty_score,
        tools = decision.max_tool_calls,
        retries = decision.max_retries,
        candidates = decision.candidate_count,
        verification = decision.verification_depth,
        approval = decision.human_approval,
        failures = decision.escalation_failure_threshold,
    )
}

pub fn apply_route_to_codex_command(
    decision: &RouteDecision,
    command: &mut std::process::Command,
    user_args: &[std::ffi::OsString],
) {
    if let Some(model) = decision.model.as_deref()
        && !user_overrides_model(user_args)
    {
        command.arg("-m").arg(model);
    }
    if !user_overrides_reasoning_effort(user_args) {
        command
            .arg("-c")
            .arg(format!("model_reasoning_effort={}", decision.reasoning_effort));
    }
    command
        .env("CODEXFLOW_ROUTE_PROFILE", &decision.profile)
        .env(
            "CODEXFLOW_DIFFICULTY_SCORE",
            decision.difficulty_score.to_string(),
        )
        .env(
            "CODEXFLOW_TOOL_BUDGET",
            decision.max_tool_calls.to_string(),
        )
        .env("CODEXFLOW_RETRY_BUDGET", decision.max_retries.to_string())
        .env(
            "CODEXFLOW_CANDIDATE_BUDGET",
            decision.candidate_count.to_string(),
        )
        .env("CODEXFLOW_VERIFICATION_DEPTH", &decision.verification_depth);
}

fn init(project_root: &Path, force: bool) -> Result<()> {
    let path = policy_path(project_root);
    if path.exists() && !force {
        bail!(
            "routing policy already exists at {}; use --force to replace it",
            path.display()
        );
    }
    fs::create_dir_all(path.parent().context("routing policy parent")?)?;
    fs::write(&path, serde_json::to_vec_pretty(&RoutingPolicy::default())?)
        .with_context(|| format!("write {}", path.display()))?;
    println!("{}", path.display());
    Ok(())
}

fn classify_task(task: &str) -> (u32, Vec<String>) {
    let lower = task.to_ascii_lowercase();
    let mut score = 2u32;
    let mut factors = Vec::new();

    if contains_any(
        &lower,
        &[
            "typo",
            "spelling",
            "comment",
            "readme",
            "docs only",
            "documentation only",
        ],
    ) {
        score = score.saturating_sub(2);
        factors.push("narrow documentation/edit task".to_string());
    }
    if contains_any(
        &lower,
        &["bug", "fix", "test", "implement", "feature", "cli", "api", "parser"],
    ) {
        score = score.saturating_add(1);
        factors.push("implementation or debugging work".to_string());
    }
    if contains_any(
        &lower,
        &[
            "architecture",
            "refactor",
            "concurrency",
            "async",
            "protocol",
            "serialization",
            "distributed",
            "performance",
            "bottleneck",
            "memory",
            "linker",
        ],
    ) {
        score = score.saturating_add(2);
        factors.push("cross-cutting or systems complexity".to_string());
    }
    if contains_any(
        &lower,
        &[
            "security",
            "authentication",
            "authorization",
            "credential",
            "secret",
            "payment",
            "production",
            "deploy",
            "migration",
            "database schema",
            "delete data",
            "irreversible",
            "kernel",
        ],
    ) {
        score = score.saturating_add(4);
        factors.push("high-impact or security-sensitive work".to_string());
    }
    if contains_any(
        &lower,
        &[
            "research",
            "compare",
            "multiple",
            "candidate",
            "uncertain",
            "ambiguous",
            "long-running",
        ],
    ) {
        score = score.saturating_add(1);
        factors.push("search or uncertainty requires additional inference".to_string());
    }
    if lower.split_whitespace().count() >= 80 {
        score = score.saturating_add(1);
        factors.push("large task specification".to_string());
    }
    if factors.is_empty() {
        factors.push("ordinary interactive task".to_string());
    }
    (score.min(12), factors)
}

fn profile_for_score(score: u32) -> &'static str {
    match score {
        0..=1 => PROFILE_FAST,
        2..=4 => PROFILE_BALANCED,
        5..=7 => PROFILE_DEEP,
        _ => PROFILE_CRITICAL,
    }
}

fn escalate_profile(profile: &str) -> &'static str {
    match profile {
        PROFILE_FAST => PROFILE_BALANCED,
        PROFILE_BALANCED => PROFILE_DEEP,
        PROFILE_DEEP | PROFILE_CRITICAL => PROFILE_CRITICAL,
        _ => PROFILE_BALANCED,
    }
}

fn policy_profile<'a>(policy: &'a RoutingPolicy, name: &str) -> &'a RouteProfile {
    match name {
        PROFILE_FAST => &policy.fast,
        PROFILE_BALANCED => &policy.balanced,
        PROFILE_DEEP => &policy.deep,
        PROFILE_CRITICAL => &policy.critical,
        _ => &policy.balanced,
    }
}

fn validate_policy(policy: &RoutingPolicy) -> Result<()> {
    if policy.schema != ROUTING_SCHEMA {
        bail!("unsupported routing schema {}", policy.schema);
    }
    for (name, profile) in [
        (PROFILE_FAST, &policy.fast),
        (PROFILE_BALANCED, &policy.balanced),
        (PROFILE_DEEP, &policy.deep),
        (PROFILE_CRITICAL, &policy.critical),
    ] {
        validate_profile(profile).with_context(|| format!("invalid {name} routing profile"))?;
    }
    Ok(())
}

fn validate_profile(profile: &RouteProfile) -> Result<()> {
    if ![
        "none",
        "minimal",
        "low",
        "medium",
        "high",
        "xhigh",
        "max",
        "ultra",
        "persistent",
    ]
    .contains(&profile.reasoning_effort.as_str())
    {
        bail!("unsupported reasoning effort {}", profile.reasoning_effort);
    }
    if !(2_000..=64_000).contains(&profile.max_context_chars) {
        bail!("max_context_chars must be between 2000 and 64000");
    }
    if profile.max_tool_calls == 0 || profile.max_tool_calls > 10_000 {
        bail!("max_tool_calls must be between 1 and 10000");
    }
    if profile.max_retries > 100 {
        bail!("max_retries must be at most 100");
    }
    if profile.candidate_count == 0 || profile.candidate_count > 32 {
        bail!("candidate_count must be between 1 and 32");
    }
    if !["focused", "standard", "deep", "exhaustive"]
        .contains(&profile.verification_depth.as_str())
    {
        bail!("verification_depth must be focused, standard, deep, or exhaustive");
    }
    Ok(())
}

fn validate_profile_name(profile: &str) -> Result<()> {
    if ![
        PROFILE_FAST,
        PROFILE_BALANCED,
        PROFILE_DEEP,
        PROFILE_CRITICAL,
    ]
    .contains(&profile)
    {
        bail!("route profile must be fast, balanced, deep, or critical");
    }
    Ok(())
}

fn validate_failure_class(failure: &str) -> Result<()> {
    if ![
        FAILURE_RETRIEVAL,
        FAILURE_TOOL_SELECTION,
        FAILURE_INVALID_ARGUMENTS,
        FAILURE_MISSING_DEPENDENCY,
        FAILURE_CONTEXT_INSUFFICIENCY,
        FAILURE_REASONING,
        FAILURE_TEST,
        FAILURE_PERMISSION,
        FAILURE_TIMEOUT,
        FAILURE_AMBIGUOUS_REQUIREMENT,
    ]
    .contains(&failure)
    {
        bail!(
            "failure must be retrieval, tool_selection, invalid_arguments, missing_dependency, context_insufficiency, reasoning, test, permission, timeout, or ambiguous_requirement"
        );
    }
    Ok(())
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn policy_path(project_root: &Path) -> PathBuf {
    project_root.join(".codexflow").join("routing.json")
}

fn load_policy(project_root: &Path) -> Result<RoutingPolicy> {
    let path = policy_path(project_root);
    if !path.exists() {
        return Ok(RoutingPolicy::default());
    }
    let data = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&data).with_context(|| format!("parse {}", path.display()))
}

fn user_overrides_model(args: &[std::ffi::OsString]) -> bool {
    args.iter().enumerate().any(|(index, value)| {
        let value = value.to_string_lossy();
        value == "-m"
            || value == "--model"
            || value.starts_with("--model=")
            || value.starts_with("model=")
            || (value == "-c"
                && args
                    .get(index + 1)
                    .is_some_and(|next| next.to_string_lossy().starts_with("model=")))
    })
}

fn user_overrides_reasoning_effort(args: &[std::ffi::OsString]) -> bool {
    args.iter().enumerate().any(|(index, value)| {
        let value = value.to_string_lossy();
        value.starts_with("model_reasoning_effort=")
            || (value == "-c"
                && args.get(index + 1).is_some_and(|next| {
                    next.to_string_lossy()
                        .starts_with("model_reasoning_effort=")
                }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn trivial_docs_task_routes_fast() {
        let (score, _) = classify_task("fix a typo in README documentation only");
        assert_eq!(profile_for_score(score), PROFILE_FAST);
    }

    #[test]
    fn concurrency_security_task_routes_critical() {
        let (score, factors) = classify_task(
            "fix production authentication concurrency and credential handling",
        );
        assert_eq!(profile_for_score(score), PROFILE_CRITICAL);
        assert!(
            factors
                .iter()
                .any(|factor| factor.contains("security-sensitive"))
        );
    }

    #[test]
    fn explicit_profile_overrides_classifier() {
        let temp = tempfile::tempdir().expect("tempdir");
        let decision = resolve_route(
            temp.path(),
            Some("production migration"),
            Some(PROFILE_FAST),
        )
        .expect("route");
        assert_eq!(decision.profile, PROFILE_FAST);
        assert_eq!(decision.reasoning_effort, "low");
    }

    #[test]
    fn repeated_reasoning_failure_escalates_and_changes_strategy() {
        let temp = tempfile::tempdir().expect("tempdir");
        let decision = resolve_recovery(
            temp.path(),
            FAILURE_REASONING,
            2,
            Some(PROFILE_BALANCED),
            Some("same hypothesis failed twice".to_string()),
        )
        .expect("recovery");
        assert_eq!(decision.next_profile, PROFILE_DEEP);
        assert!(decision.retry_allowed);
        assert!(decision.strategy_change);
        assert!(decision.preserve_failure_evidence);
    }

    #[test]
    fn permission_and_ambiguity_never_blindly_retry() {
        let temp = tempfile::tempdir().expect("tempdir");
        for failure in [FAILURE_PERMISSION, FAILURE_AMBIGUOUS_REQUIREMENT] {
            let decision = resolve_recovery(
                temp.path(),
                failure,
                1,
                Some(PROFILE_BALANCED),
                None,
            )
            .expect("recovery");
            assert!(!decision.retry_allowed);
            assert!(decision.human_approval);
            assert!(!decision.strategy_change);
        }
    }

    #[test]
    fn repeated_test_failure_recommends_rollback_and_deeper_verification() {
        let temp = tempfile::tempdir().expect("tempdir");
        let decision = resolve_recovery(
            temp.path(),
            FAILURE_TEST,
            2,
            Some(PROFILE_DEEP),
            None,
        )
        .expect("recovery");
        assert_eq!(decision.next_profile, PROFILE_CRITICAL);
        assert!(decision.rollback_recommended);
        assert_eq!(decision.verification_depth, "exhaustive");
    }

    #[test]
    fn invalid_arguments_repairs_schema_without_escalating_strategy() {
        let temp = tempfile::tempdir().expect("tempdir");
        let decision = resolve_recovery(
            temp.path(),
            FAILURE_INVALID_ARGUMENTS,
            3,
            Some(PROFILE_FAST),
            None,
        )
        .expect("recovery");
        assert_eq!(decision.next_profile, PROFILE_BALANCED);
        assert!(decision.retry_allowed);
        assert!(!decision.strategy_change);
    }

    #[test]
    fn explicit_user_model_and_effort_are_not_overridden() {
        let args = vec![
            OsString::from("-m"),
            OsString::from("local-model"),
            OsString::from("-c"),
            OsString::from("model_reasoning_effort=max"),
        ];
        assert!(user_overrides_model(&args));
        assert!(user_overrides_reasoning_effort(&args));
    }
}
