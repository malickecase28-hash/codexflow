from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one patch anchor, found {count}")
    path.write_text(text.replace(old, new, 1))


root = Path("codex-rs/Cargo.toml")
replace_once(
    root,
    '    "rollout-trace",\n    "rmcp-client",\n',
    '    "rollout-trace",\n    "runtime-harness",\n    "rmcp-client",\n',
)
replace_once(
    root,
    'codex-rollout-trace = { path = "rollout-trace" }\n',
    'codex-rollout-trace = { path = "rollout-trace" }\ncodex-runtime-harness = { path = "runtime-harness" }\n',
)

harness_cargo = Path("codex-rs/runtime-harness/Cargo.toml")
replace_once(
    harness_cargo,
    '''[workspace]\n\n[workspace.package]\nversion = "0.0.0"\nedition = "2024"\nlicense = "Apache-2.0"\n\n[workspace.lints.rust]\n\n[workspace.lints.clippy]\n\n''',
    '',
)

cargo = Path("codex-rs/tui/Cargo.toml")
replace_once(
    cargo,
    "codex-protocol = { workspace = true }\n",
    "codex-protocol = { workspace = true }\ncodex-runtime-harness = { workspace = true }\n",
)

lib = Path("codex-rs/tui/src/lib.rs")
replace_once(
    lib,
    "mod resume_picker;\n",
    "mod resume_picker;\nmod runtime_bridge;\n",
)

app = Path("codex-rs/tui/src/app.rs")
replace_once(
    app,
    "use crate::resume_picker::SessionTarget;\n",
    "use crate::resume_picker::SessionTarget;\nuse crate::runtime_bridge::RuntimeBridge;\n",
)
replace_once(
    app,
    "pub(crate) struct App {\n    model_catalog: Arc<ModelCatalog>,\n",
    "pub(crate) struct App {\n    model_catalog: Arc<ModelCatalog>,\n    pub(crate) runtime_bridge: RuntimeBridge,\n",
)

startup = Path("codex-rs/tui/src/app/startup.rs")
replace_once(
    startup,
    "        let mut app = Self {\n            model_catalog,\n",
    "        let runtime_bridge = RuntimeBridge::new(&config, &model)\n            .wrap_err(\"failed to initialize multi-runtime harness\")?;\n\n        let mut app = Self {\n            model_catalog,\n            runtime_bridge,\n",
)
replace_once(
    startup,
    "        if let Err(err) = app_server.shutdown().await {\n            tracing::warn!(error = %err, \"failed to shut down embedded app server\");\n        }\n",
    "        if let Err(err) = app.runtime_bridge.shutdown().await {\n            tracing::warn!(error = %err, \"failed to shut down multi-runtime harness\");\n        }\n        if let Err(err) = app_server.shutdown().await {\n            tracing::warn!(error = %err, \"failed to shut down embedded app server\");\n        }\n",
)
