# slint docs

Astro + MDX documentation site for slint. Prose pages are Markdown you can edit; rule pages are
generated from `slint rules --json` — never hand-written.

```bash
pnpm install          # from repo root
pnpm sync:docs        # refresh src/data/rules.json from the CLI
pnpm dev:docs         # http://localhost:4321
pnpm build:docs
```

Or from this package:

```bash
pnpm sync && pnpm dev
```

## Layout

| Path | What it is |
| --- | --- |
| `src/pages/*.mdx` | Editable prose (home, config, plugins, editors, contributing) |
| `src/pages/rules/index.mdx` | Catalogue intro + `<RuleCatalogue />` |
| `src/pages/rules/[rule].astro` | One page per rule from `rules.json` |
| `src/components/` | Hero, catalogue, and other non-prose UI |
| `src/layouts/Page.astro` | Shared shell (nav, footer) — set via MDX `layout:` frontmatter |
| `src/data/rules.json` | Synced catalogue (`pnpm sync`) |

Set `SLINT_BIN` to use an installed binary instead of `cargo run`.
