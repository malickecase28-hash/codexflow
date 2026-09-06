# Capability migration inventory

This directory is the design input for migrating the current skill-heavy setup into native CodexFlow behavior.

## Inventory covered

The map covers 137 canonical capability/skill entries across:

- 3 default backbone controls
- 27 personal/design skills
- 8 low-latency skills
- 20 Codex/plugin-provided skills
- 6 Ponytail commands
- 14 Rust maintainer skills
- 15 lean-shipping skills
- 14 Superpowers process skills
- 18 Engineering OS department skills
- 11 Codex repository-local skills
- NVIDIA SkillSpector as the install-security scanner

Aliases are recorded in `skill-migration-map.json` rather than counted as separate capabilities.

## Files

- `skill-migration-map.json` — machine-readable source-to-harness mapping.
- `ARCHITECTURE.md` — runtime model, install guard, TUI decision flow, self-extension path, speed/context rules.

## Rollout order

Do not remove the existing skills in one cutover.

1. **Native install guard** — SkillSpector + quarantine + TUI approval + extension ledger.
2. **Kernel policies** — minimal-change, completion evidence, process controller, build-cost manager.
3. **Tool conversion** — browsers/connectors/artifact tools stop consuming skill-catalog context.
4. **Default quality profiles** — frontend, writing, Rust, low-latency, observability, typed-core.
5. **Lifecycle flows** — planning/debugging/review/delivery/caretaker/release.
6. **Department gates** — security, SRE, architecture, data, quant, financial, deployment.
7. **Lazy specialist migration** — rare style/domain modules become capability plugins with small routing cards.
8. **Remove redundant skill exposure** only after outcome parity/regression tests pass.

## Success criteria

- A skill/install request cannot bypass the install-security gate.
- Frontend and writing tasks receive improved defaults without the user naming a skill.
- Normal code tasks inherit minimal-change and verification policy without loading those skills.
- A Rust-only project does not wake frontend/quant/low-latency modules unless task/project evidence requires them.
- Rare expert modules are invisible to the root context until selected.
- Installed skills can remain ordinary lazy skills or be proposed for harness integration.
- Harness integration creates a branch/worktree and validated release candidate; it never mutates the running harness in place.
- Capability routing adds negligible latency for common tasks and does not require an LLM classifier in the normal path.
