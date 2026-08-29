#!/usr/bin/env node
/**
 * Advisory ignores in deny.toml must be time-boxed.
 *
 * Every `{ id = "RUSTSEC-…" }` entry in [advisories].ignore needs an
 * `# expires: YYYY-MM-DD` line in the comment block directly above it, and
 * that date must be in the future. When the date passes, CI fails, forcing a
 * re-evaluation of the advisory instead of a silent indefinite exemption.
 */

import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(fileURLToPath(new URL('.', import.meta.url)), '..')
const target = process.argv[2] ? join(root, process.argv[2]) : join(root, 'deny.toml')

const lines = readFileSync(target, 'utf8').split(/\r?\n/)

const pad = (n) => String(n).padStart(2, '0')
const now = new Date()
const today = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`

let fails = 0
for (let i = 0; i < lines.length; i++) {
  const entry = lines[i].match(/^\s*\{\s*id\s*=\s*"(RUSTSEC-[^"]+)"/)
  if (!entry) continue
  const id = entry[1]

  let expires = null
  for (let j = i - 1; j >= 0; j--) {
    if (!lines[j].trimStart().startsWith('#')) break
    const m = lines[j].match(/expires:\s*(\d{4}-\d{2}-\d{2})\b/)
    if (m) expires = m[1]
  }

  if (!expires) {
    console.error(
      `deny.toml:${i + 1}: advisory ${id} is ignored without a time-box — add "# expires: YYYY-MM-DD" to its comment block`,
    )
    fails++
  } else if (expires < today) {
    console.error(
      `deny.toml:${i + 1}: ignore for ${id} expired on ${expires} — re-evaluate the advisory and remove or renew the ignore`,
    )
    fails++
  }
}

if (fails > 0) process.exit(1)
console.log('check-ignore-deadlines: clean')
