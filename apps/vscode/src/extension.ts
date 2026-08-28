import { execFile } from 'node:child_process'
import * as path from 'node:path'
import { promisify } from 'node:util'
import * as vscode from 'vscode'
import {
  diagnosticRangeForFinding,
  patchText,
  positionAtByteOffset,
  quickFixEditForFinding,
} from './fixes.js'
import { ignoreEditsForFinding, ruleIdFromCode } from './ignore.js'
import { LintRunCoordinator, type StatusUpdate } from './lint-runs.js'

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
  fix?: { start: number; end: number; replacement: string; description: string }
}

type Spawn = {
  argv: string[]
  env: NodeJS.ProcessEnv
}

let diagnostics: vscode.DiagnosticCollection
let output: vscode.OutputChannel
let status: vscode.StatusBarItem
let pending: NodeJS.Timeout | undefined
/** Cancels superseded child processes and keeps the status bar honest across overlapping runs. */
const runs = new LintRunCoordinator()
/** Last static envelope per target — merged back in when the model pass publishes so static never vanishes. */
const lastStatic = new Map<string, Envelope>()
/**
 * Fixable findings per document, straight from the last published envelope. The Problems panel
 * carries only rule ids and ranges, so the quick-fix provider reads the computed fixes from here.
 */
const fixableByFile = new Map<string, Finding[]>()

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
    vscode.languages.registerCodeActionsProvider(
      { language: 'markdown', pattern: '**/SKILL.md' },
      new IgnoreCodeActionProvider(),
      { providedCodeActionKinds: IgnoreCodeActionProvider.providedCodeActionKinds },
    ),
  )

  context.subscriptions.push(
    vscode.languages.registerCodeActionsProvider(
      { language: 'markdown', pattern: '**/SKILL.md' },
      new FixCodeActionProvider(),
      { providedCodeActionKinds: FixCodeActionProvider.providedCodeActionKinds },
    ),
  )

  context.subscriptions.push(
    vscode.workspace.onDidSaveTextDocument((document) => {
      if (!isSkill(document)) return

      const onSave = setting<string>('onSave', document.uri) ?? 'no-llm'
      if (onSave === 'nothing') return

      void lint(directoryOf(document), { model: onSave === 'llm', resource: document.uri })
    }),
  )

  context.subscriptions.push(
    vscode.workspace.onDidChangeTextDocument((event) => {
      if (!setting<boolean>('onType', event.document.uri)) return
      if (!isSkill(event.document)) return

      // Debounced, and static-only: the point of linting while typing is that it is instant.
      clearTimeout(pending)
      pending = setTimeout(
        () =>
          void lint(directoryOf(event.document), { model: false, resource: event.document.uri }),
        400,
      )
    }),
  )

  // Open skills at activation: static only, so the Problems panel is populated without spending.
  for (const document of vscode.workspace.textDocuments) {
    if (isSkill(document) && (setting<string>('onSave', document.uri) ?? 'no-llm') !== 'nothing') {
      void lint(directoryOf(document), { model: false, resource: document.uri })
    }
  }
}

export function deactivate(): void {
  diagnostics?.dispose()
  clearTimeout(pending)
}

function setting<T>(name: string, resource?: vscode.Uri): T | undefined {
  // Passing the document's URI is what makes a setting resolve per folder: in a multi-root
  // workspace each folder can carry its own value, and the document decides which one wins.
  return vscode.workspace.getConfiguration('slint', resource).get<T>(name)
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
 *
 * `resource` is the document (or folder) the run is for: every setting read here resolves against
 * it, so two folders in one workspace can lint with different binaries, providers, or rules.
 */
function spawnFor(
  target: string,
  options: { model?: boolean; fix?: boolean; resource?: vscode.Uri } = {},
): Spawn {
  const binaryArgs: string[] = [target]
  if (options.fix) binaryArgs.push('--fix')
  binaryArgs.push(
    '--format',
    'json',
    '--no-color',
    ...(setting<string[]>('arguments', options.resource) ?? []),
  )

  if (options.model) {
    binaryArgs.push('--llm', ...llmArgv(options.resource))
  } else {
    binaryArgs.push('--no-llm')
  }

  return { argv: binaryArgs, env: llmEnv(options.resource) }
}

function llmArgv(resource?: vscode.Uri): string[] {
  const args: string[] = []
  const provider = setting<string>('llm.provider', resource)?.trim()
  const model = setting<string>('llm.model', resource)?.trim()
  const apiKey = setting<string>('llm.apiKey', resource)?.trim()

  if (provider) args.push('--llm-provider', provider)
  if (model) args.push('--llm-model', model)
  if (apiKey) args.push('--llm-api-key-env', EDITOR_API_KEY_ENV)

  return args
}

function llmEnv(resource?: vscode.Uri): NodeJS.ProcessEnv {
  const apiKey = setting<string>('llm.apiKey', resource)?.trim()
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
    await lint(folder.uri.fsPath, { ...options, resource: folder.uri })
  }
}

/**
 * Static first (publish immediately), then the model pass when asked.
 *
 * Save never asks: the model pass is a command. Waiting on a provider before showing the half that
 * needs no network is how people conclude the static rules "stopped working".
 */
async function lint(
  target: string,
  options: { model: boolean; resource?: vscode.Uri },
): Promise<void> {
  const { generation, signal } = runs.begin(target)

  const staticResult = await runPass(
    target,
    { model: false, followWithModel: options.model, resource: options.resource },
    generation,
    signal,
  )
  if (!staticResult || !runs.isCurrent(target, generation)) return

  if (!options.model) return

  setStatus(
    '$(sync~spin) slint · model',
    `${summarize(staticResult.summary)} — model pass running…`,
  )
  output.appendLine(`→ ${path.basename(target)} · model`)

  await runPass(
    target,
    { model: true, followWithModel: false, resource: options.resource },
    generation,
    signal,
  )
}

/**
 * Runs one slint pass and publishes if this generation is still current.
 * Returns the envelope on success, or undefined when the binary failed.
 */
async function runPass(
  target: string,
  options: { model: boolean; followWithModel: boolean; resource?: vscode.Uri },
  generation: number,
  signal: AbortSignal,
): Promise<Envelope | undefined> {
  const binary = setting<string>('path', options.resource) ?? 'slint'
  const { argv, env } = spawnFor(target, options)
  const label = path.basename(target)

  runs.markStarted()
  if (!options.model) {
    setStatus('$(sync~spin) slint', `Linting ${label}…`)
    output.appendLine(`→ ${label} · static`)
  }

  let stdout: string

  try {
    const result = await run(binary, argv, { maxBuffer: 16 * 1024 * 1024, env, signal })
    stdout = result.stdout
  } catch (failure) {
    if (isAbortError(failure) || signal.aborted || !runs.isCurrent(target, generation)) {
      output.appendLine(`· ${label} · cancelled (newer lint started)`)
      applyStatus(runs.markFinished({ superseded: true }))
      return undefined
    }

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

  if (!runs.isCurrent(target, generation)) {
    applyStatus(runs.markFinished({ superseded: true }))
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
    runs.releaseSlot()
    setStatus(`$(check) slint: ${summary}`, 'Static done — model pass starting…')
  } else {
    finishFromSummaryText(summary, envelope.summary.skills)
  }

  return envelope
}

function isAbortError(failure: unknown): boolean {
  if (!failure || typeof failure !== 'object') return false
  const error = failure as { name?: string; code?: string }
  return error.name === 'AbortError' || error.code === 'ABORT_ERR'
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
  applyStatus(runs.markFinished({ text, detail }))
}

function applyStatus(update: StatusUpdate): void {
  if (update.type === 'noop') return
  setStatus(update.text, update.detail)
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
  const fixable = new Map<string, Finding[]>()
  const skillPaths = envelope.data.skills.map((skill) => skill.path)

  const add = (finding: Finding) => {
    const list = byFile.get(finding.file) ?? []
    list.push(toDiagnostic(finding))
    byFile.set(finding.file, list)

    if (finding.fix) {
      const fixes = fixable.get(finding.file) ?? []
      fixes.push(finding)
      fixable.set(finding.file, fixes)
    }
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
    fixableByFile.set(file, fixable.get(file) ?? [])
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
    fixableByFile.delete(uri.fsPath)
  }
}

function toDiagnostic(finding: Finding): vscode.Diagnostic {
  const range = diagnosticRangeForFinding(finding)

  const diagnostic = new vscode.Diagnostic(
    new vscode.Range(range.start.line, range.start.character, range.end.line, range.end.character),
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

/** Quick Fix entries that insert `slint-disable` / `slint-disable-next-line` comments. */
class IgnoreCodeActionProvider implements vscode.CodeActionProvider {
  static readonly providedCodeActionKinds = [vscode.CodeActionKind.QuickFix]

  provideCodeActions(
    document: vscode.TextDocument,
    _range: vscode.Range | vscode.Selection,
    context: vscode.CodeActionContext,
  ): vscode.CodeAction[] {
    if (!isSkill(document)) return []

    const actions: vscode.CodeAction[] = []

    for (const diagnostic of context.diagnostics) {
      if (diagnostic.source !== 'slint' && diagnostic.source !== 'slint-model') continue

      const ruleId = ruleIdFromCode(diagnostic.code)
      if (!ruleId) continue

      for (const edit of ignoreEditsForFinding({
        ruleId,
        findingLine: diagnostic.range.start.line,
        documentText: document.getText(),
      })) {
        const action = new vscode.CodeAction(edit.title, vscode.CodeActionKind.QuickFix)
        action.diagnostics = [diagnostic]
        action.isPreferred = edit.kind === 'next-line'
        action.edit = new vscode.WorkspaceEdit()
        action.edit.insert(document.uri, new vscode.Position(edit.insertAtLine, 0), edit.text)
        actions.push(action)
      }
    }

    return actions
  }
}

/**
 * Quick Fix entries that apply one of slint's computed fixes, plus the source action that applies
 * every fix in the document at once.
 *
 * The fixes come from the last published envelope (see `fixableByFile`), matched back to the
 * diagnostic under the cursor by rule id and range. `source.fixAll.slint` is what makes
 * `"editor.codeActionsOnSave": { "source.fixAll.slint": "always" }` work.
 */
class FixCodeActionProvider implements vscode.CodeActionProvider {
  static readonly fixAllKind = vscode.CodeActionKind.SourceFixAll.append('slint')
  static readonly providedCodeActionKinds = [
    vscode.CodeActionKind.QuickFix,
    FixCodeActionProvider.fixAllKind,
  ]

  provideCodeActions(
    document: vscode.TextDocument,
    _range: vscode.Range | vscode.Selection,
    context: vscode.CodeActionContext,
  ): vscode.CodeAction[] {
    if (!isSkill(document)) return []

    const wantsQuickFix = requested(context.only, vscode.CodeActionKind.QuickFix)
    const wantsFixAll = requested(context.only, FixCodeActionProvider.fixAllKind)
    if (!wantsQuickFix && !wantsFixAll) return []

    const actions: vscode.CodeAction[] = []
    const findings = fixableByFile.get(document.uri.fsPath) ?? []
    const text = document.getText()

    if (wantsQuickFix) {
      for (const diagnostic of context.diagnostics) {
        const action = quickFixForDiagnostic(document, diagnostic, findings, text)
        if (action) actions.push(action)
      }
    }

    if (wantsFixAll) {
      const action = fixAllAction(document.uri, text, findings)
      if (action) actions.push(action)
    }

    return actions
  }
}

/** Whether `only` (unset for the lightbulb) asks for a kind this provider serves. */
function requested(only: vscode.CodeActionKind | undefined, kind: vscode.CodeActionKind): boolean {
  return !only || only.intersects(kind)
}

function quickFixForDiagnostic(
  document: vscode.TextDocument,
  diagnostic: vscode.Diagnostic,
  findings: Finding[],
  text: string,
): vscode.CodeAction | undefined {
  if (diagnostic.source !== 'slint' && diagnostic.source !== 'slint-model') return undefined

  const ruleId = ruleIdFromCode(diagnostic.code)
  if (!ruleId) return undefined

  const finding = findings.find((candidate) => {
    if (candidate.rule !== ruleId) return false

    const range = diagnosticRangeForFinding(candidate)
    return (
      range.start.line === diagnostic.range.start.line &&
      range.start.character === diagnostic.range.start.character &&
      range.end.line === diagnostic.range.end.line &&
      range.end.character === diagnostic.range.end.character
    )
  })
  if (!finding) return undefined

  const edit = quickFixEditForFinding(finding, text)
  if (!edit) return undefined

  const action = new vscode.CodeAction(edit.title, vscode.CodeActionKind.QuickFix)
  action.diagnostics = [diagnostic]
  action.isPreferred = true
  action.edit = new vscode.WorkspaceEdit()
  action.edit.replace(
    document.uri,
    new vscode.Range(
      edit.range.start.line,
      edit.range.start.character,
      edit.range.end.line,
      edit.range.end.character,
    ),
    edit.replacement,
  )
  return action
}

/**
 * Applies every computed fix in the document, deferring overlaps exactly like the CLI's fixer so
 * an editor-applied fixAll can never corrupt a file.
 */
function fixAllAction(
  uri: vscode.Uri,
  text: string,
  findings: Finding[],
): vscode.CodeAction | undefined {
  const { applied } = patchText(
    text,
    findings.flatMap((finding) => (finding.fix ? [finding.fix] : [])),
  )

  if (applied.length === 0) return undefined

  const action = new vscode.CodeAction('Apply all slint fixes', FixCodeActionProvider.fixAllKind)
  action.edit = new vscode.WorkspaceEdit()

  for (const fix of applied) {
    const start = positionAtByteOffset(text, fix.start)
    const end = positionAtByteOffset(text, fix.end)
    action.edit.replace(
      uri,
      new vscode.Range(start.line, start.character, end.line, end.character),
      fix.replacement,
    )
  }

  return action
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

  const binary = setting<string>('path', editor.document.uri) ?? 'slint'
  const directory = directoryOf(editor.document)
  const { argv, env } = spawnFor(directory, {
    fix: true,
    model: false,
    resource: editor.document.uri,
  })

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
