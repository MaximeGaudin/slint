import { execFileSync } from 'node:child_process'
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

/**
 * Pulls the rule catalogue out of the linter itself.
 *
 * The site never writes down what a rule does. It asks the binary, which is the only copy — a
 * documentation site that describes rules from memory is a site that is wrong the first time
 * someone edits a rationale and forgets it exists.
 *
 * The result is committed, so building the site needs Node and nothing else. Run this after
 * changing a rule.
 */

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..', '..', '..')
const out = join(here, '..', 'src', 'data', 'rules.json')

const binary = process.env.SLINT_BIN

const json = binary
  ? execFileSync(binary, ['rules', '--json'], { encoding: 'utf8' })
  : execFileSync('cargo', ['run', '--quiet', '--package', 'slint-cli', '--', 'rules', '--json'], {
      cwd: root,
      encoding: 'utf8',
      maxBuffer: 32 * 1024 * 1024,
    })

const rules = JSON.parse(json)

if (!Array.isArray(rules) || rules.length === 0) {
  throw new Error('the catalogue came back empty, which is never right')
}

for (const rule of rules) {
  if (!rule.reference_url?.startsWith('https://')) {
    throw new Error(`${rule.name} has no citation, and the site will not publish one that does not`)
  }
}

mkdirSync(dirname(out), { recursive: true })
writeFileSync(out, `${JSON.stringify(rules, null, 2)}\n`)

console.log(`Wrote ${rules.length} rules to ${out}`)
