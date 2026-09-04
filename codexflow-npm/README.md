# CodexFlow npm launcher

This package installs a complete prebuilt CodexFlow runtime from a matching GitHub Release. It never invokes Cargo, rustc, or a local linker during installation.

## Install the current GitHub edge build

With Node.js 18 or newer installed:

```bash
npm install -g "https://github.com/malickecase28-hash/codexflow/releases/download/codexflow-edge/codexflow-npm-edge.tgz"
```

Then run:

```bash
codexflow --version
codexflow-supervisor --version
codexflow doctor --json
```

The npm package resolves the current OS and CPU architecture, downloads the matching prebuilt release binaries, verifies each binary against the release `checksums.txt`, and installs the verified bytes under the package-local `vendor` directory.

The installed runtime includes four sibling native executables so `codexflow run` does not depend on a separately installed Rust build:

- `codex`
- `codexflow`
- `codexflow-supervisor`
- `codex-code-mode-host`

Supported release targets:

- Linux x64: `x86_64-unknown-linux-gnu`
- Linux arm64: `aarch64-unknown-linux-gnu`
- macOS x64: `x86_64-apple-darwin`
- macOS arm64: `aarch64-apple-darwin`
- Windows x64: `x86_64-pc-windows-msvc`
- Windows arm64: `aarch64-pc-windows-msvc`

## Install a versioned release

For an immutable release such as `0.1.0-alpha.1`:

```bash
VERSION=0.1.0-alpha.1
npm install -g "https://github.com/malickecase28-hash/codexflow/releases/download/codexflow-v${VERSION}/codexflow-npm-${VERSION}.tgz"
```

## Release contract

Each release contains target-specific copies of all four native executables, plus:

- `checksums.txt`
- an npm tarball (`codexflow-npm-edge.tgz` for the moving edge release, or `codexflow-npm-<version>.tgz` for a versioned release)

Every downloaded native executable is verified against `checksums.txt` before installation. The edge tarball embeds `codexflow-edge` as its release tag; versioned tarballs embed their immutable `codexflow-v<version>` tag.

## Optional installer overrides

- `CODEXFLOW_RELEASE_TAG`: override the release tag embedded in the npm tarball.
- `CODEXFLOW_RELEASE_BASE_URL`: override the GitHub Release asset base URL.
- `CODEXFLOW_SKIP_DOWNLOAD=1`: skip native download for package-development tests only.

These overrides are not required for normal installs from a published GitHub Release.
