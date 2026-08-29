/**
 * Pure path matching for the diagnostics lifecycle (publish, delete, rename).
 *
 * Kept free of the vscode module so unit tests can run under plain Node.
 */

import * as path from 'node:path'

/** The only file slint lints. */
export const SKILL_FILE = 'SKILL.md'

/**
 * True when `file` is one of `roots`, sits inside one, or is the SKILL.md of one.
 *
 * Mirrors how the CLI scopes a run: the target is a skill directory, findings live in that
 * directory's SKILL.md (or files it references). The prefix check is boundary-aware, so
 * `/ws/skills/demo` does not match `/ws/skills/demo-other`.
 */
export function isUnderAnyRoot(file: string, roots: string[]): boolean {
  return roots.some((root) => {
    if (file === root) return true
    if (file.startsWith(root + path.sep)) return true
    return file === path.join(root, SKILL_FILE)
  })
}
