#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "${repo_root}"

mode="${1:-}"
shift || true

case "${mode}" in
  fmt-check)
    cargo fmt --all --check
    ;;
  clippy)
    cargo clippy --workspace --all-targets -- -D warnings
    ;;
  *)
    printf 'post-tool-use-rust.sh: unknown mode %s\n' "${mode}" >&2
    exit 64
    ;;
esac
