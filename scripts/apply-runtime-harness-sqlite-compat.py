import shutil
import subprocess
import tempfile
from pathlib import Path

PIN = "c839bd4de69397612d09fdc9312e03cf6e9c9e05"
UPSTREAM = "https://github.com/x0c/subswap"
VENDOR = Path("third_party/subswap-provider-cursor-compat")


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected exactly one patch anchor, found {count}")
    path.write_text(text.replace(old, new, 1))


with tempfile.TemporaryDirectory(prefix="subswap-cursor-") as temp_dir:
    checkout = Path(temp_dir) / "subswap"
    subprocess.run(["git", "init", "-q", str(checkout)], check=True)
    subprocess.run(["git", "-C", str(checkout), "remote", "add", "origin", UPSTREAM], check=True)
    subprocess.run(
        ["git", "-C", str(checkout), "fetch", "-q", "--depth", "1", "origin", PIN],
        check=True,
    )
    subprocess.run(["git", "-C", str(checkout), "checkout", "-q", "FETCH_HEAD"], check=True)
    resolved = subprocess.check_output(
        ["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True
    ).strip()
    if resolved != PIN:
        raise SystemExit(f"subswap pin mismatch: expected {PIN}, got {resolved}")

    source = checkout / "crates/providers/cursor"
    if VENDOR.exists():
        shutil.rmtree(VENDOR)
    (VENDOR / "src").mkdir(parents=True)
    shutil.copy2(source / "src/lib.rs", VENDOR / "src/lib.rs")
    shutil.copy2(source / "src/tests.rs", VENDOR / "src/tests.rs")
    shutil.copy2(checkout / "LICENSE", VENDOR / "LICENSE")

(VENDOR / "Cargo.toml").write_text(
    f'''[package]\nname = "subswap-provider-cursor"\nversion = "1.6.1"\nedition = "2021"\nrust-version = "1.80"\nlicense = "MIT"\npublish = false\nrepository = "{UPSTREAM}"\n\n[dependencies]\nsubswap-core = {{ git = "{UPSTREAM}", rev = "{PIN}" }}\nanyhow = "1"\nasync-trait = "0.1"\nbase64 = "0.22"\nchrono = {{ version = "0.4", features = ["serde"] }}\ndirectories = "5"\nfs2 = "0.4"\nreqwest = {{ version = "0.12", default-features = false, features = ["json", "rustls-tls"] }}\nrusqlite = {{ version = "0.39", features = ["bundled"] }}\nserde = {{ version = "1", features = ["derive"] }}\nserde_json = "1"\nsha2 = "0.10"\ntokio = {{ version = "1", features = ["macros", "rt-multi-thread", "signal", "time", "fs", "sync"] }}\ntracing = "0.1"\n\n[dev-dependencies]\ntempfile = "3"\n'''
)
(VENDOR / "README.md").write_text(
    f'''# subswap Cursor provider compatibility copy\n\nThis directory contains the Cursor provider source copied verbatim from\n`x0c/subswap` commit `{PIN}`.\n\nThe only compatibility delta is in this local `Cargo.toml`: `rusqlite` is raised\nfrom upstream `0.32` to `0.39` so the provider resolves against the same\n`libsqlite3-sys 0.37` native link already owned by the Codex workspace. The Rust\nsource is intentionally unchanged.\n\nUpstream license: MIT; see `LICENSE`.\n'''
)

harness_manifest = Path("codex-rs/runtime-harness/Cargo.toml")
replace_once(
    harness_manifest,
    f'subswap-provider-cursor = {{ git = "{UPSTREAM}", rev = "{PIN}" }}\n',
    'subswap-provider-cursor = { path = "../../third_party/subswap-provider-cursor-compat" }\n',
)
