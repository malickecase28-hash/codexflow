# Capability/Profile Migration Coverage Contract

## Purpose

When multiple skills are distilled into one native CodexFlow policy/profile, the migration must not cherry-pick a few memorable ideas and silently discard the rest.

A merged profile is accepted only after every normative source rule is accounted for.

## Coverage rule

For every source skill selected for migration:

1. Parse its instructions into atomic normative rules.
2. Give every rule a stable source id.
3. Classify every rule into exactly one disposition.
4. Preserve source/version/hash evidence.
5. Run outcome/regression tests before removing the source skill from normal exposure.

Allowed rule dispositions:

```text
retained_core
    Distilled into the common harness profile.

conditional_overlay
    Important, but only correct when project/task evidence matches.

specialist_module
    Valuable deep/style-specific knowledge; remains lazy rather than becoming a global default.

tool_or_flow
    Better enforced as deterministic tool routing, event flow, or gate.

superseded
    Replaced by a stricter or more general rule. The replacement must be named.

rejected_conflict
    Conflicts with a higher-authority rule or would damage generic behavior. A written reason is mandatory.
```

`unmapped` is never valid for a completed migration.

## Migration ledger

Each merged profile gets a ledger similar to:

```json
{
  "profile": "quality.frontend",
  "sources": [
    {
      "name": "design",
      "source_hash": "...",
      "rules_total": 42,
      "rules_accounted": 42
    }
  ],
  "rules": [
    {
      "id": "design:typography:03",
      "source": "design",
      "disposition": "retained_core",
      "target": "frontend.typography.hierarchy",
      "notes": "Preserved as a generic quality default."
    }
  ]
}
```

The source hash changes on update, invalidating the migration coverage until the changed rules are reviewed.

## Frontend profile source set

The current generic frontend profile is sourced from:

- `design`
- `design-system`
- `gpt-taste` / `gpt-tasteskill`
- `high-end-visual-design`
- `impeccable`
- `taste-skill` / `design-taste-frontend`
- `ui-styling`

These sources must be fully extracted before `quality.frontend` is implemented as a replacement for them.

The intent is **not** to average them into generic styling advice. The migration should find:

- common rules that deserve always-on frontend defaults
- conditional rules for specific frameworks, artifact types, motion, brand systems, or redesign tasks
- stylistic opinions that should remain selectable specialists rather than becoming universal
- quality checks that are better implemented as deterministic verification

This distinction protects both quality and variety.

## Example of correct synthesis

Suppose one source says:

```text
Use deliberate typographic hierarchy.
```

and another says:

```text
Avoid default framework typography and choose type deliberately.
```

These can be synthesized into one core invariant.

But if a source says:

```text
Use asymmetric editorial layouts.
```

that may become a conditional or specialist rule rather than a global requirement, because forcing asymmetry on every admin table or settings screen would be inappropriate.

Likewise, a rule about GSAP should not disappear; it may move to a motion/framework overlay rather than the generic frontend core.

## No generic-AI regression gate

The frontend migration is not complete merely because the code is shorter than the seven source skills.

Before retiring source exposure, benchmark the merged profile against the source skills on a design corpus containing:

- landing pages
- dashboards
- dense data/admin interfaces
- mobile-responsive pages
- forms/settings
- redesigns of existing products
- screenshot-to-code work
- accessible interaction states
- motion-heavy pages

Compare:

- first-pass visual hierarchy
- typography quality
- component coherence
- responsive behavior
- accessibility
- visual distinctiveness
- unnecessary visual effects
- template-pattern frequency
- implementation complexity
- token/context cost

If the compact profile is cheaper but produces visibly more generic work, the migration failed.

## Apply this contract beyond design

The same rule applies to every merged family.

Examples:

### Rust

`rust-core-engineering`, `rust-async-tokio`, and `rust-concurrency-determinism` require rule-level coverage before the compact Rust profile replaces them.

### Low latency

Common low-latency guidance can move into a compact profile, but advanced Linux fastpath, PTP, kernel bypass, and distributed WAN guidance must remain reachable as specialists rather than being discarded.

### Lean shipping

When the lean pack dissolves into kernel policies and lifecycle flows, every original invariant must map to a deterministic policy, gate, flow, metric, specialist, or documented rejection.

### Engineering OS

Department skills become routing/gates, but their acceptance criteria and evidence requirements must survive the conversion.

## Migration implementation process

```text
source skills
    |
    v
RuleExtractor
    |
    v
coverage ledger
    |
    +--> common invariants ------> compact profile/kernel
    +--> conditional rules -----> overlays
    +--> procedural rules ------> flows/gates/tools
    +--> deep knowledge --------> specialist modules
    |
    v
benchmark against source behavior
    |
    v
independent review
    |
    v
only then reduce/remove source skill exposure
```

This process can be automated substantially, but final conflict resolution is a harness-development change and should receive normal review.
