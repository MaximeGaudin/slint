import { execFile } from 'node:child_process'
import * as path from 'node:path'
import { promisify } from 'node:util'
import * as vscode from 'vscode'

const run = promisify(execFile)

/** Env var the extension injects when `slint.llm.apiKey` is set. Never appears on the argv. */
const EDITOR_API_KEY_ENV = 'SLINT_EDITOR_API_KEY'

/**
 * slint, in the editor.
 *
 * Save behavior is chosen in settings (`slint.onSave`: nothing / no-llm / llm). Typing can run
 * static rules when `slint.onType` is on. Model-on-save still publishes static findings first so
 * the Problems panel is never blank while waiting on a provider.
 */

/** What `slint --format json` answers with. */
type Envelope = {
  ok: boolean
  summary: { skills: number; errors: number; warnings: number; infos: number; fixable: number }
  data: { skills: SkillReport[] }
}

type SkillReport = {
  path: string
  name: string
  messages: Finding[]
  notes: string[]
}

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

type Spawn = {
  argv: string[]
  env: NodeJS.ProcessEnv
}

let diagnostics: vscode.DiagnosticCollection
let output: vscode.OutputChannel
let status: vscode.StatusBarItem
let pending: NodeJS.Timeout | undefined
/** How many lint runs are in flight — status stays "running" until the last one finishes. */
let inflight = 0
/** Per-target generation so a slow model pass from save N cannot overwrite save N+1. */
const generations = new Map<string, number>()
/** Last static envelope per target — merged back in when the model pass publishes so static never vanishes. */
const lastStatic = new Map<string, Envelope>()

export function activate(context: vscode.ExtensionContext): void {
  diagnostics = vscode.languages.createDiagnosticCollection('slint')
  output = vscode.window.createOutputChannel('slint')
  status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 50)
  status.command = 'slint.showOutput'
  status.tooltip = 'Show the slint output channel'
  status.text = 'slint'
  status.show()

  context.subscriptions.push(diagnostics, output, status)

  context.subscriptions.push(
    vscode.commands.registerCommand('slint.lintWorkspace', () => lintWorkspace({ model: false })),
    vscode.commands.registerCommand('slint.lintWithModel', () => lintActiveWithModel()),
    vscode.commands.registerCommand('slint.lintWorkspaceWithModel', () =>
      lintWorkspace({ model: true }),
    ),
    vscode.commands.registerCommand('slint.fixDocument', () => fixActiveDocument()),
    vscode.commands.registerCommand('slint.showOutput', () => output.show(true)),
  )

  context.subscriptions.push(
    vscode.workspace.onDidSaveTextDocument((document) => {
      if (!isSkill(document)) return

      const onSave = setting<string>('onSave') ?? 'no-llm'
      if (onSave === 'nothing') return

      void lint(directoryOf(document), { model: onSave === 'llm' })
    }),
  )

  context.subscriptions.push(
    vscode.workspace.onDidChangeTextDocument((event) => {
      if (!setting<boolean>('onType')) return
      if (!isSkill(event.document)) return

      // Debounced, and static-only: the point of linting while typing is that it is instant.
      clearTimeout(pending)
      pending = setTimeout(() => void lint(directoryOf(event.document), { model: false }), 400)
    }),
  )

  // Open skills at activation: static only, so the Problems panel is populated without spending.
  for (const document of vscode.workspace.textDocuments) {
    if (isSkill(document) && (setting<string>('onSave') ?? 'no-llm') !== 'nothing') {
      void lint(directoryOf(document), { model: false })
    }
  }
}

export function deactivate(): void {
  diagnostics?.dispose()
  clearTimeout(pending)
}

function setting<T>(name: string): T | undefined {
  return vscode.workspace.getConfiguration('slint').get<T>(name)
}

function isSkill(document: vscode.TextDocument): boolean {
  return path.basename(document.fileName) === 'SKILL.md'
}

function directoryOf(document: vscode.TextDocument): string {
  return path.dirname(document.fileName)
}

function setStatus(text: string, detail?: string): void {
  status.text = text
  status.tooltip = detail ?? 'Show the slint output channel'
}

/**
 * Builds argv + env for one slint run.
 *
 * LLM settings from the editor become `--llm-*` flags. The API key is never put on the command
 * line: it is injected as `SLINT_EDITOR_API_KEY` and pointed at with `--llm-api-key-env`.
 */
function spawnFor(target: string, options: { model?: boolean; fix?: boolean } = {}): Spawn {
  const binaryArgs: string[] = [target]
  if (options.fix) binaryArgs.push('--fix')
  binaryArgs.push('--format', 'json', '--no-color', ...(setting<string[]>('arguments') ?? []))

  if (options.model) {
    binaryArgs.push('--llm', ...llmArgv())
  } else {
    binaryArgs.push('--no-llm')
  }

  return { argv: binaryArgs, env: llmEnv() }
}

function llmArgv(): string[] {
  const args: string[] = []
  const provider = setting<string>('llm.provider')?.trim()
  const model = setting<string>('llm.model')?.trim()
  const apiKey = setting<string>('llm.apiKey')?.trim()

  if (provider) args.push('--llm-provider', provider)
  if (model) args.push('--llm-model', model)
  if (apiKey) args.push('--llm-api-key-env', EDITOR_API_KEY_ENV)

  return args
}

function llmEnv(): NodeJS.ProcessEnv {
  const apiKey = setting<string>('llm.apiKey')?.trim()
  if (!apiKey) return { ...process.env }

  return { ...process.env, [EDITOR_API_KEY_ENV]: apiKey }
}

async function lintActiveWithModel(): Promise<void> {
  const editor = vscode.window.activeTextEditor

  if (!editor || !isSkill(editor.document)) {
    void vscode.window.showInformationMessage('Open a SKILL.md to run the model pass.')
    return
  }

  await lint(directoryOf(editor.document), { model: true })
}

async function lintWorkspace(options: { model: boolean }): Promise<void> {
  const folders = vscode.workspace.workspaceFolders ?? []

  // One slint invocation per folder: the CLI parallelizes the model pass across skills inside it.
  for (const folder of folders) {
    await lint(folder.uri.fsPath, options)
  }
}

/**
 * Static first (publish immediately), then the model pass when asked.
 *
 * Save never asks: the model pass is a command. Waiting on a provider before showing the half that
 * needs no network is how people conclude the static rules "stopped working".
 */
async function lint(target: string, options: { model: boolean }): Promise<void> {
  const generation = (generations.get(target) ?? 0) + 1
  generations.set(target, generation)

  const staticResult = await runPass(
    target,
    { model: false, followWithModel: options.model },
    generation,
  )
  if (!staticResult || generations.get(target) !== generation) return

  if (!options.model) return

  setStatus(
    '$(sync~spin) slint · model',
    `${summarize(staticResult.summary)} — model pass running…`,
  )
  output.appendLine(`→ ${path.basename(target)} · model`)

  await runPass(target, { model: true, followWithModel: false }, generation)
}

/**
 * Runs one slint pass and publishes if this generation is still current.
 * Returns the envelope on success, or undefined when the binary failed.
 */
async function runPass(
  target: string,
  options: { model: boolean; followWithModel: boolean },
  generation: number,
): Promise<Envelope | undefined> {
  const binary = setting<string>('path') ?? 'slint'
  const { argv, env } = spawnFor(target, options)
  const label = path.basename(target)

  inflight += 1
  if (!options.model) {
    setStatus('$(sync~spin) slint', `Linting ${label}…`)
    output.appendLine(`→ ${label} · static`)
  }

  let stdout: string

  try {
    const result = await run(binary, argv, { maxBuffer: 16 * 1024 * 1024, env })
    stdout = result.stdout
  } catch (failure) {
    const error = failure as { stdout?: string; code?: number; message?: string }

    // Findings are not failures: slint exits 1/2 when it found problems, and that run is the useful
    // one. Only an empty stdout means slint itself could not run.
    if (!error.stdout) {
      output.appendLine(`slint could not run: ${error.message ?? 'unknown failure'}`)
      output.appendLine(`  ${binary} ${argv.join(' ')}`)
      finishStatus(
        '$(error) slint failed',
        error.message ?? 'slint could not run — click for details',
      )
      return undefined
    }

    stdout = error.stdout
  }

  if (generations.get(target) !== generation) {
    finishStatus('$(sync~spin) slint', 'A newer lint superseded this run')
    return undefined
  }

  let envelope: Envelope

  try {
    envelope = JSON.parse(stdout) as Envelope
  } catch {
    output.appendLine('slint answered with something that was not JSON.')
    finishStatus('$(error) slint failed', 'slint answered with something that was not JSON')
    return undefined
  }

  publish(envelope, {
    target,
    // The model pass must not erase the static findings already on screen. Seed from the static
    // envelope we just published; prefer any static/plugin rows the --llm run also returned.
    keepStatic: options.model ? lastStatic.get(target) : undefined,
    // Static-only runs always skip the model by design — don't spam the output channel with
    // "Skipped N model rules" as if something went wrong.
    silenceSkippedModelNote: !options.model,
  })

  if (!options.model) {
    lastStatic.set(target, envelope)
  }

  const kind = options.model ? 'model' : 'static'
  const summary = summarizePublished(envelope, options.model ? lastStatic.get(target) : undefined)
  output.appendLine(`✓ ${label} · ${kind}: ${summary}`)

  if (options.followWithModel) {
    // Model pass is about to start — free this slot without clearing the "running" feel.
    inflight = Math.max(0, inflight - 1)
    setStatus(`$(check) slint: ${summary}`, 'Static done — model pass starting…')
  } else {
    finishFromSummaryText(summary, envelope.summary.skills)
  }

  return envelope
}

function summarizePublished(envelope: Envelope, staticSeed?: Envelope): string {
  let errors = 0
  let warnings = 0
  let infos = 0

  const count = (finding: Finding) => {
    if (finding.severity === 'error') errors += 1
    else if (finding.severity === 'warning') warnings += 1
    else infos += 1
  }

  if (staticSeed) {
    for (const skill of staticSeed.data.skills) {
      for (const finding of skill.messages) {
        if (finding.source !== 'model') count(finding)
      }
    }
    for (const skill of envelope.data.skills) {
      for (const finding of skill.messages) {
        if (finding.source === 'model') count(finding)
      }
    }
  } else {
    errors = envelope.summary.errors
    warnings = envelope.summary.warnings
    infos = envelope.summary.infos
  }

  return summarize({ errors, warnings, infos, skills: envelope.summary.skills, fixable: 0 })
}

function summarize(summary: Envelope['summary']): string {
  const { errors, warnings, infos } = summary
  const parts: string[] = []
  if (errors) parts.push(`${errors} error${errors === 1 ? '' : 's'}`)
  if (warnings) parts.push(`${warnings} warning${warnings === 1 ? '' : 's'}`)
  if (infos) parts.push(`${infos} info`)
  return parts.length === 0 ? 'clean' : parts.join(', ')
}

function finishStatus(text: string, detail?: string): void {
  inflight = Math.max(0, inflight - 1)
  if (inflight > 0) {
    setStatus('$(sync~spin) slint', `${inflight} lint run(s) still in progress`)
    return
  }

  setStatus(text, detail)
}

function finishFromSummaryText(text: string, skills: number): void {
  if (text === 'clean') {
    finishStatus('$(check) slint', `No problems across ${skills} skill${skills === 1 ? '' : 's'}`)
    return
  }

  const icon = text.includes('error') ? '$(error)' : '$(warning)'
  finishStatus(`${icon} slint: ${text}`, 'Click to open the slint output channel')
}

/**
 * Publishes findings into the Problems panel.
 *
 * When `keepStatic` is set (model pass), static/plugin findings from that earlier envelope are kept
 * and only model findings from the new envelope are layered on. A naive replace was wiping the
 * static half as soon as the model answered.
 */
function publish(
  envelope: Envelope,
  options: {
    target: string
    keepStatic?: Envelope
    silenceSkippedModelNote?: boolean
  } = { target: '' },
): void {
  const byFile = new Map<string, vscode.Diagnostic[]>()
  const skillPaths = envelope.data.skills.map((skill) => skill.path)

  const add = (finding: Finding) => {
    const list = byFile.get(finding.file) ?? []
    list.push(toDiagnostic(finding))
    byFile.set(finding.file, list)
  }

  const emitNotes = (skill: SkillReport) => {
    for (const note of skill.notes) {
      if (options.silenceSkippedModelNote && isSkippedModelNote(note)) continue
      output.appendLine(`${skill.name}: ${note}`)
    }
  }

  if (options.keepStatic) {
    for (const skill of options.keepStatic.data.skills) {
      for (const finding of skill.messages) {
        if (finding.source !== 'model') add(finding)
      }
    }
    for (const skill of envelope.data.skills) {
      for (const finding of skill.messages) {
        if (finding.source === 'model') add(finding)
      }
      emitNotes(skill)
    }
  } else {
    for (const skill of envelope.data.skills) {
      for (const finding of skill.messages) {
        add(finding)
      }
      emitNotes(skill)
    }
  }

  // Drop every diagnostic under these skills, then write the merged set — otherwise a file that
  // was dirty and is now clean keeps its stale squiggle forever.
  clearSkills(skillPaths)

  for (const [file, list] of byFile) {
    diagnostics.set(vscode.Uri.file(file), list)
  }
}

/** CLI note when --no-llm / model not requested — expected in the editor, not an error. */
function isSkippedModelNote(note: string): boolean {
  return /skipped \d+ model rules/i.test(note) || /need a model and did not run/i.test(note)
}

function clearSkills(skillPaths: string[]): void {
  const stale: vscode.Uri[] = []

  diagnostics.forEach((uri) => {
    const file = uri.fsPath
    if (
      skillPaths.some(
        (root) =>
          file === root || file.startsWith(root + path.sep) || file === path.join(root, 'SKILL.md'),
      )
    ) {
      stale.push(uri)
    }
  })

  for (const uri of stale) {
    diagnostics.delete(uri)
  }
}

function toDiagnostic(finding: Finding): vscode.Diagnostic {
  const line = Math.max(0, finding.location.line - 1)
  const column = Math.max(0, finding.location.column - 1)
  const endLine = Math.max(0, (finding.location.end_line ?? finding.location.line) - 1)
  const endColumn = Math.max(0, (finding.location.end_column ?? finding.location.column + 200) - 1)

  const diagnostic = new vscode.Diagnostic(
    new vscode.Range(line, column, endLine, endColumn),
    `${finding.message}\n\nWhat to do: ${finding.advice}`,
    severityOf(finding.severity),
  )

  // Keep source short so VS Code does not glue it to the rule id ("modelllm/...").
  diagnostic.source = finding.source === 'static' ? 'slint' : 'slint-model'

  // The rule id links to what it is derived from, so "why is this a rule" is one click away.
  diagnostic.code = {
    value: finding.rule,
    target: vscode.Uri.parse(finding.reference.url),
  }

  if (finding.fix) {
    diagnostic.tags = []
  }

  return diagnostic
}

function severityOf(severity: Finding['severity']): vscode.DiagnosticSeverity {
  switch (severity) {
    case 'error':
      return vscode.DiagnosticSeverity.Error
    case 'warning':
      return vscode.DiagnosticSeverity.Warning
    default:
      return vscode.DiagnosticSeverity.Information
  }
}

/**
 * Applies the computed fixes to the skill in front of the reader.
 *
 * slint rewrites the files itself, so the editor's job is to run it and then make sure what is on
 * screen is what is on disk.
 */
async function fixActiveDocument(): Promise<void> {
  const editor = vscode.window.activeTextEditor

  if (!editor || !isSkill(editor.document)) {
    void vscode.window.showInformationMessage('Open a SKILL.md to fix it.')
    return
  }

  if (editor.document.isDirty) {
    await editor.document.save()
  }

  const binary = setting<string>('path') ?? 'slint'
  const directory = directoryOf(editor.document)
  const { argv, env } = spawnFor(directory, { fix: true, model: false })

  setStatus('$(sync~spin) slint', 'Applying fixes…')

  try {
    await run(binary, argv, { maxBuffer: 16 * 1024 * 1024, env })
  } catch (failure) {
    const error = failure as { stdout?: string; message?: string }
    if (!error.stdout) {
      setStatus('$(error) slint failed', error.message ?? 'slint could not run')
      void vscode.window.showErrorMessage(`slint could not run: ${error.message ?? 'unknown'}`)
      return
    }
  }

  await vscode.commands.executeCommand('workbench.action.files.revert')
  await lint(directory, { model: false })
}
