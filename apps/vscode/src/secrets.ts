/**
 * Pure helpers for migrating the LLM API key out of plaintext settings.
 *
 * Kept free of the vscode module so unit tests can run under plain Node.
 */

export type ApiKeyScope = 'global' | 'workspace' | 'workspaceFolder'

export type ApiKeyMigration = {
  /** The key to store in SecretStorage: the most specific scope that holds one. */
  value: string
  /** Every scope that still holds a plaintext copy, to be cleared afterwards. */
  clear: ApiKeyScope[]
}

export function planApiKeyMigration(
  values: { global?: string; workspace?: string; workspaceFolder?: string },
  stored: string | null,
): ApiKeyMigration | undefined {
  return undefined
}
