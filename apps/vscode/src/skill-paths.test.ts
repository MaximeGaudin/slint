import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { isUnderAnyRoot } from './skill-paths.js'

describe('isUnderAnyRoot (#44)', () => {
  const root = '/ws/skills/demo'

  it('matches the root itself (a deleted SKILL.md path)', () => {
    assert.equal(isUnderAnyRoot(root, [root]), true)
  })

  it('matches a file nested inside a deleted skill folder', () => {
    assert.equal(isUnderAnyRoot(`${root}/references/cli.md`, [root]), true)
    assert.equal(isUnderAnyRoot(`${root}/SKILL.md`, [root]), true)
  })

  it('matches the SKILL.md directly under the root', () => {
    assert.equal(isUnderAnyRoot(`${root}/SKILL.md`, [root]), true)
  })

  it('does not match a sibling that merely shares a prefix', () => {
    assert.equal(isUnderAnyRoot('/ws/skills/demo-other/SKILL.md', [root]), false)
  })

  it('does not match unrelated files', () => {
    assert.equal(isUnderAnyRoot('/ws/README.md', [root]), false)
  })

  it('matches any of several roots', () => {
    const roots = ['/ws/skills/a', '/ws/skills/b']
    assert.equal(isUnderAnyRoot('/ws/skills/b/SKILL.md', roots), true)
    assert.equal(isUnderAnyRoot('/ws/skills/a/sub/SKILL.md', roots), true)
    assert.equal(isUnderAnyRoot('/ws/skills/c/SKILL.md', roots), false)
  })

  it('empty roots never match', () => {
    assert.equal(isUnderAnyRoot('/ws/skills/demo/SKILL.md', []), false)
  })
})
