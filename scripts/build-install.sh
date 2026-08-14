#!/usr/bin/env bash
# Build and install the release `slint` binary onto PATH (~/.cargo/bin).
#
# Usage:
#   ./scripts/build-install.sh               # pre-flight checks, then cargo install
#   ./scripts/build-install.sh --skip-checks # skip pre-flight checks
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

SKIP_CHECKS=0
for arg in "$@"; do
  case "$arg" in
    --skip-checks) SKIP_CHECKS=1 ;;
    -h|--help)
      sed -n '2,8p' "$0"
      exit 0
      ;;
    *)
      echo "Unknown argument: $arg" >&2
      exit 2
      ;;
  esac
done

if [ "$SKIP_CHECKS" -eq 0 ]; then
  echo "==> Running pre-flight checks…"
  "$SCRIPT_DIR/check.sh"
else
  echo "==> Skipping pre-flight checks (--skip-checks)"
fi

echo "==> Installing slint into ~/.cargo/bin (cargo install --path apps/cli --force)"
cargo install --path apps/cli --force

echo "==> Verifying"
slint --version
echo "==> Installed. Ensure ~/.cargo/bin is on your PATH."
