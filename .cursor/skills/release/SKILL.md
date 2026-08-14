---
name: release
description: Prepare, tag, and publish a new slint release — bump version, update CHANGELOG.md, build/install via project script, commit, create annotated tag, and optionally push + trigger CI release workflow when requested. Use when the user asks to release, tag, cut a release, bump version, update the changelog, or publish.
---

# Release

Step-by-step procedure for cutting a new slint release.

## 1. Determine the new version

Read the current version:

```bash
grep '^version' Cargo.toml   # workspace version
git tag --list --sort=-v:refname | head -5
```

Also keep `apps/vscode/package.json` and `apps/docs/package.json` / `apps/cli/package.json` versions in sync when they track the product release.

Choose the next version following [Semantic Versioning](https://semver.org):

| Change type | Bump |
|---|---|
| Breaking CLI / rule behaviour changes | Major (X.0.0) |
| New rules, commands, features | Minor (0.X.0) |
| Bug fixes, refactors, dependency updates | Patch (0.0.X) |

## 2. Gather changes since last tag

```bash
git log <LAST_TAG>..HEAD --oneline --reverse
# First release: git log --oneline --reverse
```

Categorize every commit into **Added**, **Changed**, **Fixed**, **Removed** per [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## 3. Update CHANGELOG.md

Insert a new section **above** the previous release (and fold any `## [Unreleased]` entries into it):

```markdown
## [X.Y.Z] - YYYY-MM-DD

### Added
- ...

### Changed
- ...

### Fixed
- ...
```

Rules:
- User-facing bullets only; past-tense section headers.
- Skip merge commits, CI-only noise, and pure formatting.
- Leave an empty `## [Unreleased]` heading at the top afterwards.

## 4. Bump versions

Edit root `Cargo.toml`:

```toml
[workspace.package]
version = "X.Y.Z"
```

Crates inherit `version.workspace = true`. Also bump `apps/vscode/package.json`, `apps/cli/package.json`, and `apps/docs/package.json` when shipping a product release.

## 5. Pre-flight checks (mirror CI)

```bash
./scripts/check.sh
```

This mirrors `.github/workflows/ci.yml` (fmt, clippy, todo check, pnpm lint, tests).

> `./scripts/build-install.sh` already calls `check.sh` unless you pass `--skip-checks`.

## 6. Build and install

```bash
./scripts/build-install.sh
slint --version   # confirm new version
```

Always use the project install script (or `pnpm install:cli`), not a manual `cp`.

## 7. Commit and tag

```bash
git add -A
git commit -m "chore: release vX.Y.Z"
git tag -a X.Y.Z -m "Release X.Y.Z"
```

Use an **annotated** tag (`-a`), not a lightweight tag.

## 8. Verify

```bash
git log --oneline -1
git tag -l "X.Y.Z" -n1
slint --version
```

## 9. Publish (only when explicitly requested)

```bash
git push origin HEAD
git push origin X.Y.Z
gh workflow run release.yml -f tag=X.Y.Z
gh run watch $(gh run list --workflow=release.yml --limit=1 --json databaseId --jq '.[0].databaseId')
```

The CI workflow builds cross-platform binaries, attaches changelog notes, and optionally updates the Homebrew tap when `HOMEBREW_TAP_DEPLOY_KEY` is set.

## Checklist

```
Release X.Y.Z:
- [ ] Determine version number
- [ ] Gather commits since last tag
- [ ] Update CHANGELOG.md
- [ ] Bump Cargo.toml (+ JS package versions if shipping together)
- [ ] Pre-flight: ./scripts/check.sh
- [ ] Build and install locally (./scripts/build-install.sh)
- [ ] slint --version shows new version
- [ ] git commit
- [ ] git tag -a X.Y.Z
- [ ] Verify tag
- [ ] (If requested) Push commit and tag
- [ ] (If requested) Trigger CI release workflow
```

## Notes

- Do NOT push commits/tags or trigger the CI release unless the user explicitly asks.
- Cross-platform binaries come from CI (`release.yml`), not from a local build.
