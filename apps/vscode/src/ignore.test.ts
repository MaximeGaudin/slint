import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { ignoreEditsForFinding, ruleIdFromCode } from './ignore.js'

describe('ruleIdFromCode', () => {
  it('reads a string code', () => {
    assert.equal(ruleIdFromCode('body/posix-paths'), 'body/posix-paths')
  })

  it('reads Diagnostic.code objects used by the extension', () => {
    assert.equal(ruleIdFromCode({ value: 'body/posix-paths' }), 'body/posix-paths')
  })

  it('returns undefined when code is missing', () => {
    assert.equal(ruleIdFromCode(undefined), undefined)
  })
})

describe('ignoreEditsForFinding (#16)', () => {
  const document = [
    '---',
    'name: demo-ignore-action',
    'description: Demo skill used only to surface a slint finding so the editor quick-fix menu can be inspected. Use when testing Ignore code actions.',
    '---',
    '',
    '# Demo ignore action',
    '',
    'Read scripts\\notes.md and summarize it.',
    '',
  ].join('\n')

  it('offers Ignore for this line with disable-next-line above the finding', () => {
    const findingLine = 7 // 0-based: the scripts\notes.md line
    const edits = ignoreEditsForFinding({
      ruleId: 'body/posix-paths',
      findingLine,
      documentText: document,
    })

    const nextLine = edits.find((edit) => edit.kind === 'next-line')
    assert.ok(nextLine, 'expected an Ignore-this-line quick fix')
    assert.equal(nextLine.insertAtLine, findingLine)
    assert.equal(nextLine.text, '<!-- slint-disable-next-line body/posix-paths -->\n')
    assert.match(nextLine.title, /ignore/i)
    assert.match(nextLine.title, /body\/posix-paths/)
  })

  it('offers Ignore for this file after the frontmatter', () => {
    const edits = ignoreEditsForFinding({
      ruleId: 'body/posix-paths',
      findingLine: 7,
      documentText: document,
    })

    const file = edits.find((edit) => edit.kind === 'file')
    assert.ok(file, 'expected an Ignore-for-this-file quick fix')
    assert.equal(file.insertAtLine, 4) // first line after closing ---
    assert.equal(file.text, '<!-- slint-disable body/posix-paths -->\n')
    assert.match(file.title, /ignore/i)
    assert.match(file.title, /file/i)
  })

  it('places disable-next-line before a fence when the finding is inside it', () => {
    const fenced = [
      '---',
      'name: demo-fence',
      'description: Demo skill with an example path inside a fence. Use when testing fence-scoped Ignore.',
      '---',
      '',
      '```bash',
      'scripts/run.sh --help',
      '```',
      '',
    ].join('\n')

    const edits = ignoreEditsForFinding({
      ruleId: 'bundle/no-dangling-path',
      findingLine: 6, // scripts/run.sh inside the fence
      documentText: fenced,
    })

    const nextLine = edits.find((edit) => edit.kind === 'next-line')
    assert.ok(nextLine, 'expected an Ignore-this-line quick fix')
    assert.equal(nextLine.insertAtLine, 5) // the ```bash opener
    assert.equal(nextLine.text, '<!-- slint-disable-next-line bundle/no-dangling-path -->\n')
  })
})
