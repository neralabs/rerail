import Database from "better-sqlite3";
import { access, mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnPty } from "./process.js";
import type { ManagedProcess, Sandbox } from "./process.js";

export const REAL_AGENT_LIFETIME_MS = 180_000;
const MIN_CREDENTIAL_LIFETIME_MS = REAL_AGENT_LIFETIME_MS + 30_000;

interface AccessSnapshot {
  access: string;
  accountId: string;
  expires: number;
}

function accessSnapshot(value: unknown): AccessSnapshot | undefined {
  if (typeof value !== "object" || value === null) return;
  const candidate = value as Record<string, unknown>;
  if (
    typeof candidate.access !== "string" ||
    !candidate.access ||
    typeof candidate.accountId !== "string" ||
    !candidate.accountId.trim() ||
    typeof candidate.expires !== "number" ||
    !Number.isFinite(candidate.expires)
  )
    return;
  try {
    const payload: unknown = JSON.parse(Buffer.from(candidate.access.split(".")[1] ?? "", "base64url").toString());
    if (
      typeof payload !== "object" ||
      payload === null ||
      !("exp" in payload) ||
      typeof payload.exp !== "number" ||
      !Number.isFinite(payload.exp)
    )
      return;
    const expires = Math.min(candidate.expires, payload.exp * 1_000);
    if (expires < Date.now() + MIN_CREDENTIAL_LIFETIME_MS) return;
    return { access: candidate.access, accountId: candidate.accountId, expires };
  } catch {
    return;
  }
}

function readAccessSnapshot(hostHome: string): AccessSnapshot {
  let db: Database.Database | undefined;
  try {
    db = new Database(join(hostHome, ".omp", "agent", "agent.db"), { readonly: true });
    const rows = db
      .prepare<[], { data: string }>(
        "SELECT data FROM auth_credentials WHERE provider = 'openai-codex' AND credential_type = 'oauth' AND disabled_cause IS NULL",
      ).all();
    for (const row of rows) {
      try {
        const snapshot = accessSnapshot(JSON.parse(row.data));
        if (snapshot) return snapshot;
      } catch {
        /* A malformed row cannot authorize a request. */
      }
    }
  } catch {
    // Never propagate database diagnostics or credential JSON to browser events.
    throw new Error(
      "Real agent prerequisite: could not read the existing OMP login database read-only. Log in outside the playground first.",
    );
  } finally {
    db?.close();
  }
  throw new Error(
    "Real agent prerequisite: an active openai-codex OAuth login valid for at least 210 seconds is required. Renew it outside the playground; this playground never refreshes credentials.",
  );
}

/** Refuse a paid request unless a real PTY child can run the terminal and cannot write outside its sandbox. */
async function proveWriteBoundary(sandbox: Sandbox, profile: string): Promise<void> {
  const outside = await mkdtemp(join(tmpdir(), "playground-denial-probe-"));
  const probe = join(sandbox.root, "write-boundary-probe.mjs");
  let child: ManagedProcess | undefined;
  let timer: NodeJS.Timeout | undefined;
  try {
    await writeFile(
      probe,
      [
        'import { writeFileSync } from "node:fs";',
        "process.stdin.setRawMode(true); process.stdin.setRawMode(false);",
        'writeFileSync(process.argv[2], "inside", { flag: "wx" });',
        'try { writeFileSync(process.argv[3], "outside", { flag: "wx" }); process.exit(2); }',
        'catch (error) { const code = error.code; process.exit(code === "EPERM" || code === "EACCES" ? 0 : 3); }',
      ].join("\n"),
      { mode: 0o600 },
    );
    child = await spawnPty(
      ["/usr/bin/sandbox-exec", "-f", profile, process.execPath, probe,
        join(sandbox.root, "write-boundary-inside"), join(outside, "denied")],
      { cwd: sandbox.cwd, env: sandbox.env, cols: 80, rows: 24,
        data() { /* Probe output is deliberately discarded. */ } },
    );
    const code = await Promise.race([
      child.child.exited,
      new Promise<never>((_resolve, reject) => {
        timer = setTimeout(
          () => reject(new Error("Real agent prerequisite: OS write-boundary probe timed out.")),
          5_000,
        );
      }),
    ]);
    if (code !== 0)
      throw new Error(
        "Real agent prerequisite: macOS sandbox did not prove terminal support and outside-write denial. No provider request was started.",
      );
  } finally {
    clearTimeout(timer);
    try {
      await child?.stop();
    } finally {
      await Promise.all([
        rm(outside, { recursive: true, force: true }),
        rm(probe, { force: true }),
        rm(join(sandbox.root, "write-boundary-inside"), { force: true }),
      ]);
    }
  }
}

/** Every replay gets an OS write boundary; only authenticated modes receive an access snapshot. */
export async function prepareReplay(sandbox: Sandbox, hostHome: string, needsAuth: boolean): Promise<string> {
  if (process.platform !== "darwin")
    throw new Error("Real agent prerequisite: macOS sandbox-exec is required; no unsandboxed fallback is allowed.");
  try {
    await access("/usr/bin/sandbox-exec");
  } catch {
    throw new Error("Real agent prerequisite: /usr/bin/sandbox-exec is unavailable.");
  }
  const root = await realpath(sandbox.root);
  const profile = join(root, "replay.sb");
  await writeFile(
    profile,
    [
      "(version 1)",
      "(deny default)",
      "(allow process*)",
      "(allow sysctl-read)",
      "(allow mach-lookup)",
      ...(needsAuth ? ["(allow network*)"] : []),
      ...(sandbox.env.RERAIL_PLAYGROUND_DAEMON_URL ? [
        // sandbox-exec accepts only `localhost` or `*` as the host here; the daemon binds 127.0.0.1.
        `(allow network-outbound (remote ip "localhost:${new URL(sandbox.env.RERAIL_PLAYGROUND_DAEMON_URL).port}"))`,
      ] : []),
      `(allow network-outbound (literal ${JSON.stringify(join(root, "worker.sock"))}))`,
      "(allow file-read*)",
      `(allow file-write* (subpath ${JSON.stringify(root)}))`,
      '(allow file-write* (literal "/dev/null") (literal "/dev/tty") (regex #"^/dev/ttys[0-9]+$"))',
      '(allow file-ioctl (literal "/dev/tty") (regex #"^/dev/ttys[0-9]+$"))',
    ].join("\n"),
    { mode: 0o600 },
  );
  await proveWriteBoundary(sandbox, profile);
  if (needsAuth) {
    const snapshot = readAccessSnapshot(hostHome);
    const home = join(sandbox.cwd, ".playground-home");
    await mkdir(home, { recursive: true, mode: 0o700 });
    await writeFile(join(home, "real-agent-auth.json"), JSON.stringify(snapshot), { mode: 0o600, flag: "wx" });
  }
  return profile;
}
