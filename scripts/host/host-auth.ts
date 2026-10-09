import { chmod, mkdir, readFile, realpath, writeFile } from "node:fs/promises";
import { basename, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import Database from "better-sqlite3";
import { z } from "zod";

const oauthSchema = z.object({ access: z.string().min(1), accountId: z.string().optional(), expires: z.number().optional() });
const jwtSchema = z.object({ exp: z.number().finite() });
export type AuthDestination = "omp" | "pi" | "codex";

type AccessCredential = { access: string; refresh: string; expires: number; accountId?: string };
type AuthCandidates = { candidates: unknown[]; idToken?: string };

function requireValidTimeout(timeoutMs: number): void {
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0) throw new Error("Invalid smoke timeout");
}

async function credentialPaths(source: string, destination: string): Promise<{ sourcePath: string; targetPath: string }> {
  await mkdir(dirname(destination), { recursive: true, mode: 0o700 });
  const sourcePath = await realpath(source);
  const targetPath = resolve(await realpath(dirname(destination)), basename(destination));
  if (sourcePath === targetPath) throw new Error("Credential source and sandbox destination must differ");
  return { sourcePath, targetPath };
}

async function readDatabaseCandidates(sourcePath: string): Promise<unknown[]> {
  const candidates: unknown[] = [];
  const db = new Database(sourcePath, { readonly: true, fileMustExist: true });
  try {
    const rows = z.array(z.object({ data: z.string() })).parse(db.prepare(
      "SELECT data FROM auth_credentials WHERE provider = 'openai-codex' AND credential_type = 'oauth' AND disabled_cause IS NULL ORDER BY id",
    ).all());
    for (const row of rows) {
      try { candidates.push(JSON.parse(row.data)); } catch { /* A malformed credential must not hide another valid account. */ }
    }
    return candidates;
  } finally { db.close(); }
}

async function readAuthCandidates(kind: AuthDestination, sourcePath: string): Promise<AuthCandidates> {
  if (kind !== "codex") return { candidates: await readDatabaseCandidates(sourcePath) };
  const auth = z.object({ tokens: z.object({ access_token: z.string(), id_token: z.string().min(1), account_id: z.string().optional() }) })
    .parse(JSON.parse(await readFile(sourcePath, "utf8")));
  return {
    candidates: [{ access: auth.tokens.access_token, accountId: auth.tokens.account_id }],
    // Codex deserializes identity claims from this JWT; it is metadata, not a refresh credential.
    idToken: auth.tokens.id_token,
  };
}

function parseCredential(candidate: unknown, minimumExpiry: number): AccessCredential | undefined {
  const parsed = oauthSchema.safeParse(candidate);
  if (!parsed.success) return undefined;
  try {
    const payload = parsed.data.access.split(".")[1];
    if (!payload) return undefined;
    const jwt = jwtSchema.parse(JSON.parse(Buffer.from(payload, "base64url").toString("utf8")));
    const expires = Math.min(jwt.exp * 1000, parsed.data.expires ?? Infinity);
    if (expires < minimumExpiry) return undefined;
    return {
      access: parsed.data.access,
      refresh: "",
      expires,
      ...(parsed.data.accountId === undefined ? {} : { accountId: parsed.data.accountId }),
    };
  } catch {
    // Never include malformed token bytes in an error.
    return undefined;
  }
}

function selectCredential(candidates: unknown[], now: number, timeoutMs: number): AccessCredential {
  // Include subprocess/setup time, not a fixed ten-minute window that may be shorter than the requested smoke.
  const minimumExpiry = now + timeoutMs + 60_000;
  for (const candidate of candidates) {
    const credential = parseCredential(candidate, minimumExpiry);
    if (credential !== undefined) return credential;
  }
  throw new Error("No enabled OpenAI Codex access token remains valid through the smoke timeout");
}

async function writeCredential(
  kind: AuthDestination,
  targetPath: string,
  credential: AccessCredential,
  idToken: string | undefined,
  now: number,
): Promise<void> {
  // Exclusive creation rejects existing files/symlinks and creates mode 0600 before SQLite ever opens the file.
  await writeFile(targetPath, "", { flag: "wx", mode: 0o600 });
  if (kind === "omp") {
    const db = new Database(targetPath);
    try {
      db.exec(`CREATE TABLE auth_credentials (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        provider TEXT NOT NULL, credential_type TEXT NOT NULL, data TEXT NOT NULL,
        disabled_cause TEXT DEFAULT NULL, identity_key TEXT DEFAULT NULL,
        created_at INTEGER NOT NULL DEFAULT (unixepoch()), updated_at INTEGER NOT NULL DEFAULT (unixepoch())
      )`);
      db.prepare("INSERT INTO auth_credentials (provider, credential_type, data) VALUES (?, ?, ?)")
        .run("openai-codex", "oauth", JSON.stringify(credential));
    } finally { db.close(); }
  } else if (kind === "pi") {
    await writeFile(targetPath, JSON.stringify({ "openai-codex": { type: "oauth", ...credential } }));
  } else {
    await writeFile(targetPath, JSON.stringify({ auth_mode: "chatgpt", OPENAI_API_KEY: null,
      last_refresh: new Date(now).toISOString(), tokens: {
        access_token: credential.access, id_token: idToken, refresh_token: "", account_id: credential.accountId,
      } }));
  }
  await chmod(targetPath, 0o600);
}

/** Seed only the credential table; pinned OMP AuthStorage creates its own cache, leases and identity indexes on opening.
 * Matches pi-ai/auth/sqlite-credential-store.ts's inferred v3 schema and OAuth serializer (type stored in its own column).
 */
export async function importAccessOnlyAuth(kind: AuthDestination, source: string, destination: string, timeoutMs: number): Promise<void> {
  requireValidTimeout(timeoutMs);
  const { sourcePath, targetPath } = await credentialPaths(source, destination);
  const now = Date.now();
  const { candidates, idToken } = await readAuthCandidates(kind, sourcePath);
  const credential = selectCredential(candidates, now, timeoutMs);
  await writeCredential(kind, targetPath, credential, idToken, now);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [kind, source, destination, timeout] = process.argv.slice(2);
  try {
    if ((kind !== "omp" && kind !== "pi" && kind !== "codex") || !source || !destination) throw new Error("Invalid import arguments");
    await importAccessOnlyAuth(kind, source, destination, Number(timeout));
  } catch {
    // Library/JSON errors can contain credential material. The subprocess must emit only a fixed prerequisite error.
    console.error("FAILED prerequisite: access-only credential import failed (missing, expired, malformed, or inaccessible credentials)");
    process.exitCode = 1;
  }
}
