import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import * as path from 'node:path'
import { describe, it } from 'node:test'
import { planApiKeyMigration } from './secrets.js'

const manifest = JSON.parse(readFileSync(path.join(__dirname, '..', 'package.json'), 'utf8')) as {
  contributes: {
    commands: Array<{ command: string }>
    configuration: { properties: Record<string, unknown> }
  }
}

describe('package.json manifest (#191)', () => {
  it('no longer declares a plaintext slint.llm.apiKey setting', () => {
    assert.equal(
      manifest.contributes.configuration.properties['slint.llm.apiKey'],
      undefined,
      'a credential in settings.json is plaintext on disk and in editor sync',
    )
  })

  it('declares commands to store and clear the key in SecretStorage', () => {
    const commands = manifest.contributes.commands.map((command) => command.command)
    assert.ok(commands.includes('slint.setApiKey'), 'need a way to store the key')
    assert.ok(commands.includes('slint.clearApiKey'), 'need a way to remove the key')
  })
})

describe('planApiKeyMigration (#191)', () => {
  it('does nothing when no scope holds a key', () => {
    assert.equal(planApiKeyMigration({ global: '' }, null), undefined)
    assert.equal(planApiKeyMigration({}, null), undefined)
  })

  it('clears leftover plaintext copies even when the key already lives in SecretStorage', () => {
    assert.deepEqual(planApiKeyMigration({ global: 'sk-old' }, 'sk-stored'), {
      clear: ['global'],
    })
    assert.equal(planApiKeyMigration({}, 'sk-stored'), undefined)
  })

  it('migrates a user-level key and clears that scope', () => {
    assert.deepEqual(planApiKeyMigration({ global: 'sk-global' }, null), {
      value: 'sk-global',
      clear: ['global'],
    })
  })

  it('prefers the most specific scope and clears every scope holding a copy', () => {
    assert.deepEqual(
      planApiKeyMigration({ global: 'sk-global', workspace: 'sk-workspace' }, null),
      { value: 'sk-workspace', clear: ['workspace', 'global'] },
    )
    assert.deepEqual(
      planApiKeyMigration(
        { global: 'sk-global', workspace: 'sk-workspace', workspaceFolder: 'sk-folder' },
        null,
      ),
      { value: 'sk-folder', clear: ['workspaceFolder', 'workspace', 'global'] },
    )
  })

  it('ignores blank values', () => {
    assert.equal(planApiKeyMigration({ global: '   ' }, null), undefined)
  })
})
