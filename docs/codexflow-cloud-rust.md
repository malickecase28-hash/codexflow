# CodexFlow cloud Rust workflow

CodexFlow supports two complementary GitHub-hosted Rust paths.

## Interactive development: GitHub Codespaces

Use Codespaces when the local machine does not have a usable Rust installation or when you want a reproducible Linux development environment.

1. Open the repository on GitHub.
2. Choose **Code → Codespaces → Create codespace** on the branch you want to develop.
3. Wait for the devcontainer bootstrap to finish.
4. Open a terminal and enter the Rust workspace:

```bash
cd codex-rs
rustc --version
cargo --version
```

The devcontainer installs Rust 1.98.1, matching `codex-rs/rust-toolchain.toml`, and prepares the same prebuilt `rusty_v8` artifact convention used by CI.

Build and run normally:

```bash
cargo build -p codex-cli --bin codexflow
cargo run -p codex-cli --bin codexflow -- --help
cargo test -p codex-cli --bin codexflow
```

To run the focused compiler check used during CodexFlow development:

```bash
bash .devcontainer/bootstrap-rust.sh --check
```

The bootstrap persists the verified `rusty_v8` paths in `~/.codexflow-rust-env`, which new Bash terminals source automatically.

## Deterministic verification: GitHub Actions

The `codexflow-source-check` workflow is the authoritative remote compile/test signal for CodexFlow runtime changes. It runs the pinned Rust toolchain, restores Cargo dependencies, uses `sccache`, configures verified `rusty_v8` artifacts, checks both runtime binaries, and runs the focused behavior suites.

For a project managed by the CodexFlow CLI, `codexflow build verify` additionally records exact command, exit-code, duration, revision, dirty-state, and evidence metadata locally. Delivery requires successful exact-HEAD verification for Rust projects unless an operator explicitly uses the emergency unverified escape hatch.

## Operating rule

Use Codespaces for interactive edit/build/run/debug loops. Use Actions and `codexflow build verify` as independent evidence before delivery. A local Rust installation is optional for this workflow.
