/**
 * Pure helpers for Ignore quick fixes (slint-disable comments).
 *
 * Kept free of the vscode module so unit tests can run under plain Node.
 */

export type IgnoreKind = 'next-line' | 'file'

export type IgnoreEdit = {
  kind: IgnoreKind
  /** Label shown in the Quick Fix menu. */
  title: string
  /** 0-based line index; the comment is inserted at column 0 of this line. */
  insertAtLine: number
  /** Full line text including trailing newline. */
  text: string
}

/** Pull the rule id out of a Diagnostic.code value (string or { value }). */
export function ruleIdFromCode(
  code: string | number | { value: string | number } | undefined,
): string | undefined {
  if (code === undefined || code === null) return undefined
  if (typeof code === 'string') return code || undefined
  if (typeof code === 'number') return String(code)
  if (typeof code === 'object' && 'value' in code) {
    const value = code.value
    if (typeof value === 'string') return value || undefined
    if (typeof value === 'number') return String(value)
  }
  return undefined
}

/**
 * Builds the Ignore quick-fix edits for one finding.
 *
 * Stubbed empty until the Ignore actions are implemented (#16).
 */
export function ignoreEditsForFinding(_options: {
  ruleId: string
  findingLine: number
  documentText: string
}): IgnoreEdit[] {
  return []
}
