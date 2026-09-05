# CodexFlow Capability Runtime

CodexFlow should not become a larger prompt full of skills. It should become a harness that makes good behavior the default and loads rare knowledge only when needed.

The migration map is `skill-migration-map.json`.

## Core rule

A current skill belongs in one of seven places:

1. **Kernel** — always-on state/policy implemented below the model.
2. **Event flow** — deterministic trigger starts a workflow, interceptor, or blocking gate.
3. **Automatic profile** — a small task/domain policy is injected only when the task matches.
4. **Tool adapter** — direct tool/connector capability; no skill prompt.
5. **Specialist module** — deep or stylistic knowledge remains lazy.
6. **Project playbook** — repository-specific knowledge remains local to that project.
7. **Command/view** — a CLI/TUI report instead of model instructions.

This makes the model-visible skill catalog a last resort rather than the primary control plane.

## Prompt assembly target

```text
User turn
   |
   v
Intent + project fingerprint              deterministic, cached
   |
   +--> kernel policies                   zero/near-zero prompt cost
   |
   +--> event interceptors                only if an event matches
   |
   +--> automatic profiles                maximum small bounded set
   |
   +--> lazy specialist module            only when needed
   v
Prompt assembly
   |
   v
Model
```

The harness should never attach all design, Rust, networking, SRE, security, research, and process guidance to every request.

## Speed budget

The capability runtime is built around a latency budget.

### Zero-model routing first

Common triggers should use deterministic evidence:

- active project id and project manifest
- file extensions and touched paths
- dependency manifests
- symbols/framework markers
- explicit tool/action intent
- current workflow state
- change class and risk class

An LLM classifier is a fallback only when deterministic routing is materially ambiguous.

### Context rules

- Kernel rule: no full skill text.
- Tool adapter: no skill text.
- Automatic profile: distilled compact policy, normally one to a few hundred tokens.
- At most a small bounded number of automatic profiles per turn.
- Specialist module: full content loaded only into the agent/subagent that needs it.
- Project playbook: only visible in the matching repository/task.
- Old profile fragments are removed from later turns by context hygiene.

### No extra critic call by default

Frontend and writing quality should primarily improve the first generation through compact task-specific policy. A second model review is reserved for non-trivial or explicitly reviewed artifacts, not every paragraph or CSS edit.

## Native install transaction

Skill installation must not depend on the model remembering to invoke a security skill.

There are two hooks:

1. **Intent hook** — detects an install/add/enable request early and starts the install UX.
2. **Authoritative tool boundary** — any actual skill/plugin/MCP activation operation is intercepted even if intent routing missed it.

The tool boundary is the security authority.

```text
User: install <source>
        |
        v
ExtensionInstallInterceptor
        |
        v
Acquire into quarantine
(no activation, no scripts)
        |
        +--> provenance/source metadata
        +--> manifest + permission extraction
        +--> content hash
        |
        v
SkillSpector static scan
        |
        +--> semantic scan only when policy/risk requires it
        |
        v
Structured InstallReport
        |
        v
TUI decision overlay
        |
        +--> install as lazy skill
        +--> propose harness integration
        +--> inspect findings
        +--> cancel/quarantine
```

NVIDIA SkillSpector is a scanner implementation, not a model-visible skill. CodexFlow consumes its structured result as one input to the native gate.

### Why static-first

Scanning is an infrequent install-time event, but it still should not waste time. Static scanning is mandatory and deterministic. Semantic/LLM scanning is activated by policy when, for example:

- static risk is non-trivial or uncertain
- a skill requests shell/network/filesystem/MCP authority
- prompt-injection or encoded-content signals exist
- the source is untrusted/new
- the user chooses deep inspection

Projects or users can configure semantic scan to always-on if desired.

### Fail behavior

The install transaction fails closed if the scanner cannot produce the minimum configured report.

An explicit elevated override can exist, but it must be a user action in the TUI and must be written to the extension audit ledger. It is never inferred from model text.

## TUI interaction

Do not ask the user to type a sentence such as `1` or `2` into chat.

Codex already has a native request-user-input overlay with option rows and selection state. CodexFlow should reuse that UI plumbing for extension decisions.

Example safe report:

```text
Install candidate: example-skill
Risk: LOW  8/100
Source: github.com/example/skill @ <commit>
Findings: 2 informational

› 1. Install as on-demand skill
  2. Integrate into CodexFlow harness
  3. View security findings
  4. Cancel
```

The existing overlay can support list navigation and direct numbered option selection. CodexFlow should add dedicated labels/hotkeys only if needed rather than creating another modal framework.

For a warning/high-risk report the default selection changes to Cancel/Quarantine.

A critical report must not offer one-keystroke harness integration.

## Install as skill

Normal installation remains the cheap path.

```text
approved report
   |
   v
copy/install extension
   |
   v
record source commit/hash + scan result + granted authority
   |
   v
register lazy routing card
```

The installed skill is **not** automatically put into every prompt. It becomes a lazy specialist module and is surfaced only by adaptive capability selection or explicit user mention.

Updating an installed skill re-runs the security transaction because the source hash changed.

## Integrate into CodexFlow harness

The user can choose to turn a useful skill into native harness behavior, but the running harness must never rewrite itself in place.

```text
TUI: Integrate into CodexFlow harness
                |
                v
CapabilityMigrationPlanner
                |
        classify destination
        kernel / flow / profile /
        tool adapter / project playbook
                |
                v
create CodexFlow worktree + branch
                |
                v
edit harness source + capability manifest
                |
                v
focused source checks
                |
                v
independent review
                |
                v
GitHub CI / prebuilt candidate
                |
                v
transactional runtime promotion
```

The original skill remains quarantined/on-demand until the new harness build is independently validated.

This is self-extension, not self-modification.

## Harness extension API

Long-term, promoted capabilities should target stable extension points rather than modifying arbitrary core files.

Suggested phases:

- `BeforePrompt` — project/task classification and compact profile selection.
- `BeforeToolCall` — security/action interceptors.
- `AfterToolCall` — evidence capture, failure tracking, circuit-breaker signals.
- `AfterFirstGreen` — simplification and targeted review scheduling.
- `BeforeCompletion` — evidence and blocking gates.
- `BeforeDelivery` — PR/release/deploy qualification.
- `Scheduled` — caretaker/security/dependency scans.

Capability metadata should declare:

```text
id
version
trigger
phase
risk class
context cost
required tools
permissions
blocking behavior
project selectors
```

Core loads this metadata into a small routing index; detailed implementation remains outside model context.

## Better default frontend output

The generic frontend quality engine distills the overlapping portions of Design, Taste, Impeccable, high-end visual design, design-system, and UI styling into one default profile.

It should enforce concepts such as:

- deliberate hierarchy instead of template section stacking
- coherent typography and spacing scale
- restrained surfaces/effects
- responsive composition rather than desktop-first shrinkage
- accessible interaction states
- project-native component reuse
- motion only when it contributes to hierarchy or feedback
- no gratuitous gradients, glass cards, generic icon rows, or fake dashboard filler

Specific aesthetics remain lazy modules. A user asking for brutalist UI should get the brutalist module; everyone should not receive brutalist instructions.

## Better default writing output

The writing quality engine should apply to user-facing prose without requiring `humanizer` to be called.

It should suppress:

- generic throat-clearing
- repetitive sentence cadence
- synthetic emphasis and filler
- unnecessary sectioning
- vague promotional language

It must preserve technical precision and requested register. Exact/legal/verbatim transformations bypass stylistic normalization.

## Better default code output

Minimal-change policy combines Ponytail, one-line-first, and change-budget below the model:

- discover existing authority before adding a new one
- prefer existing helpers/native features
- new files/dependencies/crates default to zero
- reject convenience abstractions without a real boundary
- after first green, perform one bounded simplification pass
- verify before completion

This is more reliable than hoping the model remembered a skill name.

## Domain knowledge remains lazy

Some skills should intentionally remain modules because making them universal would reduce quality and speed:

- DPDK/OpenOnload/kernel-bypass
- Linux NIC/NUMA fastpath tuning
- PTP/PHC time synchronization
- distributed VPS/WAN optimization
- Three.js/WebGPU
- specific visual aesthetics
- infrastructure provisioning
- chaos experiments
- deep quant validation
- financial-core independent review
- repository-specific Codex maintenance playbooks

These are activated by project capabilities and task evidence, often into fresh specialist subagents rather than the root context.

## Project capability manifest

Projects should declare what they are, not duplicate the entire harness policy.

Example:

```json
{
  "languages": ["rust"],
  "frameworks": [],
  "domains": ["trading", "low_latency"],
  "departments": ["security", "sre", "data", "quant"],
  "path_classes": {
    "financial_core": ["crates/trinity-oms/**", "crates/trinity-execution/**"],
    "protocol_parser": ["crates/trinity-gateway/**"]
  }
}
```

CodexFlow caches the project fingerprint. Normal turns should not rediscover the repository stack from scratch.

## Migration invariant

A skill is removed from the model-visible default catalog only after its replacement path is operational.

The migration order should be:

1. implement native capability
2. test trigger and bypass behavior
3. compare outcomes against the original skill
4. remove skill from default exposure
5. retain compatibility alias if necessary

This prevents a capability migration from silently reducing quality.
