import { readdirSync, readFileSync } from 'node:fs'
import path from 'node:path'

/**
 * Reads the rule catalogue straight out of the core Rust sources.
 *
 * The manifest's `slint.rules` autocomplete is generated from this, and a test keeps the two in
 * step — a new rule that forgets the manifest fails the build instead of shipping without
 * autocomplete.
 */

export type RuleCatalogueEntry = {
  name: string
  summary: string
  defaultSeverity: 'error' | 'warning' | 'info'
}

const CORE_SRC = path.resolve(__dirname, '../../../packages/core/src')

const BLOCK = /RuleMeta\s*\{([^}]*)\}/g

function entriesFrom(text: string): RuleCatalogueEntry[] {
  const entries: RuleCatalogueEntry[] = []

  for (const [, body] of text.matchAll(BLOCK)) {
    const name = body.match(/(?:^|[\s{])name:\s*"([^"]+)"/)?.[1]
    if (!name) continue

    const summary = body.match(/summary:\s*"([^"]+)"/)?.[1] ?? ''
    const severity = body.match(/default_severity:\s*Severity::(\w+)/)?.[1] ?? 'Warning'

    entries.push({
      name,
      summary,
      defaultSeverity: severity.toLowerCase() as RuleCatalogueEntry['defaultSeverity'],
    })
  }

  return entries
}

/** Every rule the CLI knows about, sorted by id. */
export function scanRuleCatalogue(): RuleCatalogueEntry[] {
  const files = [
    ...readdirSync(path.join(CORE_SRC, 'rules'), { withFileTypes: true })
      .filter((entry) => entry.isFile() && entry.name.endsWith('.rs'))
      .map((entry) => path.join(CORE_SRC, 'rules', entry.name)),
    path.join(CORE_SRC, 'llm', 'rules.rs'),
  ]

  return files
    .flatMap((file) => entriesFrom(readFileSync(file, 'utf8')))
    .sort((first, second) => first.name.localeCompare(second.name))
}
