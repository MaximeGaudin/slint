/**
 * Pure helpers for migrating the LLM API key out of plaintext settings.
 *
 * Kept free of the vscode module so unit tests can run under plain Node.
 */

export type ApiKeyScope = 'global' | 'workspace' | 'workspaceFolder'

export type ApiKeyMigration = {
  /**
   * The key to store in SecretStorage: the most specific scope that holds one.
   * Absent when a secret is already stored and only the plaintext copies need clearing.
   */
  value?: string
  /** Every scope that still holds a plaintext copy, most specific first, to be cleared. */
  clear: ApiKeyScope[]
}

/**
 * Decide how to move a plaintext key from settings into SecretStorage.
 *
 * The most specific scope holding a value wins, and every scope holding a copy must be cleared
 * afterwards — the point is that no plaintext copy survives, even when the secret already exists.
 */
export function planApiKeyMigration(
  values: { global?: string; workspace?: string; workspaceFolder?: string },
  stored: string | null,
): ApiKeyMigration | undefined {
  const scopes: Array<{ scope: ApiKeyScope; value?: string }> = [
    { scope: 'workspaceFolder', value: values.workspaceFolder?.trim() },
    { scope: 'workspace', value: values.workspace?.trim() },
    { scope: 'global', value: values.global?.trim() },
  ]
  const present = scopes.filter((scope) => Boolean(scope.value))
  if (present.length === 0) return undefined

  const clear = present.map((scope) => scope.scope)

  if (stored !== null) return { clear }

  return { value: present[0].value as string, clear }
}
