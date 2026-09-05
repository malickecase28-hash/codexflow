# CodexFlow Master 60 Capability Map

This file converts the 60-category research checklist into a concrete CodexFlow implementation map. Each category records what we are choosing/taking and why. Mapping does not mean blindly vendoring an upstream project: every dependency or borrowed implementation must pass license, security, maintenance, benchmark, and integration review. The preferred pattern is to keep CodexFlow's differentiating policy/orchestration logic native while reusing solved infrastructure through narrow adapters.

Status vocabulary:

- **native-now** — already materially present in current CodexFlow/Codex base.
- **extend** — existing foundation exists but needs stronger native behavior.
- **adopt-adapter** — integrate an external component behind a CodexFlow interface.
- **research-gated** — valuable but only after benchmarks justify its cost/complexity.

## 1. Context engineering — ContextBus

**Choice: extend Headroom + Codex context hygiene + DeepSeek-style event projection, with LLMLingua only as an experiment for natural-language evidence.** CodexFlow already has the right foundation: adaptive skill projection, stale tool-result pruning and Headroom compression. We will make the durable event/state store canonical and derive model-visible context from it, so compaction never becomes the source of truth. Headroom remains the general reversible WARM-context layer; LLMLingua is research-gated for verbose prose/retrieved documents rather than code because adding another model call to ordinary context management would violate the latency/token objective.

## 2. Map, not the entire world — ProjectMap

**Choice: combine Aider-style ranked repo maps, Repomix structural maps, Tree-sitter syntax indexes and Serena semantic lookup.** No single representation should dominate: Tree-sitter provides cheap syntax structure, Repomix-style summaries provide cold-start shape, Aider-style graph ranking chooses globally important symbols, and Serena/LSP answers precise semantic questions on demand. The model receives a map and targeted regions instead of repository dumps, while non-code projects can implement equivalent source/entity maps through the same interface.

## 3. Excellent tool design — Agent Computer Interface contract

**Choice: mine Oh My Pi and SWE-agent for empirically model-friendly tool interfaces, then enforce an internal ACI lint/metadata contract.** CodexFlow should expose a small number of semantically distinct operations with bounded output, useful defaults, clear errors, pagination and cost/permission metadata. We should not copy dozens of project-specific tools; we take the design lessons and benchmark them per model because weak-model performance is unusually sensitive to naming, edit format and response shape.

## 4. Dynamic tool loading — ToolRouter

**Choice: extend our existing tool pruning with PydanticAI-style deferred toolsets and ToolHive-style discovery/governance.** The hot surface should stay close to search/read/edit/run/test/discover, while long-tail connectors and MCP tools remain cold until deterministic project/task evidence or explicit discovery activates them. This directly reduces context and branching factor, and it aligns with the skill migration work where capabilities become runtime bundles rather than markdown automatically exposed to every turn.

## 5. Programmatic tool orchestration — Deterministic Execution Kernel

**Choice: take the Oh My Pi persistent-kernel principle, but implement a sandboxed CodexFlow execution kernel rather than forcing the model between every mechanical operation.** Filtering, joining, sorting, parsing, arithmetic, retries and API chaining should execute as ordinary code once the model has supplied a bounded program/plan. This preserves tokens and prevents giant intermediate results from re-entering model context; the kernel gets strict time/resource/tool capability boundaries and is never equivalent to unrestricted host execution.

## 6. Constrain the model action space — StructuredAction layer

**Choice: use JSON Schema everywhere possible and add XGrammar/provider-native constrained decoding for compatible local inference backends.** Prompting a weak model to emit valid structured actions is inferior to making invalid actions unrepresentable. CodexFlow will keep normal typed tool schemas for all providers, and local SGLang/vLLM adapters can optionally add XGrammar-level sampler constraints. This is an adapter capability, not a hard dependency for frontier APIs that already provide structured tool calling.

## 7. Bounded workflows — WorkflowEngine

**Choice: native deterministic FSM/event workflow inspired by DeepSeek Harness and LangGraph, without making Python LangGraph the core runtime.** CodexFlow already has task/gate concepts and should own inspect/plan/retrieve/implement/verify/review/deliver transitions in Rust so common workflows remain fast and portable. LangGraph remains useful research/reference material and possibly an external workflow adapter, but the harness's fundamental completion and authority state should not depend on a second language/runtime.

## 8. Explicit decomposition — TopologyPlanner

**Choice: extend GOD's task classification with Qwen Code/Gemini/Deep Agents-style bounded delegation.** The planner decides task graph, dependencies, critical path and ownership before spawning the smallest useful team, then workers receive disjoint scopes and compact contracts. This is especially important for weaker models because it moves executive decomposition out of their individual contexts while still allowing the model to propose subtasks when the task is genuinely ambiguous.

## 9. Explicit acceptance criteria — GoalLedger

**Choice: make acceptance criteria durable runtime state rather than conversational prose.** The existing goal/completion ideas become a GoalLedger containing required evidence, tests, artifacts, blockers and unresolved criteria, with status independent from what a model claims. A task cannot transition to DONE until the ledger passes, while BLOCKED_WAITING permits event-driven sleep without falsely completing the goal.

## 10. Independent verification — VerificationEngine

**Choice: native verifier router over compilers, type checkers, linters, tests, Playwright, schema validators, security scanners and domain-specific evidence tools.** We do not need an LLM to decide whether syntax compiles or a deterministic check passed. The model can select or interpret evidence when needed, but verification results remain authoritative machine state; subjective/artifact review uses a fresh reviewer only when deterministic verification cannot fully judge quality.

## 11. Automatic repair loops — RepairEngine

**Choice: connect verification failures to bounded failure-specific repair transitions, reusing the Phase 3B self-heal/service-agent model.** A failed compile, test, parser or permission check must not become a generic 'try again'; the failure classifier chooses a recovery strategy, passes exact evidence to a fresh/bounded repair turn and revalidates. Attempt fingerprints, cooldowns and budgets stop loops, and autonomous repository repair defaults to a draft PR plus human merge review.

## 12. Backtracking — CheckpointManager

**Choice: combine git worktrees/commits with workflow checkpoints and transactional tool operations.** Cline/Claude/LangGraph are useful references, but CodexFlow can exploit Git as the authoritative code checkpoint while persisting non-code task state separately. Before risky transitions the runtime records a last-known-good reference; failed trajectories can fork or roll back without asking the model to reconstruct prior state from chat.

## 13. Last-known-good state — BaselineState

**Choice: make baseline capture an automatic project workflow step for non-trivial changes.** CodexFlow records starting commit, dirty-state ownership, relevant passing/failing checks, toolchain/config identity and known pre-existing failures. Post-change verification compares against that baseline so agents do not blame old failures on their patch or silently regress previously green behavior.

## 14. Persistent task state — ProjectStateStore

**Choice: extend the existing Codex SQLite/project and CodexFlow task ledgers into event-sourced durable task state.** Objective, current subtask, next action, blockers, changed files, decisions, tests and evidence should survive thread compaction/restart without becoming prompt history. The model sees only a projection required for the current turn; complete state remains inspectable and replayable outside context.

## 15. Long-running handoff — HandoffEnvelope

**Choice: retain our compact handoff rule and formalize it as a typed artifact.** A handoff records accomplished work, remaining criteria, failures, relevant refs, decisions, restart commands and next action; it never contains a full transcript unless explicitly requested. Fresh contexts and service agents can resume from this object plus durable state, which reduces stale-history costs and allows model replacement mid-task.

## 16. Specialist skills — CapabilityRuntime

**Choice: continue the Phase 3 capability migration instead of eliminating skills.** Rare deep knowledge remains lazy specialist modules with tiny routing cards; repetitive quality/safety behavior moves into kernel policy, flows, profiles or tools. This preserves expertise without exposing 100+ skill descriptions to every model, and every merged profile must satisfy the rule-level coverage contract before original skill exposure is reduced.

## 17. Few-shot procedural examples — ProcedureCards

**Choice: attach examples to tools/capabilities only when a model profile demonstrates that schemas alone are insufficient.** Correct calls, incorrect calls, boundary cases and recovery examples become small lazy ProcedureCards selected by model/tool telemetry, not permanent prompt text. This keeps strong models fast while giving weaker/local models concrete demonstrations where benchmark evidence shows they improve tool success.

## 18. Specialized agents — AgentRoleRegistry

**Choice: expand the existing flow roles into configurable functional roles plus lifecycle class.** Planner, researcher, implementer, debugger, verifier, reviewer, security reviewer, summarizer and service workers can use the same or different models, but each gets narrow instructions, tools and permissions. Functional role is separate from ownership so protected service agents remain user/supervisor-managed even if their functional role is `flow_worker`.

## 19. Supervisor-worker architecture — GOD + Supervisor planes

**Choice: preserve GOD for interactive orchestration and add the Phase 3B non-LLM Service Supervisor for background work.** GOD owns user-task decomposition and synthesis; the supervisor owns triggers, schedules, wakeups, budgets and protected service lifecycle. This prevents the root model from babysitting jobs and keeps background automation alive without requiring an interactive terminal or resident model context.

## 20. Parallelism — Native Codex multi-agent

**Choice: use Codex's native multi-agent runtime rather than an external PTY farm, with topology limits enforced by CodexFlow.** Parallelism is enabled only for independent work or genuinely useful candidate diversity; tiny tasks stay single-agent. We take Qwen/Gemini/Cline ideas for isolated scopes and tool restriction while retaining Codex's native thread transport, because duplicating process/auth/session infrastructure would add latency and failure modes.

## 21. Self-consistency and ensembles — EnsembleMode

**Choice: research-gated optional ensemble execution for cheap/local models, not a default behavior.** Multiple independent candidates and consensus can improve some tasks, but unconditional sampling multiplies tokens and latency. CodexFlow will expose ensemble count/diversity/judge as an adaptive inference option activated by benchmark-proven task classes or explicit user policy, with deterministic validators preferred over model voting whenever possible.

## 22. Search over reasoning trajectories — SearchMode

**Choice: keep branching/beam/tree-style trajectory exploration as a high-cost specialist runtime.** The capability can be useful when weak models can recognize a good direction better than generate it immediately, but it is inappropriate for ordinary coding. We will implement the generic checkpoint/fork/scoring seams first and benchmark simple candidate search before considering Monte Carlo/tree strategies; no research algorithm becomes baseline scaffolding without measured value.

## 23. Generator and verifier separation — IndependentReviewFlow

**Choice: formalize the already-planned fresh-context reviewer/verifier as a first-class pattern inspired by Aider Architect and critic/repair systems.** Generation, deterministic validation and independent review receive separate contexts so a model cannot simply rationalize its own output. The same weights may serve multiple roles, but the runtime preserves role/context independence and only sends the minimum artifact/evidence needed for each pass.

## 24. Confidence and escalation — EscalationRouter

**Choice: validator/failure-driven confidence first, RouteLLM-style model routing second, self-reported model confidence last.** Failed verification, repeated uncertainty, ambiguous requirements and risk class are stronger escalation signals than asking a weak model how confident it feels. The router can move from cheap/local to stronger models or human review according to project policy, with every escalation measurable in HarnessBench.

## 25. Model routing — ModelRoleRouter

**Choice: combine OMP-style named roles with RouteLLM concepts and a provider abstraction adapter such as LiteLLM only where useful.** CodexFlow owns the intelligence policy—tiny/default/planner/reviewer/vision/slow/service—while provider plumbing remains separable. Users can bind any available model/provider/reasoning level to roles, which is central to benchmarking whether a cheaper model in CodexFlow can match a stronger model in vanilla Codex.

## 26. Non-LLM components — DeterministicServices

**Choice: aggressively prefer parsers, ASTs, compilers, databases, search, calculators, graph algorithms and validators over neural reasoning.** This is not one dependency; it is a kernel design rule enforced during capability review. A capability proposal must justify any model call that could be replaced with reliable software, and the benchmark trace records model calls avoided by deterministic execution.

## 27. Good editing primitives — EditEngine

**Choice: keep `apply_patch`, add OMP-inspired hash/content-anchored edits and ast-grep structural edits behind one transactional EditEngine.** Whole-file regeneration is expensive and dangerous for weaker models. Hashline-like anchors help stale-position recovery, AST rewrites handle repetitive structural changes, and all-or-nothing multi-file application plus diff preview/validation prevents partial corruption; exact upstream code reuse is license/security-reviewed before adoption.

## 28. Search before read — ReadPolicy

**Choice: make search/structural inspection a deterministic precondition for large reads rather than a prompt suggestion.** Small targeted reads remain direct; large/unknown files require a relevant range, symbol, search hit or structural summary first unless the task explicitly needs the full artifact. Serena, Tree-sitter and text search feed the policy, reducing context cost without making the model reason about token conservation.

## 29. Read before edit — EditPolicy

**Choice: enforce target inspection, dependency/interface inspection and post-edit diff review at the tool boundary according to risk.** A model may not modify an unseen source region merely because it guessed the contents, and public API changes can trigger dependent/reference/test inspection automatically. Trivial/generated operations can use specialized safe tools, so the policy remains adaptive rather than adding ritual to every edit.

## 30. Plan before expensive action — AdaptivePlanGate

**Choice: require a compact structured plan only when the action is expensive, broad, irreversible or high-risk.** The plan contains goal, intended scope and validation, which allows deterministic policy checks before expensive work starts. Tiny edits bypass the gate; this keeps Codex CLI speed while giving weaker models an explicit intermediate representation when impulsive action would be costly.

## 31. Tool-result transformation — ResultProjector

**Choice: combine Headroom with tool-specific semantic projectors.** Raw API/tool output is stored durably, while the model receives normalized names, dates/units, relevance ordering, grouped abnormalities and bounded surrounding context. Tool-side filtering/aggregation should happen before projection, and Headroom compresses remaining verbose payloads reversibly; this avoids asking a model to interpret infrastructure noise token by token.

## 32. Environment introspection — EnvironmentSnapshot

**Choice: extend Phase 2B project management into a cheap cached environment snapshot.** Project/root, branch, dirty state, runtimes, dependencies, available tools, services, test/build entrypoints and permission profile become machine-readable state available before the first model request. This replaces repeated `pwd`, directory listings and environment guessing with one deterministic orientation object.

## 33. Deterministic bootstrapping — BootstrapPipeline

**Choice: create a fixed startup pipeline that resolves project, validates runtime bundle, loads project instructions/state, refreshes maps, resumes waits/goals and calculates context/tool budgets before invoking the model.** Expensive checks are cached and invalidated by relevant fingerprints; baseline tests only run when policy requires them. This prevents every new context from spending tokens rediscovering how the repository and harness work.

## 34. Environment standardization — ProjectContract

**Choice: define a project-management contract for standard build/test/lint commands, directories, logging, reproducibility and dependency locks without forcing every repository into one framework.** CodexFlow project metadata records the project's native conventions, and adapters translate them into consistent agent-facing operations. Containers or dev environments remain optional project capabilities, not mandatory overhead for a local CLI.

## 35. Mechanically enforce invariants — Policy + CI spine

**Choice: extend existing architecture/dependency/gate checks into machine-enforced project policies, using OPA concepts where an external policy engine adds value.** Rules such as import boundaries, generated-file protection, required compatibility tests and merge gates should fail in tools/CI rather than live only in prompts. CodexFlow may implement common fast policies natively and optionally support OPA/Rego for complex organization-level rules.

## 36. Guardrails — AuthorityEngine

**Choice: combine Codex sandbox/approval separation with CodexFlow lifecycle/action policy and ToolHive-style MCP permissions.** Filesystem, network, shell, secrets and irreversible actions receive mechanical capability boundaries first and human/model reviewer approval second. Guardrails are also intelligence scaffolding: reducing dangerous/irrelevant actions shrinks the weak model's search space.

## 37. Sandboxing — SandboxAdapter

**Choice: retain Codex's native sandbox for normal workstation tasks and evaluate Microsandbox as the first stronger local isolation backend, with gVisor/Firecracker reserved for server/multi-user deployment.** We should not build a microVM system ourselves. E2B can be an optional managed abstraction, but local CodexFlow should prioritize a portable low-friction sandbox and benchmark startup/I/O overhead before making it default.

## 38. State persistence and durable execution — EventStore + Supervisor

**Choice: event-source CodexFlow workflow state and let the cross-platform supervisor own timers, external jobs and recovery; evaluate Temporal only for truly long external/enterprise workflows.** Interactive coding should not require a heavyweight workflow service. Durable AwaitSpecs, idempotent events, checkpoints and resume tokens cover local work, while an adapter to Temporal remains plausible for jobs that must survive machine/process failure across hours or days.

## 39. Human-in-the-loop at correct places — DecisionGate UI

**Choice: reuse Codex's native selectable TUI overlays for ambiguity, high-impact approvals, security installs, lifecycle ownership and repeated-failure escalation.** Humans should not approve routine safe operations, and agents should not silently authorize irreversible ones. Every gate resumes the suspended workflow after a structured decision instead of turning approval into an unstructured chat exchange.

## 40. Observation and tracing — OpenTelemetry/Phoenix adapter

**Choice: make a local event/trace schema authoritative and export it through OpenTelemetry, with Phoenix preferred initially for experimental analysis and Langfuse optional.** We need prompts/context composition, tool calls, model selection, state transitions, tokens, latency, validation and outcome to diagnose whether a failure belongs to the model or harness. External telemetry systems are sinks, not the canonical runtime state, so CodexFlow remains usable offline.

## 41. Transcript replay — ReplayEngine

**Choice: build replay from the same append-only event store plus Codex rollout traces.** Full trajectories, projections, tool results and checkpoints allow reproducing failures, comparing models and replaying from a known state without corrupting original history. Replay is also the foundation for benchmark ablations because a harness change can be tested against equivalent captured conditions.

## 42. Harness evaluations — HarnessBench

**Choice: implement our native evidence schema and runner, integrate Inspect AI for deep agent evaluations and Promptfoo for fast regression/red-team matrices.** CodexFlow must evaluate model+harness+tools+context+workflow, not only final text. We already designed paired vanilla/CodexFlow and stronger/weaker-model comparisons; this category turns those schemas into executable suites and held-out fixtures.

## 43. Optimize per model — ModelProfileLab

**Choice: make model-adaptive harness profiles a benchmark product rather than hard-coded folklore.** Tool count/naming, prompt/profile length, map size, chunking, planning depth, retries, edit format and compaction thresholds become tunable parameters associated with a model family/version. HarnessBench ablations promote only configurations that improve correctness first and efficiency second, preventing 'more scaffolding' from becoming an unquestioned default.

## 44. Adaptive inference budgets — BudgetController

**Choice: deterministic difficulty/risk signals allocate model, token, tool, retry, candidate and verification budgets.** Easy tasks should complete with one cheap trajectory; hard/high-risk tasks may receive planning, candidates, fresh review and stronger models. Budget expansion is event/failure driven and bounded, which lets local models receive extra compute where it buys capability without imposing that cost on every request.

## 45. Failure-aware routing — FailureClassifier

**Choice: classify retrieval, tool-selection, argument, dependency, context, reasoning, test, permission, timeout and requirement failures into distinct recovery transitions.** This extends Phase 2C circuit breakers and prevents blind retries. Classification should be deterministic from structured errors where possible; a small model classifier is used only for ambiguous cases, and repeated identical fingerprints escalate rather than retry forever.

## 46. Anti-loop mechanisms — LoopBreaker + event waits

**Choice: keep Phase 2C repeated-action/error/no-progress detection and add event-driven WAITING so legitimate long operations do not themselves create polling loops.** Repeated normalized tool calls, identical failures and no-progress turns trigger strategy change, backtracking or escalation. Waiting on children/builds/CI becomes a suspended runtime state, so the best anti-loop action is often not another model turn at all.

## 47. Anti-premature termination — CompletionGate

**Choice: GoalLedger plus verification/review gates owns the DONE transition.** Acceptance criteria, required artifacts, test state, blockers, final diff and outstanding tasks are machine state independent of the assistant's confidence. This preserves the useful 'goal' pressure you value while allowing `BLOCKED_WAITING` when progress genuinely depends on an external event.

## 48. Grounding and retrieval — RetrievalRouter

**Choice: route web/docs/code/database/graph/vector/keyword retrieval through typed adapters with provenance, not one giant RAG subsystem.** Retrieval source depends on project/task and tool availability; deterministic search and primary documentation are preferred before embeddings. The router returns bounded evidence objects so smaller models can reason over the relevant facts rather than depending on memorized weights.

## 49. Source quality control — EvidenceRanker

**Choice: make provenance, date, authority, duplication and contradiction checks part of retrieval projection.** Primary/original sources rank above derivative copies, source timestamps are preserved, contradictions remain explicit rather than silently merged, and final claims can retain evidence refs. This is especially valuable for research/service agents where a weak model should not be expected to infer source trustworthiness from prose alone.

## 50. Prompt hierarchy and modular instructions — PromptAssembler

**Choice: continue adaptive context hygiene and formalize instruction layers: kernel, tool policy, project, task, selected profiles, specialist modules, evidence and output contract.** Only applicable layers enter a turn and each carries provenance/priority. This is the instruction-space equivalent of dynamic tools and is the mechanism that lets 100+ capabilities exist without becoming a 100-skill system prompt.

## 51. Explicit priorities — PriorityKernel

**Choice: encode a small stable priority order in the harness rather than repeating it in every skill.** Safety/authority and correctness dominate acceptance criteria, regression avoidance, minimal change and elegance in that order unless a higher-authority project policy says otherwise. Conflicting instructions can therefore be resolved mechanically before the model receives them, reducing ambiguity for weaker models.

## 52. Strong defaults — DefaultResolver

**Choice: project/model/task profiles provide defaults for working directory, search depth, pagination, test/build command, patch style, timeout, retries and delivery behavior.** Defaults are visible and overridable but normally remove low-value choices from the model. The build-cost manager is an early example: `cargo check`/focused tests are default while expensive release linking requires a real reason.

## 53. Semantic affordances — ToolSemantics

**Choice: enforce semantically meaningful tool/action names, parameters, errors and result objects as part of ACI review.** Weak models benefit when the interface itself carries domain meaning; opaque IDs are resolved where possible and machine IDs remain secondary references. HarnessBench should measure tool-selection/argument success by model, so interface wording becomes an empirical engineering variable rather than aesthetics.

## 54. Reduce choice where choice has no value — ActionMask

**Choice: combine tool pruning, workflow-state allowed actions and role permissions into a dynamic action mask.** A debugger should not see deployment tools, a reviewer should not see normal edit authority, and a WAITING thread should not see arbitrary next actions. This shrinks branching factor at zero model cost and is one of the strongest ways to make lower-tier models behave more reliably.

## 55. Increase choice where exploration has value — CandidateExplorer

**Choice: separate action-space restriction from solution-space exploration.** When benchmarks show benefit, CodexFlow may ask multiple isolated planners/implementers for alternatives while keeping each agent's tools and permissions narrow, then use deterministic tests plus an independent judge/reviewer to select. This is adaptive/high-cost behavior, never a justification for exposing more irrelevant tools to every model.

## 56. Workspace as external cognition — WorkspaceMemory

**Choice: standardize durable plans, progress, notes, research, hypotheses, evidence and handoffs as typed artifacts outside model context.** Git-backed/human-readable storage is preferred initially, borrowing the useful Letta MemFS idea without importing a large memory platform by default. Context projections retrieve only the artifacts needed for the current task while the workspace remains inspectable by users and fresh agents.

## 57. Make environment legible — SemanticProjection

**Choice: every subsystem must expose concise agent-facing state rather than raw internal protocol blobs.** Build/test status, deployment state, project identity, permission state and service results become named structured summaries with links to raw evidence. This overlaps ResultProjector and ToolSemantics deliberately: legibility is an end-to-end interface requirement that should be benchmarked for weak-model tool accuracy.

## 58. Externalize metacognition — InquiryState

**Choice: represent known facts, unknowns, hypothesis, required evidence, next experiment, result and completion test as optional structured task fields activated for debugging/research complexity.** Strong models may fill them cheaply; weaker models gain explicit scaffolding. They are not mandatory prose sections in every turn—the WorkflowEngine persists them and injects only unresolved/current fields.

## 59. Externalize executive function — ExecutiveKernel

**Choice: treat memory, attention, planning, inhibition, calculation, verification, task switching and uncertainty management as composable harness services.** This is the architectural synthesis of ContextBus, WorkflowEngine, AuthorityEngine, DeterministicServices, VerificationEngine, ModelRouter and Supervisor. The model supplies judgment where software cannot; the kernel supplies the executive functions we can guarantee mechanically.

## 60. Turn intelligence problems into engineering problems — CodexFlow governing principle

**Choice: make this the acceptance rule for every future capability.** Before adding prompt text or another model call, ask whether the failure can instead be prevented with state, a parser, search, tool design, policy, event, test, checkpoint, deterministic computation or better environment representation. Every promoted feature must show benchmark evidence that it improves safety/correctness or produces worthwhile time/token/repository efficiency; otherwise it remains experimental or is rejected.

---

# Cross-cutting implementation order

The 60 categories collapse into a smaller set of actual runtime systems. We should implement them in dependency order rather than sixty separate mini-frameworks:

1. **EventStore + EventBus + Await/Wake runtime** — canonical events, sleeping workflows, no model polling.
2. **Prompt/Context/Project maps** — event projection, Headroom tiers, repo/symbol maps, adaptive instructions.
3. **Tool/ACI layer** — dynamic discovery, semantic tools/results, search/read/edit policy, deterministic kernels.
4. **Workflow/Goal/Checkpoint layer** — bounded FSM, decomposition, acceptance criteria, rollback and handoff.
5. **Verification/Repair/Review layer** — exact validators first, bounded repair and fresh review.
6. **Authority/Sandbox/Policy layer** — lifecycle ownership, permissions, OPA-like invariants, stronger sandbox adapters.
7. **Model/Inference layer** — roles, escalation, model profiles, adaptive budgets and optional constrained decoding.
8. **Service-agent plane** — protected background runs, triggers, inbox and self-heal.
9. **Observability/Replay/Benchmark layer** — traces, ablations, model/harness comparisons and performance budget.
10. **Advanced exploration/memory adapters** — ensembles, trajectory search, temporal/vector memory only where benchmarks justify them.

# Progress rule

A category is not considered covered merely because this file names an upstream. `covered` requires a CodexFlow primitive or adapter, tests/evidence appropriate to the category, security/license review for imported code, integration with telemetry, and at least one benchmark or deterministic acceptance test showing the intended behavior. The master map should eventually become machine-readable so Phase 3+ work can report `mapped / implementing / validated / benchmarked / rejected` per category.
