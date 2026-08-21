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

export function disableNextLineComment(ruleId: string): string {
  return `<!-- slint-disable-next-line ${ruleId} -->\n`
}

export function disableFileComment(ruleId: string): string {
  return `<!-- slint-disable ${ruleId} -->\n`
}

/**
 * 0-based line where a disable-next-line comment should be inserted so that
 * `findingLine` (0-based) is covered — before a fence opener when the finding
 * sits inside a fenced block, otherwise on the finding line itself.
 */
export function insertAtLineForDisableNext(lines: string[], findingLine: number): number {
  let openAt: number | undefined
  let openMarker: string | undefined

  for (let i = 0; i <= findingLine; i++) {
    const trimmed = (lines[i] ?? '').trimStart()
    if (openAt === undefined) {
      if (trimmed.startsWith('```')) {
        openAt = i
        openMarker = '```'
      } else if (trimmed.startsWith('~~~')) {
        openAt = i
        openMarker = '~~~'
      }
      continue
    }

    if (openMarker && trimmed.startsWith(openMarker)) {
      if (i < findingLine) {
        openAt = undefined
        openMarker = undefined
      }
    }
  }

  return openAt ?? findingLine
}

/** 0-based line after YAML frontmatter, or 0 when there is none. */
export function insertAtLineForFileDisable(lines: string[]): number {
  if ((lines[0] ?? '').trim() !== '---') return 0

  for (let i = 1; i < lines.length; i++) {
    if ((lines[i] ?? '').trim() === '---') return i + 1
  }

  return 0
}

/** Builds the Ignore quick-fix edits for one finding. */
export function ignoreEditsForFinding(options: {
  ruleId: string
  findingLine: number
  documentText: string
}): IgnoreEdit[] {
  const { ruleId, findingLine, documentText } = options
  const lines = documentText.split(/\r?\n/)

  return [
    {
      kind: 'next-line',
      title: `Ignore '${ruleId}' for this line`,
      insertAtLine: insertAtLineForDisableNext(lines, findingLine),
      text: disableNextLineComment(ruleId),
    },
    {
      kind: 'file',
      title: `Ignore '${ruleId}' for this file`,
      insertAtLine: insertAtLineForFileDisable(lines),
      text: disableFileComment(ruleId),
    },
  ]
}
