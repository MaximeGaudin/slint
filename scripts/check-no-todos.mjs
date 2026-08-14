#!/usr/bin/env node
/**
 * Fails when TODO / FIXME / XXX / HACK / STUB appear in comments.
 *
 * String literals are ignored on purpose: tests and docs fixtures for the
 * house/no-todo plugin example must be allowed to contain the word TODO.
 *
 * Stub macros (todo!, unimplemented!) are handled separately by Clippy.
 */

import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative, extname } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(fileURLToPath(new URL('.', import.meta.url)), '..')

const roots = process.argv.slice(2).length
  ? process.argv.slice(2).map((p) => (p.startsWith('/') ? p : join(root, p)))
  : [
      join(root, 'apps/cli/src'),
      join(root, 'apps/vscode/src'),
      join(root, 'apps/docs/src'),
      join(root, 'apps/docs/scripts'),
      join(root, 'apps/docs/public'),
      join(root, 'packages/core/src'),
    ]

const EXTS = new Set(['.rs', '.ts', '.tsx', '.js', '.mjs', '.astro', '.css'])

/** Markers that mean "unfinished work left in the tree" (uppercase only). */
const MARKER = /\b(TODO|FIXME|XXX|HACK|STUB)\b/

/**
 * Strip string / char / raw-string literals so fixture text like `pattern = "TODO"`
 * does not trip the check — only comments should.
 */
function stripLiterals(source, ext) {
  if (ext === '.rs') {
    return source
      .replace(/r#+".*?"#+/gs, '""')
      .replace(/b?"(?:\\.|[^"\\])*"/g, '""')
      .replace(/b?'(?:\\.|[^'\\])*'/g, "''")
  }
  return source
    .replace(/`(?:\\.|[^`\\])*`/g, '""')
    .replace(/"(?:\\.|[^"\\])*"/g, '""')
    .replace(/'(?:\\.|[^'\\])*'/g, "''")
}

function isCommentLine(line, ext) {
  const trimmed = line.trim()
  if (trimmed.startsWith('//')) return true
  if (trimmed.startsWith('///')) return true
  if (trimmed.startsWith('//!')) return true
  if (trimmed.startsWith('/*') || trimmed.startsWith('*') || trimmed.startsWith('*/')) return true
  // Trailing line comment: code; // TODO
  if (ext === '.rs' || ext === '.ts' || ext === '.tsx' || ext === '.js' || ext === '.mjs' || ext === '.astro') {
    // Rough: a // not inside what remains after literal strip
    const idx = line.indexOf('//')
    if (idx >= 0) return true
  }
  if (ext === '.css' && (trimmed.startsWith('/*') || line.includes('/*'))) return true
  return false
}

function walk(dir, files = []) {
  let entries
  try {
    entries = readdirSync(dir)
  } catch {
    return files
  }
  for (const name of entries) {
    if (name === 'node_modules' || name === 'dist' || name === 'out' || name === 'target') continue
    const path = join(dir, name)
    const st = statSync(path)
    if (st.isDirectory()) walk(path, files)
    else if (EXTS.has(extname(name))) files.push(path)
  }
  return files
}

const hits = []

for (const dir of roots) {
  for (const file of walk(dir)) {
    const ext = extname(file)
    const raw = readFileSync(file, 'utf8')
    const stripped = stripLiterals(raw, ext)
    const lines = stripped.split(/\r?\n/)
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i]
      if (!MARKER.test(line)) continue
      if (!isCommentLine(line, ext) && !line.includes('/*')) continue
      // Require the marker to sit in a comment portion of the line
      const commentStart = Math.max(line.indexOf('//'), line.indexOf('/*'), line.trimStart().startsWith('*') ? 0 : -1)
      if (commentStart < 0 && !line.trim().startsWith('*')) continue
      const comment = commentStart >= 0 ? line.slice(commentStart) : line
      if (!MARKER.test(comment)) continue
      hits.push(`${relative(root, file)}:${i + 1}: ${raw.split(/\r?\n/)[i].trim()}`)
    }
  }
}

if (hits.length > 0) {
  console.error('Forbidden unfinished-work markers in comments (TODO / FIXME / XXX / HACK / STUB):\n')
  for (const hit of hits) console.error(`  ${hit}`)
  console.error('\nFinish the work, or remove the marker. Stub macros are denied by Clippy separately.')
  process.exit(1)
}

console.log('check-no-todos: clean')
