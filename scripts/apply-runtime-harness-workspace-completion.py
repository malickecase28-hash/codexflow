from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    text = target.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected exactly one patch anchor, found {count}")
    target.write_text(text.replace(old, new, 1))


# Join the runtime harness to the shipping Cargo workspace.
replace_once(
    "codex-rs/Cargo.toml",
    '    "rollout-trace",\n    "rmcp-client",\n',
    '    "rollout-trace",\n    "runtime-harness",\n    "rmcp-client",\n',
)
replace_once(
    "codex-rs/Cargo.toml",
    'codex-rollout-trace = { path = "rollout-trace" }\n',
    'codex-rollout-trace = { path = "rollout-trace" }\n'
    'codex-runtime-harness = { path = "runtime-harness" }\n',
)

# The harness was previously an intentionally isolated nested workspace. Once it is
# a root member, package metadata and lint configuration come from codex-rs.
replace_once(
    "codex-rs/runtime-harness/Cargo.toml",
    '[workspace]\n\n'
    '[workspace.package]\n'
    'version = "0.0.0"\n'
    'edition = "2024"\n'
    'license = "Apache-2.0"\n\n'
    '[workspace.lints.rust]\n\n'
    '[workspace.lints.clippy]\n\n',
    '',
)

replace_once(
    "codex-rs/tui/Cargo.toml",
    'codex-protocol = { workspace = true }\n',
    'codex-protocol = { workspace = true }\n'
    'codex-runtime-harness = { workspace = true }\n',
)

# Link the already-landed bridge source into the TUI module graph.
replace_once(
    "codex-rs/tui/src/lib.rs",
    'mod resume_picker;\nmod selection_list;\n',
    'mod resume_picker;\nmod runtime_bridge;\nmod selection_list;\n',
)

replace_once(
    "codex-rs/tui/src/app.rs",
    'use crate::resume_picker::SessionTarget;\nuse crate::session_state::ThreadSessionState;\n',
    'use crate::resume_picker::SessionTarget;\nuse crate::runtime_bridge::RuntimeBridge;\n'
    'use crate::session_state::ThreadSessionState;\n',
)
replace_once(
    "codex-rs/tui/src/app.rs",
    'pub(crate) struct App {\n    model_catalog: Arc<ModelCatalog>,\n',
    'pub(crate) struct App {\n    model_catalog: Arc<ModelCatalog>,\n'
    '    pub(crate) runtime_bridge: RuntimeBridge,\n',
)

# Construct exactly one process-lifetime bridge. Cursor remains lazy inside
# RuntimeHarness::embedded, so startup does not launch `agent acp`.
replace_once(
    "codex-rs/tui/src/app/startup.rs",
    '        let mut app = Self {\n            model_catalog,\n            session_telemetry: session_telemetry.clone(),\n',
    '        let runtime_bridge = RuntimeBridge::new(&config, &model)\n'
    '            .wrap_err("failed to initialize multi-runtime harness")?;\n\n'
    '        let mut app = Self {\n'
    '            model_catalog,\n'
    '            runtime_bridge,\n'
    '            session_telemetry: session_telemetry.clone(),\n',
)
replace_once(
    "codex-rs/tui/src/app/startup.rs",
    '        if let Err(err) = app_server.shutdown().await {\n'
    '            tracing::warn!(error = %err, "failed to shut down embedded app server");\n'
    '        }\n',
    '        if let Err(err) = app.runtime_bridge.shutdown().await {\n'
    '            tracing::warn!(error = %err, "failed to shut down multi-runtime harness");\n'
    '        }\n'
    '        if let Err(err) = app_server.shutdown().await {\n'
    '            tracing::warn!(error = %err, "failed to shut down embedded app server");\n'
    '        }\n',
)
