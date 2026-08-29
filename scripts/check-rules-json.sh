#!/usr/bin/env bash
# Fail when apps/docs/src/data/rules.json no longer matches the rule catalogue
# of the current binary. This is the CI gate behind the README promise that the
# docs site and the binary can never disagree about what a rule does.
#
# Delegates binary resolution to apps/docs/scripts/sync-rules.mjs: SLINT_BIN
# wins, otherwise it falls back to `cargo run --package slint-cli`. The check
# itself never hardcodes a rule count, so it stays valid as rules come and go.
#
# Usage:
#   ./scripts/check-rules-json.sh                       # cargo run fallback
#   SLINT_BIN=/path/to/slint-cli ./scripts/check-rules-json.sh
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

RULES_JSON="apps/docs/src/data/rules.json"

node apps/docs/scripts/sync-rules.mjs

if ! git diff --exit-code -- "$RULES_JSON"; then
  git checkout -- "$RULES_JSON"
  echo "error: $RULES_JSON is out of sync with the binary's rule catalogue." >&2
  echo "       Run 'pnpm sync:docs' and commit the refreshed file." >&2
  exit 1
fi

echo "==> $RULES_JSON matches the binary catalogue"