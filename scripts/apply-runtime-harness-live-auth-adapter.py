from pathlib import Path


path = Path("codex-rs/tui/src/app/startup.rs")
text = path.read_text()
old = '''        let runtime_bridge = RuntimeBridge::new(&config, &model)
            .wrap_err("failed to initialize multi-runtime harness")?;
'''
new = '''        let runtime_bridge = RuntimeBridge::new(
            &config,
            &model,
            app_server.request_handle(),
        )
        .wrap_err("failed to initialize multi-runtime harness")?;
'''
count = text.count(old)
if count != 1:
    raise SystemExit(
        f"codex-rs/tui/src/app/startup.rs: expected one runtime bridge constructor, found {count}"
    )
path.write_text(text.replace(old, new, 1))
