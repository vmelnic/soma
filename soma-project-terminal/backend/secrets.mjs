// Secrets vault — AES-256-GCM encrypted key-value store in postgres.
//
// Uses the crypto port for encryption/decryption and the postgres
// port for storage. The encryption key comes from SOMA_SECRETS_KEY
// env var (32-byte hex). If not set, all operations return null.

const SECRETS_KEY = process.env.SOMA_SECRETS_KEY || "";

export function secretsEnabled() {
  return SECRETS_KEY.length >= 32;
}

export async function ensureSecretsTable(soma, namespace) {
  await soma.invokePort("postgres", "execute", {
    sql:
      `CREATE TABLE IF NOT EXISTS ${namespace}_secrets (` +
      `  name TEXT PRIMARY KEY,` +
      `  encrypted_value TEXT NOT NULL,` +
      `  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),` +
      `  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()` +
      `)`,
  });
}

export async function storeSecret(soma, namespace, name, value) {
  if (!secretsEnabled()) throw new Error("SOMA_SECRETS_KEY not configured");

  // Encrypt the value
  const encrypted = await soma.invokePort("crypto", "aes_encrypt", {
    data: value,
    key: SECRETS_KEY,
  });

  // Upsert into the secrets table
  await soma.invokePort("postgres", "execute", {
    sql:
      `INSERT INTO ${namespace}_secrets (name, encrypted_value, updated_at)` +
      ` VALUES ($1, $2, NOW())` +
      ` ON CONFLICT (name) DO UPDATE SET encrypted_value = $2, updated_at = NOW()`,
    params: [name, JSON.stringify(encrypted)],
  });

  return true;
}

export async function getSecret(soma, namespace, name) {
  if (!secretsEnabled()) return null;

  const result = await soma.invokePort("postgres", "query", {
    sql: `SELECT encrypted_value FROM ${namespace}_secrets WHERE name = $1`,
    params: [name],
  });

  if (!result.rows || result.rows.length === 0) return null;

  const encrypted = JSON.parse(result.rows[0].encrypted_value);

  // Decrypt
  const decrypted = await soma.invokePort("crypto", "aes_decrypt", {
    ...encrypted,
    key: SECRETS_KEY,
  });

  return decrypted.data || decrypted.plaintext || decrypted;
}

export async function deleteSecret(soma, namespace, name) {
  if (!secretsEnabled()) return false;

  const result = await soma.invokePort("postgres", "execute", {
    sql: `DELETE FROM ${namespace}_secrets WHERE name = $1`,
    params: [name],
  });

  return (result.rows_affected || 0) > 0;
}

export async function listSecrets(soma, namespace) {
  if (!secretsEnabled()) return [];

  const result = await soma.invokePort("postgres", "query", {
    sql: `SELECT name, created_at, updated_at FROM ${namespace}_secrets ORDER BY name`,
  });

  return result.rows || [];
}
