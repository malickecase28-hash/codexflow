#!/usr/bin/env python3
"""Materialize release caching and six-target compiler-free install smoke."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/codexflow-release.yml"


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    return text.replace(old, new, 1)


def main() -> None:
    text = WORKFLOW.read_text()
    text = replace_once(
        text,
        "    permissions:\n      contents: read\n    strategy:\n      fail-fast: false\n",
        "    permissions:\n      contents: read\n    env:\n      CARGO_INCREMENTAL: \"0\"\n      SCCACHE_CACHE_SIZE: 10G\n    strategy:\n      fail-fast: false\n",
        "release build cache env",
    )

    rust_block = '''      - name: Install Rust 1.98.1
        uses: dtolnay/rust-toolchain@ce678459e9fc7500d337468f904b95f1b5c10b5e
        with:
          toolchain: "1.98.1"
          targets: ${{ matrix.target }}

'''
    cached_rust_block = rust_block + '''      - name: Restore Cargo dependency cache
        uses: actions/cache@668228422ae6a00e4ad889ee87cd7109ec5666a7 # v5.0.4
        with:
          path: |
            ~/.cargo/registry/index/
            ~/.cargo/registry/cache/
            ~/.cargo/git/db/
          key: codexflow-release-cargo-${{ runner.os }}-${{ matrix.target }}-rust-1.98.1-${{ hashFiles('codex-rs/Cargo.lock') }}
          restore-keys: |
            codexflow-release-cargo-${{ runner.os }}-${{ matrix.target }}-rust-1.98.1-

      - name: Install sccache
        uses: taiki-e/install-action@44c6d64aa62cd779e873306675c7a58e86d6d532 # v2.62.49
        with:
          tool: sccache

      - name: Configure sccache
        shell: bash
        run: |
          set -euo pipefail
          if [[ -n "${ACTIONS_CACHE_URL:-}" && -n "${ACTIONS_RUNTIME_TOKEN:-}" ]]; then
            echo "SCCACHE_GHA_ENABLED=true" >> "$GITHUB_ENV"
          else
            echo "SCCACHE_GHA_ENABLED=false" >> "$GITHUB_ENV"
            echo "SCCACHE_DIR=${RUNNER_TEMP}/codexflow-sccache" >> "$GITHUB_ENV"
          fi
          echo "RUSTC_WRAPPER=sccache" >> "$GITHUB_ENV"

'''
    text = replace_once(text, rust_block, cached_rust_block, "release Rust cache setup")

    upload_block = '''      - name: Upload native release assets
        uses: actions/upload-artifact@bbbca2ddaa5d8feaa63e36b76fdaad77386f024f # v7.0.0
        with:
          name: codexflow-native-${{ matrix.target }}
          path: dist/*
          if-no-files-found: error
          retention-days: 7

'''
    text = replace_once(
        text,
        upload_block,
        upload_block
        + '''      - name: Show sccache statistics
        if: always()
        shell: bash
        run: sccache --show-stats || true

''',
        "release sccache stats",
    )

    text = replace_once(
        text,
        "    timeout-minutes: 20\n    permissions:\n      contents: write\n",
        "    timeout-minutes: 45\n    permissions:\n      contents: write\n",
        "release assembly timeout",
    )

    smoke_start = text.find("  release-install-smoke-linux:\n")
    if smoke_start < 0:
        raise SystemExit("release install smoke start not found")
    text = text[:smoke_start] + '''  release-install-smoke-unix:
    needs:
      - prepare
      - package-and-release
    name: Published install ${{ matrix.runtime }}
    runs-on: ${{ matrix.runner }}
    timeout-minutes: 30
    strategy:
      fail-fast: false
      matrix:
        include:
          - runner: ubuntu-24.04
            runtime: linux-x64
          - runner: ubuntu-24.04-arm
            runtime: linux-arm64
          - runner: macos-15-intel
            runtime: macos-x64
          - runner: macos-15
            runtime: macos-arm64
    steps:
      - name: Setup Node.js
        uses: actions/setup-node@53b83947a5a98c8d113130e565377fae1a50d02f # v6.3.0
        with:
          node-version: 22

      - name: Install published npm tarball with Rust execution poisoned
        shell: bash
        env:
          TAG: ${{ needs.prepare.outputs.tag }}
          NPM_ASSET: ${{ needs.prepare.outputs.npm_asset }}
        run: |
          set -euo pipefail
          trap_bin="$RUNNER_TEMP/no-rust-bin"
          compiler_marker="$RUNNER_TEMP/compiler-invoked"
          mkdir -p "$trap_bin"
          for compiler in cargo rustc; do
            cat > "$trap_bin/$compiler" <<'SH'
          #!/bin/sh
          printf compiler-invoked > "$RUNNER_TEMP/compiler-invoked"
          exit 97
          SH
            chmod 0755 "$trap_bin/$compiler"
          done
          export PATH="$trap_bin:$PATH"

          url="https://github.com/${GITHUB_REPOSITORY}/releases/download/${TAG}/${NPM_ASSET}"
          prefix="$RUNNER_TEMP/npm-prefix"
          npm install --global --prefix "$prefix" "$url"
          "$prefix/bin/codexflow" --version
          "$prefix/bin/codexflow-supervisor" --version
          export CODEX_HOME="$RUNNER_TEMP/codex-home"
          mkdir -p "$CODEX_HOME"
          "$prefix/bin/codexflow" doctor --json > "$RUNNER_TEMP/doctor.json"
          node - "$RUNNER_TEMP/doctor.json" <<'NODE'
          const fs = require("node:fs");
          const report = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
          if (!report.codex) {
            throw new Error("npm-installed codexflow cannot resolve its sibling codex binary");
          }
          if (!String(report.codex).includes("vendor")) {
            throw new Error(`codexflow resolved unexpected codex binary: ${report.codex}`);
          }
          NODE
          test ! -e "$compiler_marker"

  release-install-smoke-windows:
    needs:
      - prepare
      - package-and-release
    name: Published install ${{ matrix.runtime }}
    runs-on: ${{ matrix.runner }}
    timeout-minutes: 30
    strategy:
      fail-fast: false
      matrix:
        include:
          - runner: windows-latest
            runtime: windows-x64
          - runner: windows-11-arm
            runtime: windows-arm64
    steps:
      - name: Setup Node.js
        uses: actions/setup-node@53b83947a5a98c8d113130e565377fae1a50d02f # v6.3.0
        with:
          node-version: 22

      - name: Install published npm tarball with Rust execution poisoned
        shell: pwsh
        env:
          TAG: ${{ needs.prepare.outputs.tag }}
          NPM_ASSET: ${{ needs.prepare.outputs.npm_asset }}
        run: |
          $ErrorActionPreference = "Stop"
          $trapBin = "$env:RUNNER_TEMP\\no-rust-bin"
          $compilerMarker = "$env:RUNNER_TEMP\\compiler-invoked"
          New-Item -ItemType Directory -Force -Path $trapBin | Out-Null
          $trap = "@echo off`r`necho compiler-invoked>`"$compilerMarker`"`r`nexit /b 97`r`n"
          Set-Content -NoNewline -Path "$trapBin\\cargo.cmd" -Value $trap
          Set-Content -NoNewline -Path "$trapBin\\rustc.cmd" -Value $trap
          $env:PATH = "$trapBin;$env:PATH"

          $url = "https://github.com/$env:GITHUB_REPOSITORY/releases/download/$env:TAG/$env:NPM_ASSET"
          $prefix = "$env:RUNNER_TEMP\\npm-prefix"
          npm install --global --prefix $prefix $url
          if ($LASTEXITCODE -ne 0) { throw "npm install from GitHub Release failed" }
          & "$prefix\\codexflow.cmd" --version
          if ($LASTEXITCODE -ne 0) { throw "codexflow npm launcher failed" }
          & "$prefix\\codexflow-supervisor.cmd" --version
          if ($LASTEXITCODE -ne 0) { throw "codexflow-supervisor npm launcher failed" }
          $env:CODEX_HOME = "$env:RUNNER_TEMP\\codex-home"
          New-Item -ItemType Directory -Force -Path $env:CODEX_HOME | Out-Null
          $doctor = (& "$prefix\\codexflow.cmd" doctor --json | Out-String) | ConvertFrom-Json
          if (-not $doctor.codex) { throw "npm-installed codexflow cannot resolve its sibling codex binary" }
          if ([string]$doctor.codex -notmatch "vendor") { throw "codexflow resolved unexpected codex binary: $($doctor.codex)" }
          if (Test-Path $compilerMarker) { throw "npm installation executed a local Rust compiler" }
'''

    WORKFLOW.write_text(text)
    print("release workflow polish materialized")


if __name__ == "__main__":
    main()
