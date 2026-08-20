#!/usr/bin/env bash
# Pre-flight checks that mirror the GitHub Actions CI workflow.
# Run this before pushing or cutting a release.
#
# Usage:
#   ./scripts/check.sh           # run all checks
#   ./scripts/check.sh --no-test # skip cargo test
#   ./scripts/check.sh --no-js   # skip pnpm lint (docs/vscode)
set -euo pipefail

export RUSTFLAGS="${RUSTFLAGS:--D warnings}"

RUN_TESTS=1
RUN_JS=1
for arg in "$@"; do
  case "$arg" in
    --no-test) RUN_TESTS=0 ;;
    --no-js) RUN_JS=0 ;;
    -h|--help)
      sed -n '2,10p' "$0"
      exit 0
      ;;
    *)
      echo "Unknown argument: $arg" >&2
      exit 2
      ;;
  esac
done

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "==> cargo fmt --all -- --check"
cargo fmt --all -- --check

echo "==> cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> cargo deny check"
if ! cargo deny --version >/dev/null 2>&1; then
  echo "cargo-deny is required (CI runs it). Install with: cargo install cargo-deny --locked" >&2
  exit 1
fi
cargo deny check

echo "==> check-no-todos"
node scripts/check-no-todos.mjs

if [ "$RUN_JS" -eq 1 ]; then
  if [ ! -d node_modules ]; then
    echo "==> pnpm install"
    pnpm install --frozen-lockfile
  fi
  echo "==> pnpm lint (biome + tsc + astro check)"
  # Turbo package lints already invoke check-no-todos; skip the root double-run.
  pnpm exec turbo run lint
else
  echo "==> pnpm lint (skipped via --no-js)"
fi

if [ "$RUN_TESTS" -eq 1 ]; then
  echo "==> cargo test --workspace"
  cargo test --workspace
else
  echo "==> cargo test (skipped via --no-test)"
fi

echo "==> All pre-flight checks passed"
