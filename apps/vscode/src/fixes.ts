/**
 * Pure helpers for computed-fix code actions.
 *
 * Kept free of the vscode module so unit tests can run under plain Node, and so the byte math —
 * the part that corrupts files when it is wrong — can be tested without an editor.
 *
 * A fix is a UTF-8 byte range and a replacement (see `packages/core/src/diagnostics.rs`), and the
 * JSON envelope hands those bytes straight through. The editor, however, wants line/character in
 * UTF-16 code units, so every range that reaches VS Code is re-resolved from the document text.
 */

export type Point = { line: number; character: number }

/** A computed fix as the CLI's JSON envelope carries it. */
export type FixData = {
  /** Byte offset into the file's text, inclusive. */
  start: number
  /** Byte offset into the file's text, exclusive. */
  end: number
  replacement: string
  description: string
}

/** The slice of a finding a quick fix needs. `Finding` in extension.ts satisfies this. */
export type FixableFinding = {
  rule: string
  location: { line: number; column: number; end_line?: number; end_column?: number }
  fix?: FixData
}

/** UTF-8 length in bytes of one code point. */
function bytesFor(code: number): number {
  if (code < 0x80) return 1
  if (code < 0x800) return 2
  if (code < 0x10000) return 3
  return 4
}

/**
 * The editor position of a UTF-8 byte offset.
 *
 * Offsets inside a multi-byte character land before it — there is no honest position inside a
 * character — and offsets past the end of the text clamp to the end.
 */
export function positionAtByteOffset(text: string, byteOffset: number): Point {
  let line = 0
  let character = 0
  let byte = 0
  let index = 0

  while (index < text.length && byte < byteOffset) {
    const code = text.codePointAt(index) ?? 0
    const width = code > 0xffff ? 2 : 1
    const bytes = bytesFor(code)

    if (byte + bytes > byteOffset) break

    if (code === 0x0a) {
      line += 1
      character = 0
    } else {
      character += width
    }

    byte += bytes
    index += width
  }

  return { line, character }
}

/**
 * The JS string index of a UTF-8 byte offset, or -1 when the offset is not on a character
 * boundary (a splice there would produce invalid text).
 */
function stringIndexAtByteOffset(text: string, byteOffset: number): number {
  let byte = 0
  let index = 0

  while (byte < byteOffset) {
    if (index >= text.length) return -1

    const code = text.codePointAt(index) ?? 0
    const bytes = bytesFor(code)
    if (byte + bytes > byteOffset) return -1

    byte += bytes
    index += code > 0xffff ? 2 : 1
  }

  return index
}

/**
 * Splices fixes into text, last one first — the same rules the CLI's fixer lives by
 * (`packages/core/src/fix.rs`), because an edit that overlaps one already applied was computed
 * against text that has since moved, and applying it would corrupt the file.
 *
 * Fixes are byte ranges, so every splice is resolved through the text it lands in: bytes below
 * the range already applied are untouched, which is exactly why the CLI applies last first.
 */
export function patchText(
  text: string,
  fixes: FixData[],
): { text: string; applied: FixData[]; skipped: number } {
  // The zero-width empty fix is the CLI's way of saying "set the executable bit" — nothing to
  // splice, and an editor cannot chmod a file by editing its text.
  const ordered = fixes
    .filter((fix) => !(fix.start === 0 && fix.end === 0 && fix.replacement === ''))
    .sort((first, second) => second.start - first.start)

  let patched = text
  let lowestApplied = Number.MAX_SAFE_INTEGER
  let skipped = 0
  const applied: FixData[] = []

  for (const fix of ordered) {
    if (
      fix.end > Buffer.byteLength(patched, 'utf8') ||
      fix.start > fix.end ||
      fix.end > lowestApplied
    ) {
      skipped += 1
      continue
    }

    const start = stringIndexAtByteOffset(patched, fix.start)
    const end = stringIndexAtByteOffset(patched, fix.end)
    if (start === -1 || end === -1) {
      skipped += 1
      continue
    }

    patched = patched.slice(0, start) + fix.replacement + patched.slice(end)
    lowestApplied = fix.start
    applied.push(fix)
  }

  return { text: patched, applied, skipped }
}

/**
 * The 0-based diagnostic range a finding would draw — the same math `toDiagnostic` uses, so a
 * quick fix can match a diagnostic in the Problems panel to the finding that produced it.
 */
export function diagnosticRangeForFinding(finding: FixableFinding): { start: Point; end: Point } {
  const line = Math.max(0, finding.location.line - 1)
  const column = Math.max(0, finding.location.column - 1)
  const endLine = Math.max(0, (finding.location.end_line ?? finding.location.line) - 1)
  const endColumn = Math.max(0, (finding.location.end_column ?? finding.location.column + 200) - 1)

  return { start: { line, character: column }, end: { line: endLine, character: endColumn } }
}

/**
 * The quick fix for one fixable finding: replace exactly the bytes the fix names.
 * Returns nothing for a finding without a computed fix.
 */
export function quickFixEditForFinding(
  finding: FixableFinding,
  documentText: string,
):
  | { title: string; ruleId: string; range: { start: Point; end: Point }; replacement: string }
  | undefined {
  if (!finding.fix) return undefined

  return {
    title: `Apply slint fix: ${finding.fix.description}`,
    ruleId: finding.rule,
    range: {
      start: positionAtByteOffset(documentText, finding.fix.start),
      end: positionAtByteOffset(documentText, finding.fix.end),
    },
    replacement: finding.fix.replacement,
  }
}
