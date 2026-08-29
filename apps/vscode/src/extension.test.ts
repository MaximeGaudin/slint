import assert from 'node:assert/strict'
import * as fs from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import { before, describe, it } from 'node:test'
import {
  Diagnostic,
  DiagnosticSeverity,
  type FakeTextDocument,
  Range,
  resetForTest,
  secrets,
  state,
  Uri,
} from './vscode-stub.js'

type ExtensionModule = typeof import('./extension.js')

let extension: ExtensionModule | undefined

// The fixture lives outside src/ (not compiled); __dirname is out/ at test time.
const FIXTURES = path.join(__dirname, '..', 'test', 'fixtures')
const FAKE_SLINT = path.join(FIXTURES, 'fake-slint')

const DOCUMENT_TEXT = [
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

type Finding = {
  rule: string
  severity: 'error' | 'warning' | 'info'
  message: string
  advice: string
  file: string
  location: { line: number; column: number; end_line?: number; end_column?: number }
  source: 'static' | 'model' | 'plugin'
  reference: { title: string; url: string }
  fix?: { description: string }
}

type Envelope = {
  ok: boolean
  summary: { skills: number; errors: number; warnings: number; infos: number; fixable: number }
  data: { skills: { path: string; name: string; messages: Finding[]; notes: string[] }[] }
}

type SpawnRecord = { argv: string[]; envApiKey: string | null }

function makeFinding(target: string, overrides: Partial<Finding> = {}): Finding {
  return {
    rule: 'body/demo-rule',
    severity: 'error',
    message: 'Something is off',
    advice: 'Fix it',
    file: path.join(target, 'SKILL.md'),
    location: { line: 3, column: 2 },
    source: 'static',
    reference: { title: 'Demo rule', url: 'https://slint.dev/rules/body/demo-rule' },
    ...overrides,
  }
}

function makeEnvelope(
  skills: { path: string; name: string; messages: Finding[]; notes: string[] }[],
): Envelope {
  return {
    ok: true,
    summary: {
      skills: skills.length,
      errors: 0,
      warnings: 0,
      infos: 0,
      fixable: 0,
    },
    data: { skills },
  }
}

function writeEnvelope(dir: string, name: string, envelope: Envelope): string {
  const file = path.join(dir, name)
  fs.writeFileSync(file, JSON.stringify(envelope))
  return file
}

function activateExtension(): void {
  assert.ok(extension, 'extension module must be loaded before activate()')
  resetForTest()
  const context = { subscriptions: [] as unknown[], secrets }
  extension.activate(context as unknown as Parameters<ExtensionModule['activate']>[0])
}

function setSettings(values: Record<string, unknown>): void {
  Object.assign(state.settings, values)
}

async function runCommand(name: string): Promise<unknown> {
  await new Promise((resolve) => setTimeout(resolve, 0))
  const handler = state.commands.get(name)
  assert.ok(handler, `command ${name} must be registered by activate()`)
  return Promise.resolve(handler())
}

function readRecords(record: string): SpawnRecord[] {
  if (!fs.existsSync(record)) return []
  return fs
    .readFileSync(record, 'utf8')
    .split('\n')
    .filter((line) => line.trim().length > 0)
    .map((line) => JSON.parse(line) as SpawnRecord)
}

function setFixtureEnv(values: Record<string, string | undefined>): void {
  for (const [key, value] of Object.entries(values)) {
    if (value === undefined) delete process.env[key]
    else process.env[key] = value
  }
}

function clearFixtureEnv(): void {
  for (const key of Object.keys(process.env)) {
    if (key.startsWith('FAKE_SLINT_')) delete process.env[key]
  }
}

function outputLines(): string[] {
  return state.outputChannels[0]?.lines ?? []
}

function statusText(): string {
  return state.statusItems[0]?.text ?? ''
}

function diagnosticsFor(file: string): Diagnostic[] {
  return state.diagnostics.get(Uri.file(file).fsPath) ?? []
}

async function waitFor(predicate: () => boolean, what: string, timeoutMs = 5000): Promise<void> {
  const start = Date.now()
  while (!predicate()) {
    if (Date.now() - start > timeoutMs) throw new Error(`timed out waiting for ${what}`)
    await new Promise((resolve) => setTimeout(resolve, 20))
  }
}

function fakeSkillDocument(fileName: string, text: string): FakeTextDocument {
  return {
    uri: Uri.file(fileName),
    fileName,
    isDirty: false,
    getText: () => text,
    save: () => Promise.resolve(true),
  }
}

before(async () => {
  fs.chmodSync(FAKE_SLINT, 0o755)
  extension = await import('./extension.js')
})

describe('spawning: argv + env construction (#45)', () => {
  it('builds static-run argv: target first, format/color flags, extra arguments, --no-llm', async (t) => {
    t.after(clearFixtureEnv)
    activateExtension()

    const target = fs.mkdtempSync(path.join(os.tmpdir(), 'slint-static-'))
    const record = path.join(target, 'record.jsonl')
    setFixtureEnv({
      FAKE_SLINT_RECORD: record,
      FAKE_SLINT_ENVELOPE: writeEnvelope(target, 'envelope.json', makeEnvelope([])),
    })
    setSettings({ path: FAKE_SLINT, arguments: ['--rule', 'body/demo-rule=off'] })
    state.workspaceFolders = [{ uri: Uri.file(target) }]

    await runCommand('slint.lintWorkspace')

    const records = readRecords(record)
    assert.equal(records.length, 1)
    const { argv, envApiKey } = records[0]
    assert.deepEqual(argv, [
      target,
      '--format',
      'json',
      '--no-color',
      '--rule',
      'body/demo-rule=off',
      '--no-llm',
    ])
    assert.equal(envApiKey, null)
  })

  it('builds model-run argv from llm settings and passes the API key via env, never argv', async (t) => {
    t.after(clearFixtureEnv)
    activateExtension()

    const target = fs.mkdtempSync(path.join(os.tmpdir(), 'slint-model-'))
    const record = path.join(target, 'record.jsonl')
    setFixtureEnv({
      FAKE_SLINT_RECORD: record,
      FAKE_SLINT_ENVELOPE: writeEnvelope(target, 'static.json', makeEnvelope([])),
      FAKE_SLINT_ENVELOPE_2: writeEnvelope(target, 'model.json', makeEnvelope([])),
    })
    setSettings({
      path: FAKE_SLINT,
      'llm.provider': 'openai',
      'llm.model': 'gpt-test',
    })
    // The key lives in SecretStorage now (#191): set it the way a user would, via the command.
    state.inputBoxValue = '  sk-secret  '
    await runCommand('slint.setApiKey')
    state.workspaceFolders = [{ uri: Uri.file(target) }]

    await runCommand('slint.lintWorkspaceWithModel')

    const records = readRecords(record)
    assert.equal(records.length, 2, 'static pass, then model pass')
    const modelArgv = records[1].argv
    assert.deepEqual(modelArgv.slice(modelArgv.indexOf('--llm')), [
      '--llm',
      '--llm-provider',
      'openai',
      '--llm-model',
      'gpt-test',
      '--llm-api-key-env',
      'SLINT_EDITOR_API_KEY',
    ])
    assert.equal(records[1].envApiKey, 'sk-secret', 'the key must reach the child env, trimmed')
    assert.equal(
      records.some((r) => r.argv.join(' ').includes('sk-secret')),
      false,
      'the API key must never appear on the command line',
    )
    assert.equal(modelArgv.includes('--no-llm'), false)
  })

  it('fixDocument spawns with --fix on the active SKILL.md and reverts the editor file', async (t) => {
    t.after(clearFixtureEnv)
    activateExtension()

    const target = fs.mkdtempSync(path.join(os.tmpdir(), 'slint-fix-'))
    const record = path.join(target, 'record.jsonl')
    setFixtureEnv({
      FAKE_SLINT_RECORD: record,
      FAKE_SLINT_ENVELOPE: writeEnvelope(target, 'envelope.json', makeEnvelope([])),
    })
    setSettings({ path: FAKE_SLINT })
    state.activeTextEditor = {
      document: fakeSkillDocument(path.join(target, 'SKILL.md'), DOCUMENT_TEXT),
    }

    await runCommand('slint.fixDocument')

    const records = readRecords(record)
    assert.ok(records.length >= 2, 'a fix run and a follow-up lint run')
    const fixArgv = records.find((r) => r.argv.includes('--fix'))?.argv
    assert.ok(fixArgv, 'expected a run with --fix')
    assert.equal(fixArgv[0], target)
    assert.deepEqual(fixArgv.slice(1), ['--fix', '--format', 'json', '--no-color', '--no-llm'])
    assert.equal(
      records[records.length - 1].argv.includes('--no-llm'),
      true,
      'the follow-up lint is static-only',
    )
    assert.equal(state.executeCommands.includes('workbench.action.files.revert'), true)
  })

  it('lintWithModel without a SKILL.md open only informs and never spawns', async (t) => {
    t.after(clearFixtureEnv)
    activateExtension()

    const target = fs.mkdtempSync(path.join(os.tmpdir(), 'slint-guard-'))
    const record = path.join(target, 'record.jsonl')
    setFixtureEnv({ FAKE_SLINT_RECORD: record })
    setSettings({ path: FAKE_SLINT })

    await runCommand('slint.lintWithModel')

    assert.equal(readRecords(record).length, 0, 'no slint run without an active skill')
    assert.deepEqual(state.messages, ['Open a SKILL.md to run the model pass.'])
  })
})

describe('diagnostics publishing (#45)', () => {
  it('maps envelope findings to per-file diagnostics with ranges, severities, sources and rule links', async (t) => {
    t.after(clearFixtureEnv)
    activateExtension()

    const target = fs.mkdtempSync(path.join(os.tmpdir(), 'slint-publish-'))
    const skillFile = path.join(target, 'SKILL.md')
    setFixtureEnv({
      FAKE_SLINT_ENVELOPE: writeEnvelope(
        target,
        'envelope.json',
        makeEnvelope([
          {
            path: target,
            name: 'demo',
            notes: ['skipped 7 model rules', 'note kept'],
            messages: [
              makeFinding(target, {}),
              makeFinding(target, {
                severity: 'warning',
                location: { line: 5, column: 1, end_line: 5, end_column: 9 },
              }),
              makeFinding(target, { severity: 'info' }),
              makeFinding(target, { source: 'model', rule: 'model/llm-review' }),
            ],
          },
        ]),
      ),
    })
    setSettings({ path: FAKE_SLINT })
    state.workspaceFolders = [{ uri: Uri.file(target) }]

    await runCommand('slint.lintWorkspace')

    const diagnostics = diagnosticsFor(skillFile)
    assert.equal(diagnostics.length, 4)

    const [errorFinding, warningFinding, infoFinding, modelFinding] = diagnostics
    assert.equal(errorFinding.severity, DiagnosticSeverity.Error)
    assert.equal(errorFinding.range.start.line, 2, '1-based envelope lines become 0-based')
    assert.equal(errorFinding.range.start.character, 1)
    assert.equal(errorFinding.range.end.line, 2)
    assert.equal(errorFinding.range.end.character, 201, 'end defaults to column + 200')
    assert.match(errorFinding.message, /Something is off/)
    assert.match(errorFinding.message, /What to do: Fix it/)
    assert.equal(errorFinding.source, 'slint')
    assert.deepEqual(errorFinding.code, {
      value: 'body/demo-rule',
      target: Uri.file('https://slint.dev/rules/body/demo-rule'),
    })

    assert.equal(warningFinding.severity, DiagnosticSeverity.Warning)
    assert.equal(warningFinding.range.end.character, 8, 'explicit end_column is honored')
    assert.equal(infoFinding.severity, DiagnosticSeverity.Information)
    assert.equal(modelFinding.source, 'slint-model')

    assert.equal(
      outputLines().some((line) => line.includes('skipped 7 model rules')),
      false,
      'static runs silence the skipped-model note',
    )
    assert.equal(
      outputLines().some((line) => line.includes('demo: note kept')),
      true,
      'other notes are forwarded to the output channel',
    )
    assert.equal(
      outputLines().some((line) => line.includes('· static:')),
      true,
    )
  })

  it('model pass keeps static findings and layers model findings on top', async (t) => {
    t.after(clearFixtureEnv)
    activateExtension()

    const target = fs.mkdtempSync(path.join(os.tmpdir(), 'slint-merge-'))
    const skillFile = path.join(target, 'SKILL.md')
    setFixtureEnv({
      FAKE_SLINT_RECORD: path.join(target, 'record.jsonl'),
      FAKE_SLINT_ENVELOPE: writeEnvelope(
        target,
        'static.json',
        makeEnvelope([
          { path: target, name: 'demo', notes: [], messages: [makeFinding(target, {})] },
        ]),
      ),
      FAKE_SLINT_ENVELOPE_2: writeEnvelope(
        target,
        'model.json',
        makeEnvelope([
          {
            path: target,
            name: 'demo',
            notes: [],
            messages: [makeFinding(target, { source: 'model', rule: 'model/llm-review' })],
          },
        ]),
      ),
    })
    setSettings({ path: FAKE_SLINT })
    state.workspaceFolders = [{ uri: Uri.file(target) }]

    await runCommand('slint.lintWorkspaceWithModel')

    const sources = diagnosticsFor(skillFile).map((d) => d.source)
    assert.deepEqual(sources, ['slint', 'slint-model'])
    assert.equal(
      outputLines().some((line) => line.includes('· model')),
      true,
    )
  })

  it('re-publishing clears diagnostics for skills that are now clean', async (t) => {
    t.after(clearFixtureEnv)
    activateExtension()

    const target = fs.mkdtempSync(path.join(os.tmpdir(), 'slint-clear-'))
    const skillFile = path.join(target, 'SKILL.md')
    setFixtureEnv({
      FAKE_SLINT_RECORD: path.join(target, 'record.jsonl'),
      FAKE_SLINT_ENVELOPE: writeEnvelope(
        target,
        'dirty.json',
        makeEnvelope([
          { path: target, name: 'demo', notes: [], messages: [makeFinding(target, {})] },
        ]),
      ),
      FAKE_SLINT_ENVELOPE_2: writeEnvelope(
        target,
        'clean.json',
        makeEnvelope([{ path: target, name: 'demo', notes: [], messages: [] }]),
      ),
    })
    setSettings({ path: FAKE_SLINT })
    state.workspaceFolders = [{ uri: Uri.file(target) }]

    await runCommand('slint.lintWorkspace')
    assert.equal(diagnosticsFor(skillFile).length, 1)

    await runCommand('slint.lintWorkspace')
    assert.equal(state.diagnostics.size, 0, 'the now-clean skill must not keep stale squiggles')
  })

  it('stdout that is not JSON publishes nothing and surfaces the failure', async (t) => {
    t.after(clearFixtureEnv)
    activateExtension()

    const target = fs.mkdtempSync(path.join(os.tmpdir(), 'slint-garbage-'))
    const notJson = path.join(target, 'not-json.txt')
    fs.writeFileSync(notJson, 'definitely not json')
    setFixtureEnv({ FAKE_SLINT_ENVELOPE: notJson })
    setSettings({ path: FAKE_SLINT })
    state.workspaceFolders = [{ uri: Uri.file(target) }]

    await runCommand('slint.lintWorkspace')

    assert.equal(state.diagnostics.size, 0)
    assert.equal(statusText(), '$(error) slint failed')
    assert.equal(
      outputLines().some((line) =>
        line.includes('slint answered with something that was not JSON'),
      ),
      true,
    )
  })

  it('findings on a non-zero exit are still published', async (t) => {
    t.after(clearFixtureEnv)
    activateExtension()

    const target = fs.mkdtempSync(path.join(os.tmpdir(), 'slint-exit-'))
    const skillFile = path.join(target, 'SKILL.md')
    setFixtureEnv({
      FAKE_SLINT_EXIT: '1',
      FAKE_SLINT_ENVELOPE: writeEnvelope(
        target,
        'envelope.json',
        makeEnvelope([
          { path: target, name: 'demo', notes: [], messages: [makeFinding(target, {})] },
        ]),
      ),
    })
    setSettings({ path: FAKE_SLINT })
    state.workspaceFolders = [{ uri: Uri.file(target) }]

    await runCommand('slint.lintWorkspace')

    assert.equal(diagnosticsFor(skillFile).length, 1)
    assert.equal(
      outputLines().some((line) => line.includes('· static:')),
      true,
    )
  })

  it('a slint binary that cannot run surfaces the failure without publishing', async (t) => {
    t.after(clearFixtureEnv)
    activateExtension()

    const target = fs.mkdtempSync(path.join(os.tmpdir(), 'slint-missing-'))
    setFixtureEnv({})
    setSettings({ path: path.join(target, 'missing-slint') })
    state.workspaceFolders = [{ uri: Uri.file(target) }]

    await runCommand('slint.lintWorkspace')

    assert.equal(state.diagnostics.size, 0)
    assert.equal(statusText(), '$(error) slint failed')
    assert.equal(
      outputLines().some((line) => line.includes('slint could not run:')),
      true,
    )
  })
})

describe('concurrency (#45)', () => {
  it('a newer lint for the same target kills the in-flight run and owns the diagnostics', async (t) => {
    t.after(clearFixtureEnv)
    activateExtension()

    const target = fs.mkdtempSync(path.join(os.tmpdir(), 'slint-race-'))
    const staleFile = path.join(target, 'stale', 'SKILL.md')
    const freshFile = path.join(target, 'fresh', 'SKILL.md')
    const record = path.join(target, 'record.jsonl')
    const killedMarker = path.join(target, 'killed')

    setFixtureEnv({
      FAKE_SLINT_RECORD: record,
      FAKE_SLINT_HANG: '1',
      FAKE_SLINT_KILLED: killedMarker,
      FAKE_SLINT_ENVELOPE: writeEnvelope(
        target,
        'stale.json',
        makeEnvelope([
          {
            path: target,
            name: 'demo',
            notes: [],
            messages: [makeFinding(target, { file: staleFile })],
          },
        ]),
      ),
      FAKE_SLINT_ENVELOPE_2: writeEnvelope(
        target,
        'fresh.json',
        makeEnvelope([
          {
            path: target,
            name: 'demo',
            notes: [],
            messages: [makeFinding(target, { file: freshFile })],
          },
        ]),
      ),
    })
    setSettings({ path: FAKE_SLINT })
    state.workspaceFolders = [{ uri: Uri.file(target) }]

    const first = runCommand('slint.lintWorkspace')
    await waitFor(() => readRecords(record).length === 1, 'the first slint child to start')

    setFixtureEnv({ FAKE_SLINT_HANG: undefined })

    const second = runCommand('slint.lintWorkspace')
    await Promise.resolve(second)

    await Promise.race([
      first,
      new Promise((_, reject) =>
        setTimeout(() => reject(new Error('the cancelled run never settled')), 5000),
      ),
    ])

    assert.equal(fs.existsSync(killedMarker), true, 'the superseded child process is killed')

    await second

    assert.equal(diagnosticsFor(freshFile).length, 1, 'the newer run publishes its findings')
    assert.equal(diagnosticsFor(staleFile).length, 0, 'the cancelled run never publishes')
    assert.equal(
      outputLines().some((line) => line.includes('cancelled (newer lint started)')),
      true,
    )
    assert.equal(
      statusText().includes('sync~spin'),
      false,
      'the status bar is never stuck spinning',
    )
  })
})

describe('code actions (#45)', () => {
  type FakeCodeActionProvider = {
    provideCodeActions(
      document: unknown,
      range: unknown,
      context: { diagnostics: Diagnostic[] },
    ): CodeActionInstance[]
  }
  type CodeActionInstance = {
    title: string
    kind: { value: string }
    diagnostics: Diagnostic[] | undefined
    isPreferred: boolean | undefined
    edit: { inserts: { uri: Uri; line: number; character: number; text: string }[] } | undefined
  }

  function provider(): FakeCodeActionProvider {
    activateExtension()
    const provider = state.codeActionProviders[0]
    assert.ok(provider, 'activate() must register the code action provider')
    return provider as FakeCodeActionProvider
  }

  function slintDiagnostic(line: number): Diagnostic {
    const diagnostic = new Diagnostic(
      new Range(line, 0, line, 4),
      'Something is off\n\nWhat to do: Fix it',
      DiagnosticSeverity.Error,
    )
    diagnostic.source = 'slint'
    diagnostic.code = { value: 'body/posix-paths' }
    return diagnostic
  }

  it('maps slint diagnostics to ignore quick fixes, next-line preferred', () => {
    const actions = provider().provideCodeActions(
      fakeSkillDocument('/skills/demo/SKILL.md', DOCUMENT_TEXT),
      undefined,
      { diagnostics: [slintDiagnostic(7)] },
    )

    assert.equal(actions.length, 2)
    for (const action of actions) {
      assert.equal(action.kind.value, 'QuickFix')
      assert.match(action.title, /ignore/i)
      assert.match(action.title, /body\/posix-paths/)
      assert.equal(action.diagnostics?.length, 1, 'the action is tied to its diagnostic')
    }

    const [nextLine, file] = actions
    assert.equal(nextLine.isPreferred, true, 'the next-line ignore is the preferred fix')
    assert.deepEqual(nextLine.edit?.inserts, [
      {
        uri: Uri.file('/skills/demo/SKILL.md'),
        line: 7,
        character: 0,
        text: '<!-- slint-disable-next-line body/posix-paths -->\n',
      },
    ])
    assert.equal(file.isPreferred, false)
    assert.deepEqual(file.edit?.inserts, [
      {
        uri: Uri.file('/skills/demo/SKILL.md'),
        line: 4,
        character: 0,
        text: '<!-- slint-disable body/posix-paths -->\n',
      },
    ])
  })

  it('offers nothing for foreign diagnostics, missing codes, or non-skill documents', () => {
    const providerInstance = provider()

    const foreign = slintDiagnostic(7)
    foreign.source = 'eslint'
    assert.equal(
      providerInstance.provideCodeActions(
        fakeSkillDocument('/skills/demo/SKILL.md', DOCUMENT_TEXT),
        undefined,
        { diagnostics: [foreign] },
      ).length,
      0,
    )

    const noCode = slintDiagnostic(7)
    noCode.code = undefined
    assert.equal(
      providerInstance.provideCodeActions(
        fakeSkillDocument('/skills/demo/SKILL.md', DOCUMENT_TEXT),
        undefined,
        { diagnostics: [noCode] },
      ).length,
      0,
    )

    assert.equal(
      providerInstance.provideCodeActions(
        fakeSkillDocument('/skills/demo/README.md', DOCUMENT_TEXT),
        undefined,
        { diagnostics: [slintDiagnostic(7)] },
      ).length,
      0,
      'non-SKILL.md documents get no ignore actions',
    )
  })
})
