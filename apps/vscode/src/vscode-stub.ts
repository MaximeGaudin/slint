import Module from 'node:module'

/**
 * Minimal stand-in for the `vscode` module so extension.ts can be exercised
 * under plain `node --test` (no @vscode/test-electron). Importing this module
 * patches the CJS loader, so `require('vscode')` inside extension.js resolves
 * to the fakes below. Import it before dynamically importing extension.js.
 */

export class Uri {
  readonly fsPath: string
  constructor(fsPath: string) {
    this.fsPath = fsPath
  }
  static file(fsPath: string): Uri {
    return new Uri(fsPath)
  }
  static parse(value: string): Uri {
    return new Uri(value)
  }
  toString(): string {
    return this.fsPath
  }
}

export class Position {
  readonly line: number
  readonly character: number
  constructor(line: number, character: number) {
    this.line = line
    this.character = character
  }
}

export class Range {
  readonly start: Position
  readonly end: Position
  constructor(startLine: number, startCharacter: number, endLine: number, endCharacter: number) {
    this.start = new Position(startLine, startCharacter)
    this.end = new Position(endLine, endCharacter)
  }
}

export class Diagnostic {
  source = ''
  code: string | number | { value: string | number } | undefined
  tags: unknown[] | undefined
  constructor(
    readonly range: Range,
    readonly message: string,
    readonly severity: number,
  ) {}
}

export const DiagnosticSeverity = { Error: 0, Warning: 1, Information: 2, Hint: 3 } as const

export class WorkspaceEdit {
  readonly inserts: { uri: Uri; line: number; character: number; text: string }[] = []
  readonly replaces: { uri: Uri; range: Range; text: string }[] = []
  insert(uri: Uri, position: Position, text: string): void {
    this.inserts.push({ uri, line: position.line, character: position.character, text })
  }
  replace(uri: Uri, range: Range, text: string): void {
    this.replaces.push({ uri, range, text })
  }
}

export const DiagnosticTag = { Unnecessary: 1, Deprecated: 2 } as const

export class CodeActionKind {
  static readonly QuickFix = new CodeActionKind('QuickFix')
  static readonly Source = new CodeActionKind('Source')
  static readonly SourceFixAll = new CodeActionKind('Source.fixAll')
  constructor(readonly value: string) {}
  append(suffix: string): CodeActionKind {
    return new CodeActionKind(`${this.value}.${suffix}`)
  }
  contains(other: CodeActionKind): boolean {
    return this.value === other.value || other.value.startsWith(`${this.value}.`)
  }
  intersects(other: CodeActionKind): boolean {
    return this.contains(other) || other.contains(this)
  }
}

export class CodeAction {
  diagnostics: Diagnostic[] | undefined
  isPreferred: boolean | undefined
  edit: WorkspaceEdit | undefined
  constructor(
    readonly title: string,
    readonly kind: CodeActionKind,
  ) {}
}

export class OutputChannel {
  readonly lines: string[] = []
  append(line: string): void {
    this.lines.push(line)
  }
  appendLine(line: string): void {
    this.lines.push(line)
  }
  show(): void {}
  hide(): void {}
  dispose(): void {}
}

export class StatusBarItem {
  text = ''
  tooltip: string | undefined
  command: string | undefined
  show(): void {}
  hide(): void {}
  dispose(): void {}
}

export class DiagnosticCollection {
  dispose(): void {}
  set(uri: Uri, items: Diagnostic[]): void {
    state.diagnostics.set(uri.fsPath, items.slice())
  }
  delete(uri: Uri): void {
    state.diagnostics.delete(uri.fsPath)
  }
  clear(): void {
    state.diagnostics.clear()
  }
  get(uri: Uri): Diagnostic[] | undefined {
    return state.diagnostics.get(uri.fsPath)
  }
  forEach(callback: (uri: Uri, items: Diagnostic[]) => void): void {
    for (const [fsPath, items] of state.diagnostics) {
      callback(Uri.file(fsPath), items)
    }
  }
}

export type FakeTextDocument = {
  uri: Uri
  fileName: string
  isDirty: boolean
  getText: () => string
  save: () => Promise<boolean>
}

export type FakeWorkspaceFolder = { uri: Uri }

export const state = {
  settings: {} as Record<string, unknown>,
  diagnostics: new Map<string, Diagnostic[]>(),
  outputChannels: [] as OutputChannel[],
  statusItems: [] as StatusBarItem[],
  commands: new Map<string, () => unknown>(),
  codeActionProviders: [] as unknown[],
  executeCommands: [] as string[],
  messages: [] as string[],
  secrets: new Map<string, string>(),
  inputBoxValue: undefined as string | undefined,
  activeTextEditor: undefined as { document: FakeTextDocument } | undefined,
  workspaceFolders: [] as FakeWorkspaceFolder[],
  textDocuments: [] as FakeTextDocument[],
  saveHandlers: [] as ((document: FakeTextDocument) => void)[],
  changeHandlers: [] as ((event: unknown) => void)[],
  deleteHandlers: [] as ((event: { files: Uri[] }) => void)[],
  renameHandlers: [] as ((event: { files: { oldUri: Uri; newUri: Uri }[] }) => void)[],
}

export function resetForTest(): void {
  state.settings = {}
  state.diagnostics.clear()
  state.outputChannels.length = 0
  state.statusItems.length = 0
  state.commands.clear()
  state.codeActionProviders.length = 0
  state.executeCommands.length = 0
  state.messages.length = 0
  state.secrets.clear()
  state.inputBoxValue = undefined
  state.activeTextEditor = undefined
  state.workspaceFolders.length = 0
  state.textDocuments.length = 0
  state.saveHandlers.length = 0
  state.changeHandlers.length = 0
  state.deleteHandlers.length = 0
  state.renameHandlers.length = 0
}

/** Fake SecretStorage backed by state.secrets, handed to the extension via activate()'s context. */
export const secrets = {
  get: async (key: string) => state.secrets.get(key),
  store: async (key: string, value: string) => {
    state.secrets.set(key, value)
  },
  delete: async (key: string) => {
    state.secrets.delete(key)
  },
}

const fakeVscode = {
  Diagnostic,
  DiagnosticSeverity,
  Position,
  Range,
  Uri,
  CodeAction,
  CodeActionKind,
  WorkspaceEdit,
  StatusBarAlignment: { Left: 1, Right: 2 } as const,
  ConfigurationTarget: { Global: 1, Workspace: 2, WorkspaceFolder: 3 } as const,
  languages: {
    createDiagnosticCollection: (_name: string) => new DiagnosticCollection(),
    registerCodeActionsProvider: (_selector: unknown, provider: unknown) => {
      state.codeActionProviders.push(provider)
      return { dispose() {} }
    },
  },
  window: {
    get activeTextEditor() {
      return state.activeTextEditor
    },
    createOutputChannel: (_name: string) => {
      const channel = new OutputChannel()
      state.outputChannels.push(channel)
      return channel
    },
    createStatusBarItem: (_alignment?: number, _priority?: number) => {
      const item = new StatusBarItem()
      state.statusItems.push(item)
      return item
    },
    showInformationMessage: (message: string) => {
      state.messages.push(message)
      return Promise.resolve(undefined)
    },
    showInputBox: () => Promise.resolve(state.inputBoxValue),
    showErrorMessage: (message: string) => {
      state.messages.push(message)
      return Promise.resolve(undefined)
    },
    showWarningMessage: (message: string, ..._items: string[]) => {
      state.messages.push(message)
      return Promise.resolve(undefined)
    },
  },
  secrets: {
    get: (key: string) => secrets.get(key),
    store: (key: string, value: string) => secrets.store(key, value),
    delete: (key: string) => secrets.delete(key),
  },
  workspace: {
    isTrusted: true,
    getConfiguration: (_section: string) => ({
      get: (key: string) => state.settings[key],
      inspect: () => undefined,
    }),
    findFiles: (): Promise<Uri[]> => Promise.resolve([]),
    getWorkspaceFolder: (uri: Uri) =>
      state.workspaceFolders.find((folder) => folder.uri.fsPath === uri.fsPath),
    onDidSaveTextDocument: (handler: (document: FakeTextDocument) => void) => {
      state.saveHandlers.push(handler)
      return { dispose() {} }
    },
    onDidChangeTextDocument: (handler: (event: unknown) => void) => {
      state.changeHandlers.push(handler)
      return { dispose() {} }
    },
    onDidDeleteFiles: (handler: (event: { files: Uri[] }) => void) => {
      state.deleteHandlers.push(handler)
      return { dispose() {} }
    },
    onDidRenameFiles: (handler: (event: { files: { oldUri: Uri; newUri: Uri }[] }) => void) => {
      state.renameHandlers.push(handler)
      return { dispose() {} }
    },
    get textDocuments() {
      return state.textDocuments
    },
    get workspaceFolders() {
      return state.workspaceFolders
    },
  },
  commands: {
    registerCommand: (id: string, handler: () => unknown) => {
      state.commands.set(id, handler)
      return { dispose() {} }
    },
    executeCommand: (id: string) => {
      state.executeCommands.push(id)
      return Promise.resolve()
    },
  },
}

type LoadableModule = typeof Module & {
  _load: (request: string, parent: Module | undefined, isMain: boolean) => unknown
}

const loadableModule = Module as LoadableModule
const originalLoad = loadableModule._load

loadableModule._load = function load(request: string, parent: Module | undefined, isMain: boolean) {
  if (request === 'vscode') return fakeVscode
  return originalLoad.call(loadableModule, request, parent, isMain)
}
