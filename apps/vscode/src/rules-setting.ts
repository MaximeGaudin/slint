/**
 * The `slint.rules` setting, translated into the CLI flags the binary already parses.
 *
 * Kept free of the vscode module so unit tests can run under plain Node.
 */

export const RULE_SEVERITIES = ['off', 'error', 'warning', 'info'] as const

export type RuleSeverity = (typeof RULE_SEVERITIES)[number]

/** Rule ids are `area/thing`, exactly as a config file writes them. */
const RULE_ID = /^[a-z0-9][a-z0-9-]*\/[a-z0-9][a-z0-9-]*$/

/**
 * Turns `{ "body/posix-paths": "off" }` into `--rule body/posix-paths=off`.
 *
 * Values the severity enum could not have produced, and keys that are not rule ids, are dropped
 * rather than passed on: a stale or hand-edited setting must not make the binary fail the run.
 */
export function ruleOverridesArgv(rules: Record<string, string> | undefined): string[] {
  if (!rules) return []

  const argv: string[] = []

  for (const name of Object.keys(rules).sort()) {
    const severity = rules[name]
    if (!RULE_ID.test(name)) continue
    if (typeof severity !== 'string') continue
    if (!RULE_SEVERITIES.includes(severity as RuleSeverity)) continue

    argv.push('--rule', `${name}=${severity}`)
  }

  return argv
}
