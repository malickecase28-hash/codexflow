# CodexFlow Benchmark and Evidence Framework

## Goal

CodexFlow should prove whether harness behavior improves outcomes. The benchmark system measures the harness separately from the model so we can answer questions such as:

- Does CodexFlow reduce input/output tokens for the same task?
- Does it reduce time to first useful action and time to verified completion?
- Does it reduce unnecessary files, dependencies, abstractions, and diff size?
- Does it improve test pass rate and independent review quality?
- Can a cheaper/faster model inside CodexFlow match or beat a stronger model in vanilla Codex?
- Which capability policies are responsible for the improvement or regression?

This must exist early in the project so later optimization is evidence-driven rather than based on impressions.

## Benchmark principle

The unit under test is:

```text
model + harness + project snapshot + task
```

A model is not compared across different repository states or different acceptance tests.

The primary comparisons are paired runs from the same immutable project snapshot.

## Standard variants

A benchmark suite should support at least:

```text
A. vanilla Codex + model X
B. CodexFlow     + model X
C. vanilla Codex + stronger model Y
D. CodexFlow     + cheaper/faster model Z
```

This enables two important measurements:

1. **Harness uplift**: B versus A.
2. **Model substitution value**: D versus C.

The second is the test for the user's central thesis: a disciplined harness may allow a weaker or cheaper model to produce work that resembles a stronger model's output.

## No benchmark contamination

Each run starts from:

- the same git commit/snapshot
- a fresh worktree
- a fresh model session unless the suite explicitly measures continuation
- the same task text
- the same project policies
- the same tool/network permissions where possible
- the same hidden verification corpus

Runs must not see previous benchmark answers, review findings, or expected patches.

## Metrics

### Resource efficiency

Record:

- prompt/input tokens
- cached input tokens where provider exposes them
- output tokens
- total model tokens
- number of model requests
- number of tool calls
- tool-result bytes/tokens exposed to the model
- context compactions
- skill/profile context tokens
- capability routing decisions
- suppressed skill catalog tokens
- subagent count
- subagent tokens
- wall-clock duration
- model-active duration when available
- tool-active duration
- build/test duration
- time to first tool call
- time to first edit
- time to first green check
- time to verified completion
- estimated provider cost when pricing metadata exists

### Change efficiency

Record:

- files touched
- files created
- files deleted
- lines added
- lines deleted
- dependencies added/removed
- crates/packages/modules added
- abstractions/traits/helpers added where statically detectable
- build scope chosen (`check`, focused test, dev build, release build)
- worktree cleanliness after run

These metrics directly measure Ponytail/minimal-change and fast-feedback outcomes.

### Correctness and delivery

Record:

- task acceptance tests passed/failed
- hidden tests passed/failed
- compiler/static-analysis status
- regression tests
- independent reviewer findings by severity
- breaking-change findings
- security findings
- unresolved blockers
- whether completion was claimed before evidence existed
- whether a PR was created when required
- whether human-review/merge policy was preserved

### Runtime quality

Record:

- repeated tool-call loops
- retries
- error storms
- no-progress circuit-breaker events
- incorrect capability activation
- missing capability activation
- stale-context incidents
- background service interference
- unauthorized lifecycle/action attempts

### Artifact quality

For UI/design tasks:

- screenshot/reference similarity when a reference exists
- accessibility checks
- responsive checks
- layout overflow/errors
- performance checks
- deterministic design-rubric results
- independent visual review score only where subjective judgment is unavoidable

For writing tasks:

- explicit task constraints satisfied
- grammar/readability checks where appropriate
- banned/generic style pattern counts
- independent rubric score for voice/naturalness only in benchmark mode

A judge-model call is permitted inside benchmark mode because the benchmark is explicitly measuring output quality. Judge use must never silently become part of normal task execution.

## Outcome hierarchy

The benchmark should not optimize raw token reduction at the expense of correctness.

Ranking priority:

```text
1. safety / authority correctness
2. task correctness
3. verification quality
4. regression avoidance
5. delivery correctness
6. time to completion
7. token/cost efficiency
8. diff/repository efficiency
```

A 50% token reduction that introduces a correctness regression is a benchmark loss.

## Capability attribution

Every CodexFlow run writes a compact activation trace:

```json
{
  "kernel": ["policy.minimal_change", "completion.evidence_gate"],
  "profiles": ["profile.rust"],
  "flows": ["verification.rust", "review.engine"],
  "tools": ["navigation.semantic"],
  "specialists": [],
  "context_tokens_added": 412,
  "skill_catalog_tokens_suppressed": 18450
}
```

This makes it possible to distinguish:

- model improvement
- harness routing improvement
- context-hygiene improvement
- agent topology cost
- verification/review cost

## Benchmark corpus

Maintain several classes rather than one synthetic score.

### Tiny fixes

Examples:

- one-line bug
- typo/config error
- missing error propagation
- small API misuse

Measure whether the harness avoids over-planning and over-agenting.

### Normal engineering

Examples:

- bounded feature
- async bug
- parser bug
- frontend component
- test repair

Measure correctness, diff size, verification and completion time.

### Cross-cutting engineering

Examples:

- multi-crate/API change
- architecture-sensitive refactor
- deployment/build change

Measure topology selection and integration/review behavior.

### High-risk changes

Examples:

- security boundary
- financial/trading invariant
- unsafe/FFI
- release/deployment

Measure whether required gates activate and unsafe shortcuts are prevented.

### Design / writing

Measure first-pass artifact quality and whether default profiles remove generic output without excessive token overhead.

### Failure recovery

Inject known failures and measure systematic debugging, self-heal, retry loops, and draft-PR quality.

## Golden tasks and hidden verification

A benchmark task should contain:

```text
id
project fixture/version
user prompt
allowed tools
expected authority
public acceptance criteria
hidden verification command(s)
risk class
artifact class
```

Do not embed a golden patch unless a task truly has one canonical implementation. Prefer behavioral verification.

## Repeatability

LLM outputs are stochastic. Important benchmark cells should support repeated runs.

Report:

- median
- p90 where enough samples exist
- success rate
- variance
- outliers

Do not declare a harness win from one lucky run.

## Benchmark storage

Suggested layout outside the project source tree or in a dedicated evidence directory:

```text
codexflow-benchmarks/
  suites/
  fixtures/
  runs/<run-id>/
    run.json
    events.jsonl
    activation.json
    metrics.json
    diff.patch
    verification.json
    review.json
    artifacts/
  reports/
```

A run record should include:

- CodexFlow commit/build identity
- base Codex commit
- model/provider/reasoning effort
- project snapshot commit
- capability manifest version
- benchmark task version
- environment/OS/toolchain identity
- start/end timestamps

## Benchmark command surface

Target CLI/TUI concepts:

```text
codexflow bench init
codexflow bench run <suite>
codexflow bench compare <run-or-variant> <run-or-variant>
codexflow bench report <suite>
codexflow bench trace <run-id>
```

Useful convenience:

```text
codexflow bench matrix \
  --models luna,sol \
  --harness vanilla,codexflow \
  --suite rust-core
```

Exact model identifiers remain provider configuration, not hard-coded product assumptions.

## Background benchmark service

The benchmark runner is an ideal protected service agent.

Triggers:

- nightly/weekly schedule
- CodexFlow capability change
- release candidate
- model configuration change
- explicit user request

It runs isolated fixtures and writes evidence to the benchmark store. On next terminal open GOD reports only the summary:

```text
Benchmark complete: rust-core-12
CodexFlow/lower-cost model: 10/12 pass
Vanilla/strong model baseline: 9/12 pass
Tokens: -41%
Median completion time: -24%
No new severe review findings
```

The user can then inspect the detailed report.

## Performance budget for the harness itself

The benchmark system must measure CodexFlow overhead separately.

Track before the first model request:

- project resolution time
- capability routing time
- policy/profile assembly time
- tool pruning time
- context-hygiene time
- supervisor/service lookup time

Common-path harness overhead should be small enough that a trivial task does not feel slower than vanilla Codex.

The router should remain deterministic/cached and avoid an LLM classifier on the common path.

## Optimization loop

```text
benchmark
   |
   v
identify regression/bottleneck
   |
   v
change harness
   |
   v
paired benchmark
   |
   +-- quality down -> reject/rework
   |
   +-- overhead up without value -> reject/rework
   |
   v
promote capability/runtime
```

This is the evidence loop for the harness itself.
