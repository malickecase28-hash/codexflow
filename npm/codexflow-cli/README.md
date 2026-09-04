# CodexFlow GitHub package

This package installs precompiled CodexFlow binaries from the repository's `codexflow-edge` GitHub Release. It does not compile Rust on the target machine.

Supported targets:

- Windows x64
- Linux x64

Install the edge package directly from GitHub:

```bash
npm install -g https://github.com/malickecase28-hash/codexflow/releases/download/codexflow-edge/codexflow-cli.tgz
```

Then run:

```bash
codexflow --version
codexflow doctor
```

The package also installs the `codexflow-supervisor` command. Internally it downloads the matching precompiled `codex`, `codexflow`, `codexflow-supervisor`, and `codex-code-mode-host` binaries and verifies every download against the release SHA-256 manifest.

For reproducible installation of a named release, set `CODEXFLOW_RELEASE_TAG` before installing a package tarball built for that release.
