#!/usr/bin/env bash
set -euo pipefail

mode="${1:---prepare}"
case "$mode" in
  --prepare|--check) ;;
  *)
    echo "usage: $0 [--prepare|--check]" >&2
    exit 2
    ;;
esac

repo_root="$(git rev-parse --show-toplevel)"
workspace="$repo_root/codex-rs"
cd "$workspace"

printf 'Rust:  '
rustc --version
printf 'Cargo: '
cargo --version

host="$(rustc -vV | sed -n 's/^host: //p')"
case "$host" in
  x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu) ;;
  *)
    echo "unsupported Codespaces Rust host target: $host" >&2
    exit 1
    ;;
esac

version="$(python3 "$repo_root/.github/scripts/rusty_v8_bazel.py" resolved-v8-crate-version)"
release_tag="rusty-v8-v${version}"
base_url="https://github.com/openai/codex/releases/download/${release_tag}"
profile="ptrcomp_sandbox_release"
cache_dir="$HOME/.cache/codexflow/rusty_v8/${version}/${host}"
mkdir -p "$cache_dir"

archive_name="librusty_v8_${profile}_${host}.a.gz"
binding_name="src_binding_${profile}_${host}.rs"
checksums_name="rusty_v8_${profile}_${host}.sha256"
archive_path="$cache_dir/$archive_name"
binding_path="$cache_dir/$binding_name"
checksums_path="$cache_dir/$checksums_name"

download_if_missing() {
  local name="$1"
  local path="$2"
  if [[ ! -s "$path" ]]; then
    echo "Downloading $name"
    curl -fsSL "$base_url/$name" -o "$path"
  fi
}

download_if_missing "$archive_name" "$archive_path"
download_if_missing "$binding_name" "$binding_path"
download_if_missing "$checksums_name" "$checksums_path"

if [[ "$(wc -l < "$checksums_path")" -ne 2 ]]; then
  echo "expected exactly two rusty_v8 checksums for $host" >&2
  exit 1
fi
(
  cd "$cache_dir"
  tr -d '\r' < "$checksums_path" | sha256sum -c -
)

env_file="$HOME/.codexflow-rust-env"
{
  printf 'export RUSTY_V8_ARCHIVE=%q\n' "$archive_path"
  printf 'export RUSTY_V8_SRC_BINDING_PATH=%q\n' "$binding_path"
  printf 'export CODEXFLOW_CLOUD_RUST_TARGET=%q\n' "$host"
} > "$env_file"

source_line='source "$HOME/.codexflow-rust-env"'
touch "$HOME/.bashrc"
if ! grep -Fqx "$source_line" "$HOME/.bashrc"; then
  printf '\n%s\n' "$source_line" >> "$HOME/.bashrc"
fi
# shellcheck disable=SC1090
source "$env_file"

cargo fetch --locked

echo "Codespaces Rust environment prepared for $host"
echo "Environment file: $env_file"

if [[ "$mode" == "--check" ]]; then
  cargo check -p codex-cli --bin codexflow --bin codexflow-supervisor
fi
