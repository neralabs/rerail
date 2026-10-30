import { afterAll, beforeAll, describe, expect, test } from "vitest";
import { execFile, spawn } from "node:child_process";
import { mkdir, mkdtemp, readdir, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { text } from "node:stream/consumers";
import { promisify } from "node:util";
import { openRerailDatabase } from "../src/engine/index.js";
import { RERAIL_EVENT_PROTOCOL_PREFACE } from "../src/runtime/index.js";

/**
 * Going back to the last published release keeps the database: `npx @neralabs/rerail@<previous> setup` reactivates
 * that version against the file this build migrated, with no copy to put back. This holds only while every schema
 * step this build runs leaves the file one the release reads and writes, which a step can break without raising the
 * floor (a NOT NULL column without a default fails the release's inserts), so the release itself is run against it.
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
  test("the release reads and writes a database this build migrated and wrote, and this build opens it after", async () => {
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
    for (const path of [environment.HOME, workspaces.published, workspaces.current]) await mkdir(path, { recursive: true });
    const cli = (packageRoot: string) => async (...args: string[]): Promise<string> =>
      (await run(process.execPath, [join(packageRoot, "dist", "cli", "cli.js"), ...args], { env: environment })).stdout;
    const release = cli(published.packageRoot);
    const current = cli(THIS_BUILD);

    // The release writes the file first, as on a machine that updates from it. This build's writer then runs its
    // steps, as its daemon does when it first starts (a read-only open waits for a step that raises the floor), and
    // its CLI writes a repository of its own.
    await release("start", "--repo", workspaces.published, "--answer", "pick-myself");
    openRerailDatabase(join(environment.RERAIL_HOME, "rerail.sqlite")).$client.close();
    await current("start", "--repo", workspaces.current, "--answer", "pick-myself");

    // Back on the release: it reads both repositories, changes one, exports the library, and records a session.
    expect((await release("state", "--repo", workspaces.current)).trim()).toBe("started");
    await release("stop", "--repo", workspaces.published);
    expect((await release("state", "--repo", workspaces.published)).trim()).toBe("stopped");
    await release("start", "--repo", workspaces.published);
    const dump = join(root, "rules.json");
    await release("rules", "export", "--output", dump);
    expect(JSON.parse(await readFile(dump, "utf8")) as unknown).toHaveProperty("repositories.length", 2);
    const eventId = `compatibility:${published.version}`;
    expect(await sessionStart(published.packageRoot, workspaces.current, environment, eventId)).toMatchObject({ eventId });
    expect(await readdir(environment.RERAIL_STORAGE_ROOT)).not.toHaveLength(0);

    // And forward again: this build still opens what the release wrote.
    expect((await current("state", "--repo", workspaces.published)).trim()).toBe("started");
  }, 60_000);
});

/** One `RERAIL-EVENTS/1` session start through `packageRoot`'s event stream, as a coding agent's hook sends it. */
async function sessionStart(packageRoot: string, cwd: string, env: NodeJS.ProcessEnv, eventId: string): Promise<unknown> {
  const child = spawn(process.execPath, [join(packageRoot, "dist", "harness", "event-stream.js")], { cwd, env, stdio: ["pipe", "pipe", "pipe"] });
  const exited = Promise.withResolvers<number | null>();
  child.once("error", exited.reject);
  child.once("exit", exited.resolve);
  child.stdin.end(`${RERAIL_EVENT_PROTOCOL_PREFACE}\n${JSON.stringify({
    event: { kind: "session-start", eventId, session: { host: "claude-code", sessionId: eventId, cwd } },
  })}\n`);
  const [stdout, stderr, code] = await Promise.all([text(child.stdout), text(child.stderr), exited.promise]);
  expect(code, stderr).toBe(0);
  return JSON.parse(stdout.split("\n")[0] ?? "");
}
