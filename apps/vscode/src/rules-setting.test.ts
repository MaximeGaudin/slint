import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { RULE_SEVERITIES, ruleOverridesArgv } from './rules-setting.js'

describe('ruleOverridesArgv (#63)', () => {
  it('maps every entry to a repeatable --rule flag the CLI already parses', () => {
    assert.deepEqual(
      ruleOverridesArgv({ 'name/not-generic': 'error', 'body/posix-paths': 'off' }),
      ['--rule', 'body/posix-paths=off', '--rule', 'name/not-generic=error'],
    )
  })

  it('is deterministic no matter how the settings object was written', () => {
    const first = ruleOverridesArgv({ 'body/posix-paths': 'off', 'name/not-generic': 'error' })
    const second = ruleOverridesArgv({ 'name/not-generic': 'error', 'body/posix-paths': 'off' })

    assert.deepEqual(first, second)
  })

  it('says nothing when there are no overrides', () => {
    assert.deepEqual(ruleOverridesArgv(undefined), [])
    assert.deepEqual(ruleOverridesArgv({}), [])
  })

  it('skips values the severity enum would never produce', () => {
    assert.deepEqual(ruleOverridesArgv({ 'body/posix-paths': 'bogus' }), [])
  })

  it('skips keys that are not rule ids, so a typo cannot reach the CLI', () => {
    assert.deepEqual(ruleOverridesArgv({ 'posix paths': 'off' }), [])
    assert.deepEqual(ruleOverridesArgv({ '': 'off' }), [])
  })

  it('accepts every severity the setting offers', () => {
    const argv = ruleOverridesArgv({
      'a/one': 'off',
      'a/two': 'error',
      'a/three': 'warning',
      'a/four': 'info',
    })

    assert.deepEqual(argv, [
      '--rule',
      'a/four=info',
      '--rule',
      'a/one=off',
      '--rule',
      'a/three=warning',
      '--rule',
      'a/two=error',
    ])
  })

  it('exposes exactly the severities the CLI understands', () => {
    assert.deepEqual([...RULE_SEVERITIES], ['off', 'error', 'warning', 'info'])
  })
})
