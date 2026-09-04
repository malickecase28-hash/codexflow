# CodexFlow npm launcher

This package installs prebuilt CodexFlow native binaries from a matching GitHub Release. It never invokes Cargo, rustc, or a local linker during installation.

## Install directly from GitHub

Choose a published CodexFlow release version and install its npm tarball:

```bash
VERSION=0.1.0-alpha.1
npm install -g "https://github.com/malickecase28-hash/codexflow/releases/download/codexflow-v${VERSION}/codexflow-npm-${VERSION}.tgz"
```

Then run:

```bash
codexflow --help
codexflow-supervisor --help
```

The npm package resolves the current OS and CPU architecture, downloads the matching prebuilt release binaries, verifies each binary against the release `checksums.txt`, and installs the verified bytes under the package-local `vendor` directory.

Supported release targets:

- Linux x64: `x86_64-unknown-linux-gnu`
- Linux arm64: `aarch64-unknown-linux-gnu`
- macOS x64: `x86_64-apple-darwin`
- macOS arm64: `aarch64-apple-darwin`
- Windows x64: `x86_64-pc-windows-msvc`
- Windows arm64: `aarch64-pc-windows-msvc`

## Release contract

A release named `codexflow-v<version>` contains:

- `codexflow-<target>[.exe]`
- `codexflow-supervisor-<target>[.exe]`
- `checksums.txt`
- `codexflow-npm-<version>.tgz`

The package version and release tag must match. A development checkout uses version `0.0.0-dev`; for development-only installation tests, set `CODEXFLOW_RELEASE_TAG` explicitly or set `CODEXFLOW_SKIP_DOWNLOAD=1`.

## Optional installer overrides

- `CODEXFLOW_RELEASE_TAG`: override the inferred `codexflow-v<package version>` tag.
- `CODEXFLOW_RELEASE_BASE_URL`: override the GitHub Release asset base URL.
- `CODEXFLOW_SKIP_DOWNLOAD=1`: skip native download for package-development tests only.

These overrides are not required for normal installs from a published GitHub Release.
