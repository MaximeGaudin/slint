/**
 * Regenerates the `slint.rules` schema in package.json from the core rule catalogue.
 *
 * Run after adding or renaming a rule: `pnpm --filter slint-vscode build && node scripts/sync-rules.mjs`.
 * The settings-schema test keeps the manifest and the catalogue in step, so a forgotten
 * regeneration fails CI instead of shipping a rule without autocomplete.
 */
import { readFileSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
const vscodeDir = path.resolve(here, '..')
const require = createRequire(import.meta.url)
const { scanRuleCatalogue } = require(path.join(vscodeDir, 'out', 'rules-catalogue.js'))

const packagePath = path.join(vscodeDir, 'package.json')
const manifest = JSON.parse(readFileSync(packagePath, 'utf8'))
const configuration = manifest.contributes.configuration.properties
configuration['slint.rules'] ??= {}
const rules = configuration['slint.rules']

rules.scope = 'resource'
rules.type = 'object'
rules.default = {}
rules.additionalProperties = {
  type: 'string',
  enum: ['off', 'error', 'warning', 'info'],
  description: 'Severity for this rule, or off.',
}
rules.properties = {}

for (const rule of scanRuleCatalogue()) {
  rules.properties[rule.name] = {
    type: 'string',
    enum: ['off', 'error', 'warning', 'info'],
    default: rule.defaultSeverity,
    description: `${rule.summary} (default: ${rule.defaultSeverity}).`,
  }
}

writeFileSync(packagePath, `${JSON.stringify(manifest, null, 2)}\n`)
console.log(`slint.rules: ${Object.keys(rules.properties).length} rules from the core catalogue`)
