/**
 * Pure helpers for the workspace-trust gate.
 *
 * Kept free of the vscode module so unit tests can run under plain Node.
 */

/**
 * Whether the extension may spawn the slint binary for a workspace.
 *
 * The binary is an external process whose path and arguments can be influenced by the workspace,
 * so it only runs in a trusted workspace. VS Code already keeps the extension inactive there
 * (`untrustedWorkspaces.enabled: false`); this is the in-code guard behind that declaration.
 */
export function mayRunBinary(isTrusted: boolean): boolean {
  return isTrusted
}
