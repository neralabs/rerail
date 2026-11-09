import { afterAll, beforeAll, describe, expect, test } from "vitest";
import { execFile, spawn } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { text } from "node:stream/consumers";
import { promisify } from "node:util";
import Database from "better-sqlite3";
import { openRerailDatabase } from "../src/engine/index.js";
import { RERAIL_EVENT_PROTOCOL_PREFACE } from "../src/runtime/index.js";

/**
 * Going back to the last published release (`npx @neralabs/rerail@<version> setup`) runs it against the file this
 * build migrated. A schema step this build runs either keeps the floor, and the release must then go on reading and
 * writing the file, or raises it, and the release must then refuse the file instead of misreading it. A step that
 * breaks the release without raising the floor (a NOT NULL column without a default fails its inserts) fails here.
 */
const run = promisify(execFile);
const THIS_BUILD = resolve(".");
const PACKAGE = "@neralabs/rerail";

let root: string;
let published: { version: string; packageRoot: string };

beforeAll(async () => {
  root = await mkdtemp(join(tmpdir(), "rerail-database-compatibility-"));
  const version = (await run("npm", ["view", `${PACKAGE}@latest`, "version"])).stdout.trim();
  const prefix = join(root, "published");
  await run("npm", ["install", "--prefer-offline", "--prefix", prefix, "--no-audit", "--no-fund", `${PACKAGE}@${version}`]);
  published = { version, packageRoot: join(prefix, "node_modules", ...PACKAGE.split("/")) };
}, 120_000);

afterAll(async () => {
  await rm(root, { recursive: true, force: true, maxRetries: 5 });
});

describe("database compatibility with the last published release", () => {
  test("the release keeps using a database this build migrated, or refuses it when this build raised the floor", async () => {
    const workspaces = { published: join(root, "ws-published"), current: join(root, "ws-current") };
    const environment = {
      ...process.env,
      HOME: join(root, "user"),
      USERPROFILE: join(root, "user"),
      BROWSER: "none",
      RERAIL_HOME: join(root, "home"),
      RERAIL_STORAGE_ROOT: join(root, "sessions"),
      RERAIL_AUTOSTART: "0",
      RERAIL_DEVICE_SECRET_STORE: "file",
    };
    const database = join(environment.RERAIL_HOME, "rerail.sqlite");
    for (const path of [environment.HOME, workspaces.published, workspaces.current]) await mkdir(path, { recursive: true });
    const cli = (packageRoot: string) => async (...args: string[]): Promise<string> =>
      (await run(process.execPath, [join(packageRoot, "dist", "cli", "cli.js"), ...args], { env: environment })).stdout;
    const release = cli(published.packageRoot);
    const current = cli(THIS_BUILD);
    const sessionStart = async (cwd: string, sessionId: string): Promise<void> => {
      await startSession(published.packageRoot, cwd, environment, sessionId);
      // The event stream answers even when it could not record, so the row is what shows the release wrote.
      const row = readDatabase(database, client => client.prepare("SELECT session_id FROM sessions WHERE session_id = ?").get(sessionId));
      expect(row, `the release ${published.version} did not record session ${sessionId}`).toEqual({ session_id: sessionId });
    };

    // The release writes the file first, as on a machine that updates from it: a repository and a session.
    await release("start", "--repo", workspaces.published, "--answer", "pick-myself");
    await sessionStart(workspaces.published, `release-before:${published.version}`);
    const releaseFloor = readDatabase(database, client => client.pragma("user_version", { simple: true }));
    // This build's writer runs its steps over that, as its daemon does when it first starts (a read-only open waits
    // for a step that raises the floor), then its CLI writes a repository of its own.
    openRerailDatabase(database).$client.close();
    const floor = readDatabase(database, client => client.pragma("user_version", { simple: true }));
    await current("start", "--repo", workspaces.current, "--answer", "pick-myself");

    if (floor !== releaseFloor) {
      // A declared break: the release must refuse the file, not misread it. Going back costs what changed since.
      await expect(release("state", "--repo", workspaces.published)).rejects.toMatchObject({
        stderr: expect.stringContaining(`was written by a newer Rerail (schema v${String(floor)})`),
      });
    } else {
      // No declared break: the release reads both repositories, changes one, exports the library, records a session.
      expect((await release("state", "--repo", workspaces.current)).trim()).toBe("started");
      await release("stop", "--repo", workspaces.published);
      expect((await release("state", "--repo", workspaces.published)).trim()).toBe("stopped");
      await release("start", "--repo", workspaces.published);
      const dump = join(root, "rules.json");
      await release("rules", "export", "--output", dump);
      expect(JSON.parse(await readFile(dump, "utf8")) as unknown).toHaveProperty("repositories.length", 2);
      await sessionStart(workspaces.current, `release-after:${published.version}`);
    }

    // Either way this build still opens the file.
    expect((await current("state", "--repo", workspaces.published)).trim()).toBe("started");
  }, 60_000);
});

/** `read` over a read-only connection to `path`, closed after. */
function readDatabase<T>(path: string, read: (client: Database.Database) => T): T {
  const client = new Database(path, { readonly: true, fileMustExist: true });
  try {
    return read(client);
  } finally {
    client.close();
  }
}

/** One `RERAIL-EVENTS/1` session start through `packageRoot`'s event stream, as a coding agent's hook sends it. */
async function startSession(packageRoot: string, cwd: string, env: NodeJS.ProcessEnv, sessionId: string): Promise<void> {
  const child = spawn(process.execPath, [join(packageRoot, "dist", "harness", "event-stream.js")], { cwd, env, stdio: ["pipe", "pipe", "pipe"] });
  const exited = Promise.withResolvers<number | null>();
  child.once("error", exited.reject);
  child.once("exit", exited.resolve);
  child.stdin.end(`${RERAIL_EVENT_PROTOCOL_PREFACE}\n${JSON.stringify({
    event: { kind: "session-start", eventId: sessionId, session: { host: "claude-code", sessionId, cwd } },
  })}\n`);
  const [stdout, stderr, code] = await Promise.all([text(child.stdout), text(child.stderr), exited.promise]);
  expect(code, stderr).toBe(0);
  expect(JSON.parse(stdout.split("\n")[0] ?? "") as unknown).toHaveProperty("eventId", sessionId);
}
