import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import { describe, it } from 'node:test'
import { scanRuleCatalogue } from './rules-catalogue.js'

const manifest = JSON.parse(readFileSync(path.resolve(__dirname, '../package.json'), 'utf8')) as {
  contributes: {
    codeActions: { title: string; kind: string }[]
    configuration: { properties: Record<string, Record<string, unknown>> }
  }
}

const properties = manifest.contributes.configuration.properties

describe('settings schema (#51)', () => {
  it('scopes every setting a team may want per folder to the resource', () => {
    const perFolder = [
      'slint.path',
      'slint.onSave',
      'slint.onType',
      'slint.llm.provider',
      'slint.llm.model',
      'slint.arguments',
      'slint.rules',
    ]

    for (const name of perFolder) {
      assert.ok(properties[name], `${name} should exist`)
      assert.equal(properties[name].scope, 'resource', `${name} should be resource-scoped`)
    }
  })

  it('keeps the API key out of workspace reach so it cannot be committed', () => {
    // #191 removed the plaintext setting entirely in favor of SecretStorage, superseding the
    // machine-scope hardening: an API key must not be a settings.json value at all.
    assert.ok(
      !('slint.llm.apiKey' in properties),
      'the API key must live in SecretStorage, not in any settings scope',
    )
  })
})

describe('slint.rules schema (#63)', () => {
  const schema = properties['slint.rules'] as {
    type: string
    additionalProperties: { type: string; enum: string[] }
    properties: Record<string, { enum: string[]; description?: string }>
  }

  it('is a structured map of rule id to severity', () => {
    assert.equal(schema.type, 'object')
    assert.deepEqual(schema.additionalProperties.enum, ['off', 'error', 'warning', 'info'])
  })

  it('offers an enum property for every rule in the core catalogue', () => {
    for (const rule of scanRuleCatalogue()) {
      const entry = schema.properties[rule.name]
      assert.ok(entry, `slint.rules should autocomplete ${rule.name}`)
      assert.deepEqual(entry.enum, ['off', 'error', 'warning', 'info'])
      assert.match(entry.description ?? '', new RegExp(rule.defaultSeverity))
    }
  })

  it('lists nothing the catalogue does not know about', () => {
    const known = new Set(scanRuleCatalogue().map((rule) => rule.name))

    for (const name of Object.keys(schema.properties)) {
      assert.ok(known.has(name), `${name} is in the manifest but not in the core catalogue`)
    }
  })
})

describe('code actions manifest (#49)', () => {
  it('documents the per-finding quick fix and the fix-all source action', () => {
    const kinds = manifest.contributes.codeActions.map((action) => action.kind)

    assert.ok(
      kinds.some((kind) => kind.startsWith('quickfix')),
      'expected a quickfix entry',
    )
    assert.ok(
      kinds.includes('source.fixAll.slint'),
      'expected source.fixAll.slint so Fix on save can be wired in settings',
    )
  })
})
