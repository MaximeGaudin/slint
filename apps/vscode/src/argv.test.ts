import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import * as path from 'node:path'
import { describe, it } from 'node:test'
import { stripReservedLlmArgs } from './argv.js'

const manifest = JSON.parse(readFileSync(path.join(__dirname, '..', 'package.json'), 'utf8')) as {
  contributes: { configuration: { properties: Record<string, { scope?: string }> } }
}

describe('package.json manifest (#33)', () => {
  it('scopes slint.llm.apiKey to machine so a workspace cannot set the credential', () => {
    assert.equal(
      manifest.contributes.configuration.properties['slint.llm.apiKey'].scope,
      'machine',
      'an API key settable from a workspace settings.json can be committed to a repo',
    )
  })
})

describe('stripReservedLlmArgs (#33)', () => {
  it('drops --llm-base-url and its value so a workspace cannot redirect the model pass', () => {
    const kept = stripReservedLlmArgs([
      '--llm-base-url',
      'https://attacker.example.invalid/v1',
      '--rule',
      'body/posix-paths=off',
    ])
    assert.deepEqual(kept, ['--rule', 'body/posix-paths=off'])
  })

  it('drops the --flag=value form without eating the following token', () => {
    const kept = stripReservedLlmArgs([
      '--llm-base-url=https://attacker.example.invalid/v1',
      '--rule',
      'body/not-empty=off',
    ])
    assert.deepEqual(kept, ['--rule', 'body/not-empty=off'])
  })

  it('drops every flag that steers the model pass, value tokens included', () => {
    const kept = stripReservedLlmArgs([
      '--llm',
      '--no-llm',
      '--llm-provider',
      'groq',
      '--llm-model',
      'llama-3.1-8b-instant',
      '--llm-api-key-env',
      'SOME_VAR',
    ])
    assert.deepEqual(kept, [])
  })

  it('keeps rule overrides and unrelated arguments untouched', () => {
    const kept = stripReservedLlmArgs(['--rule', 'body/not-empty=off', '--max-warnings', '5', '-q'])
    assert.deepEqual(kept, ['--rule', 'body/not-empty=off', '--max-warnings', '5', '-q'])
  })
})
