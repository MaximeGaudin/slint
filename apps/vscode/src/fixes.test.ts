import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import {
  diagnosticRangeForFinding,
  patchText,
  positionAtByteOffset,
  quickFixEditForFinding,
} from './fixes.js'
import type { FixableFinding } from './fixes.js'

function findingWithFix(overrides: Partial<FixableFinding> = {}): FixableFinding {
  return {
    rule: 'body/posix-paths',
    location: { line: 7, column: 12 },
    fix: {
      start: 96,
      end: 108,
      replacement: 'scripts/notes.md',
      description: 'Use forward slashes',
    },
    ...overrides,
  }
}

describe('positionAtByteOffset (#49)', () => {
  const text = 'hello\nworld'

  it('turns a byte offset into a line and character', () => {
    assert.deepEqual(positionAtByteOffset(text, 0), { line: 0, character: 0 })
    assert.deepEqual(positionAtByteOffset(text, 3), { line: 0, character: 3 })
    assert.deepEqual(positionAtByteOffset(text, 6), { line: 1, character: 0 })
    assert.deepEqual(positionAtByteOffset(text, 8), { line: 1, character: 2 })
  })

  it('counts multi-byte characters the way UTF-8 does, not the way UTF-16 does', () => {
    // 'h' is one byte, 'é' is two, 'llo' is three, '\n' is one — so 'w' sits at byte 7.
    const accented = 'héllo\nwörld'
    assert.deepEqual(positionAtByteOffset(accented, 7), { line: 1, character: 0 })

    // An emoji is four UTF-8 bytes but two UTF-16 units.
    const emoji = 'a😀\nb'
    assert.deepEqual(positionAtByteOffset(emoji, 5), { line: 0, character: 2 })
    assert.deepEqual(positionAtByteOffset(emoji, 6), { line: 1, character: 0 })
  })

  it('clamps instead of pointing past the document', () => {
    assert.deepEqual(positionAtByteOffset(text, 999), { line: 1, character: 5 })
    assert.deepEqual(positionAtByteOffset(text, -4), { line: 0, character: 0 })
  })

  it('lands before a multi-byte character when an offset falls inside it', () => {
    // Byte 2 is the second byte of 'é'; the only honest editor position is before it.
    assert.deepEqual(positionAtByteOffset('héllo', 2), { line: 0, character: 1 })
  })
})

describe('patchText (#49)', () => {
  it('splices one fix in', () => {
    const result = patchText('hello world', [
      { start: 6, end: 11, replacement: 'there', description: 'greet differently' },
    ])

    assert.equal(result.text, 'hello there')
    assert.equal(result.skipped, 0)
    assert.equal(result.applied.length, 1)
  })

  it('applies several fixes last-first so earlier offsets stay valid', () => {
    const result = patchText('hello world', [
      { start: 0, end: 5, replacement: 'goodbye', description: 'first' },
      { start: 6, end: 11, replacement: 'everyone', description: 'second' },
    ])

    assert.equal(result.text, 'goodbye everyone')
    assert.equal(result.applied.length, 2)
  })

  it('leaves a fix that overlaps one already applied for the next pass', () => {
    const result = patchText('hello world', [
      { start: 0, end: 11, replacement: 'entirely new text', description: 'whole line' },
      { start: 6, end: 11, replacement: 'everyone', description: 'tail' },
    ])

    assert.equal(result.text, 'entirely new text')
    assert.equal(result.applied.length, 1)
    assert.equal(result.skipped, 1)
  })

  it('defers a fix that points past the end of the document', () => {
    const result = patchText('short', [
      { start: 0, end: 99, replacement: 'nope', description: 'out of range' },
    ])

    assert.equal(result.text, 'short')
    assert.equal(result.applied.length, 0)
    assert.equal(result.skipped, 1)
  })

  it('skips the zero-width empty fix that means "set the executable bit"', () => {
    const result = patchText('hello world', [
      { start: 0, end: 0, replacement: '', description: 'chmod +x' },
    ])

    assert.equal(result.text, 'hello world')
    assert.equal(result.applied.length, 0)
  })

  it('keeps multi-byte text intact around a splice', () => {
    const result = patchText('voir émotion\net chemin', [
      { start: 14, end: 16, replacement: 'path', description: 'translate' },
    ])

    assert.equal(result.text, 'voir émotion\net path')
  })
})

describe('quickFixEditForFinding (#49)', () => {
  const document = [
    '---',
    'name: demo-fix-action',
    'description: Demo skill used only to surface a computed fix so the editor quick-fix menu can be inspected. Use when testing fix code actions.',
    '---',
    '',
    '# Demo fix action',
    '',
    'Read scripts\\notes.md and summarize it.',
    '',
  ].join('\n')

  it('titles the action after the fix description', () => {
    const finding = findingWithFix({
      fix: {
        start: document.indexOf('scripts\\notes.md'),
        end: document.indexOf('scripts\\notes.md') + 16,
        replacement: 'scripts/notes.md',
        description: 'Use forward slashes',
      },
    })

    const edit = quickFixEditForFinding(finding, document)

    assert.ok(edit, 'expected a quick fix for a fixable finding')
    assert.match(edit.title, /apply slint fix/i)
    assert.match(edit.title, /use forward slashes/i)
    assert.equal(edit.ruleId, 'body/posix-paths')
  })

  it('ranges the edit over the exact bytes the fix replaces', () => {
    const start = document.indexOf('scripts\\notes.md')
    const finding = findingWithFix({
      fix: { start, end: start + 16, replacement: 'scripts/notes.md', description: 'Use forward slashes' },
    })

    const edit = quickFixEditForFinding(finding, document)

    assert.ok(edit)
    assert.deepEqual(edit.range.start, { line: 7, character: 5 })
    assert.deepEqual(edit.range.end, { line: 7, character: 21 })
    assert.equal(edit.replacement, 'scripts/notes.md')
  })

  it('returns nothing for a finding without a computed fix', () => {
    const plain = findingWithFix({ fix: undefined })
    assert.equal(quickFixEditForFinding(plain, document), undefined)
  })
})

describe('diagnosticRangeForFinding (#49)', () => {
  it('converts the 1-based location an editor would show into a 0-based range', () => {
    const range = diagnosticRangeForFinding(findingWithFix({ location: { line: 7, column: 12 } }))

    assert.deepEqual(range.start, { line: 6, character: 11 })
    assert.deepEqual(range.end, { line: 6, character: 211 })
  })

  it('uses the end of the span when the rule reports one', () => {
    const range = diagnosticRangeForFinding(
      findingWithFix({ location: { line: 7, column: 12, end_line: 7, end_column: 28 } }),
    )

    assert.deepEqual(range.start, { line: 6, character: 11 })
    assert.deepEqual(range.end, { line: 6, character: 27 })
  })
})
