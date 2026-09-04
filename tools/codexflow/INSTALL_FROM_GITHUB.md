# Install CodexFlow from GitHub

CodexFlow publishes precompiled edge binaries and an npm installer package to the `codexflow-edge` GitHub Release. Rust is not required on the machine that installs the release.

## npm

Windows PowerShell, Command Prompt, Linux, or a Codespace with Node.js 18 or newer:

```text
npm install -g https://github.com/malickecase28-hash/codexflow/releases/download/codexflow-edge/codexflow-cli.tgz
```

Verify the installation:

```text
codexflow --version
codexflow-supervisor --version
codexflow doctor
```

The npm package downloads the platform-specific precompiled runtime from the same release and validates SHA-256 checksums before making the binaries available.

## Manual bundles

The same GitHub Release also contains platform bundles:

- `codexflow-linux-x86_64.tar.gz`
- `codexflow-windows-x86_64.zip`
- `SHA256SUMS.txt`

Each bundle contains `codex`, `codexflow`, `codexflow-supervisor`, and `codex-code-mode-host` so `codexflow run` can find the required sibling Codex executable.
