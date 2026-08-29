import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { mayRunBinary } from './trust.js'

describe('mayRunBinary (#136)', () => {
  it('runs the binary in a trusted workspace', () => {
    assert.equal(mayRunBinary(true), true)
  })

  it('never runs the binary in an untrusted workspace', () => {
    assert.equal(
      mayRunBinary(false),
      false,
      'an external process steered by workspace settings must not run untrusted',
    )
  })
})
