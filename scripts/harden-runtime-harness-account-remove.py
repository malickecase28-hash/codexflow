import re
from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one patch anchor, found {count}: {old!r}")
    path.write_text(text.replace(old, new, 1))


accounts = Path("codex-rs/runtime-harness/src/accounts.rs")
replace_once(
    accounts,
    "        Self::with_components(providers, HashMap::new(), None)",
    "        Self::with_importers(providers, HashMap::new())",
)

acp = Path("codex-rs/runtime-harness/src/cursor_acp.rs")
replace_once(
    acp,
    "    pub executable: Option<PathBuf>,\n    pub process_cwd: Option<PathBuf>,",
    "    pub executable: Option<PathBuf>,\n"
    "    /// Optional launcher arguments inserted before the required `acp` subcommand.\n"
    "    /// This supports stable wrappers/interpreters without changing the official\n"
    "    /// `agent acp` default invocation.\n"
    "    pub launcher_args: Vec<String>,\n"
    "    pub process_cwd: Option<PathBuf>,",
)
replace_once(
    acp,
    '            command.arg("acp");',
    '            command.args(&config.launcher_args);\n            command.arg("acp");',
)

integration = Path("codex-rs/runtime-harness/tests/cursor_acp_mock.rs")
text = integration.read_text()
text = text.replace("use std::os::unix::fs::PermissionsExt;\n", "")
old = """fn write_mock_agent(directory: &Path) -> std::path::PathBuf {
    let path = directory.join("mock-agent");
    fs::write(&path, MOCK_AGENT).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}
"""
new = """fn mock_config(directory: &Path) -> CursorAcpConfig {
    let script = directory.join("mock-agent.py");
    fs::write(&script, MOCK_AGENT).unwrap();
    CursorAcpConfig {
        executable: Some("python3".into()),
        launcher_args: vec![script.to_string_lossy().into_owned()],
        process_cwd: Some(directory.to_path_buf()),
    }
}
"""
count = text.count(old)
if count != 1:
    raise SystemExit(f"{integration}: expected one mock writer, found {count}")
text = text.replace(old, new, 1)
executable_decl = "    let executable = write_mock_agent(temp.path());\n"
count = text.count(executable_decl)
if count != 6:
    raise SystemExit(
        f"{integration}: expected six mock executable declarations, found {count}"
    )
text = text.replace(executable_decl, "")
config_literal = re.compile(
    r"CursorAcpConfig \{\n"
    r"(?P<indent>[ \t]+)executable: Some\(executable\),\n"
    r"(?P=indent)process_cwd: Some\(temp\.path\(\)\.to_path_buf\(\)\),\n"
    r"[ \t]+\}",
)
count = len(config_literal.findall(text))
if count != 6:
    raise SystemExit(f"{integration}: expected six mock config literals, found {count}")
text = config_literal.sub("mock_config(temp.path())", text)
integration.write_text(text)
