/**
 * Pure helpers for the slint invocation built by the extension.
 *
 * Kept free of the vscode module so unit tests can run under plain Node.
 */

/**
 * Flags that steer the model pass. The extension sets them itself from its own settings, so a
 * workspace injecting them through `slint.arguments` could point the model pass (and the API key
 * it carries) at an endpoint of its choosing.
 */
const RESERVED_LLM_FLAGS: ReadonlySet<string> = new Set([
  'llm',
  'enable-llm-rules',
  'no-llm',
  'llm-provider',
  'llm-model',
  'llm-base-url',
  'llm-api-key-env',
])

/** Reserved flags that take a separate value token (`--llm-base-url https://…`). */
const RESERVED_WITH_VALUE: ReadonlySet<string> = new Set([
  'llm-provider',
  'llm-model',
  'llm-base-url',
  'llm-api-key-env',
])

/**
 * Drop reserved model-pass flags from user-supplied arguments, their value tokens included.
 * Everything else (rule overrides, output tuning) passes through untouched.
 */
export function stripReservedLlmArgs(args: string[]): string[] {
  const kept: string[] = []
  let droppingValue = false

  for (const token of args) {
    if (droppingValue) {
      droppingValue = false
      continue
    }

    if (!token.startsWith('--')) {
      kept.push(token)
      continue
    }

    const eq = token.indexOf('=')
    const name = (eq === -1 ? token : token.slice(0, eq)).slice(2)

    if (!RESERVED_LLM_FLAGS.has(name)) {
      kept.push(token)
      continue
    }

    if (eq === -1 && RESERVED_WITH_VALUE.has(name)) droppingValue = true
  }

  return kept
}
