import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import * as path from 'node:path'
import { describe, it } from 'node:test'

const manifest = JSON.parse(readFileSync(path.join(__dirname, '..', 'package.json'), 'utf8')) as {
  capabilities?: { untrustedWorkspaces?: { enabled?: boolean } }
  restrictedConfigurations?: string[]
}

describe('package.json manifest (#136)', () => {
  it('declares that the extension runs only in trusted workspaces', () => {
    assert.equal(
      manifest.capabilities?.untrustedWorkspaces?.enabled,
      false,
      'the extension executes an external binary whose path/args the workspace can steer, so it must not run untrusted',
    )
  })

  it('restricts the settings that steer the executed binary', () => {
    const restricted = manifest.restrictedConfigurations ?? []
    for (const key of ['slint.path', 'slint.arguments']) {
      assert.ok(
        restricted.includes(key),
        `${key} can be set per workspace and steers the process the extension runs`,
      )
    }
  })

  it('restricts the LLM settings so a workspace cannot steer the model pass (#50)', () => {
    const restricted = manifest.restrictedConfigurations ?? []
    for (const key of ['slint.llm.provider', 'slint.llm.model', 'slint.llm.apiKey']) {
      assert.ok(restricted.includes(key), `${key} can be set per workspace`)
    }
  })
})
