import { afterEach, describe, expect, it } from "vitest";
import { execFile, execFileSync, spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readdir, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { text } from "node:stream/consumers";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";
import { defaultRerailDatabasePath } from "../../src/engine/index.js";
import { RERAIL_EVENT_PROTOCOL_PREFACE } from "../../src/runtime/event-protocol.js";
import { isPreSetupHealthCheck, skipsSessionsApproval } from "../../src/runtime/pre-setup-health-check.js";
import { workspaceStartState } from "../../src/runtime/repository-start.js";

const SOURCE = resolve(import.meta.dirname, "../../src");
// The old check runs the CLI from its scratch folder, where a bare `--import tsx` would not resolve.
const TSX = pathToFileURL(createRequire(import.meta.url).resolve("tsx")).href;

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function scratch(prefix: string): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), prefix));
  temporaryDirectories.push(root);
  return root;
}

/** The event-stream round-trip 0.1.0-next.0's health check makes: one session-start, answered once, then exit. */
async function sessionStart(workspace: string, env: NodeJS.ProcessEnv): Promise<{ code: number | null; line: string; stderr: string }> {
  const child = spawn(process.execPath, ["--import", TSX, join(SOURCE, "harness", "event-stream.ts")], { cwd: workspace, env, stdio: ["pipe", "pipe", "pipe"] });
  const eventId = "health:pre-setup";
  child.stdin.end(`${RERAIL_EVENT_PROTOCOL_PREFACE}\n${JSON.stringify({
    event: { kind: "session-start", eventId, session: { host: "claude-code", sessionId: eventId, cwd: workspace } },
  })}\n`);
  const [stdout, stderr, code] = await Promise.all([
    text(child.stdout),
    text(child.stderr),
    new Promise<number | null>((done, fail) => {
      child.once("error", fail);
      child.once("close", done);
    }),
  ]);
  return { code, line: stdout.split("\n")[0] ?? "", stderr };
}

describe("a release from before set-up checking an update to this one", () => {
  it("starts Rerail from the old health check's bare start, so the session it then opens is recorded", async () => {
    // 0.1.0-next.0's validateManagedInstallation, step for step: its scratch homes, then state, start and a session.
    const root = await scratch("rerail-update-health-");
    const workspace = join(root, "workspace");
    const storageRoot = join(root, "sessions");
    // The old check keeps the person's own HOME; a scratch one with Claude Code in it shows nothing is connected.
    const user = join(root, "user");
    await mkdir(join(user, ".claude"), { recursive: true });
    const env = {
      ...process.env,
      HOME: user,
      USERPROFILE: user,
      RERAIL_HOME: join(root, "home"),
      RERAIL_STORAGE_ROOT: storageRoot,
      RERAIL_AUTOSTART: "0",
    };
    await mkdir(workspace);
    const run = (args: readonly string[]) => promisify(execFile)(
      process.execPath, ["--import", TSX, join(SOURCE, "cli", "cli.ts"), ...args], { cwd: workspace, env, timeout: 30_000 },
    );

    expect(["started", "stopped", "never"]).toContain((await run(["state", "--repo", workspace])).stdout.trim());
    // execFile rejects on a non-zero exit, as the old check does.
    await run(["start", "--repo", workspace]);

    const { code, line, stderr } = await sessionStart(workspace, env);
    expect({ code, stderr }).toEqual({ code: 0, stderr: "" });
    expect(JSON.parse(line)).toMatchObject({ eventId: "health:pre-setup", deliveries: expect.any(Array) });
    expect(await readdir(storageRoot)).not.toEqual([]);

    // Started without set-up: no rule packs, no coding agent connected, no dashboard daemon; set-up runs next time.
    expect(workspaceStartState(workspace, defaultRerailDatabasePath(env))).toBe("not-set-up");
    expect(await readdir(join(user, ".claude"))).toEqual([]);
    expect(existsSync(join(env.RERAIL_HOME, "daemon.json"))).toBe(false);
  }, 60_000);

  it("starts Rerail with no rule packs from a later release's check, fetching none, so its session is recorded", async () => {
    // `startHealthRepository` in updater.ts: state, then `start --answer pick-myself`, in a scratch place with no
    // coding agent found. npm points nowhere: a run that fetched packs would fail to, and ask rather than start.
    const root = await scratch("rerail-update-health-");
    const workspace = join(root, "workspace");
    const storageRoot = join(root, "sessions");
    const user = join(root, "user");
    await mkdir(user);
    const env = {
      ...process.env,
      HOME: user,
      USERPROFILE: user,
      RERAIL_HOME: join(root, "home"),
      RERAIL_STORAGE_ROOT: storageRoot,
      RERAIL_AUTOSTART: "0",
      NPM_CONFIG_REGISTRY: "http://127.0.0.1:9/",
    };
    await mkdir(workspace);
    const run = (args: readonly string[]) => promisify(execFile)(
      process.execPath, ["--import", TSX, join(SOURCE, "cli", "cli.ts"), ...args], { cwd: workspace, env, timeout: 30_000 },
    );

    await run(["start", "--repo", workspace, "--answer", "pick-myself"]);

    const { code, stderr } = await sessionStart(workspace, env);
    expect({ code, stderr }).toEqual({ code: 0, stderr: "" });
    expect(await readdir(storageRoot)).not.toEqual([]);
    expect(workspaceStartState(workspace, defaultRerailDatabasePath(env))).toBe("started");
    expect(existsSync(join(env.RERAIL_HOME, "packs"))).toBe(false);
  }, 60_000);

  it("leaves every other start to set-up", async () => {
    const root = await scratch("rerail-pre-setup-");
    const empty = join(root, "empty");
    const filled = join(root, "filled");
    const repository = join(root, "repository");
    const inRepository = join(repository, "empty");
    await Promise.all([mkdir(empty), mkdir(filled), mkdir(inRepository, { recursive: true })]);
    await writeFile(join(filled, "index.ts"), "export {};\n");
    execFileSync("git", ["init", "-q"], { cwd: repository });
    const env = { RERAIL_HOME: join(root, "home"), RERAIL_STORAGE_ROOT: join(root, "sessions"), RERAIL_AUTOSTART: "0" };

    expect(isPreSetupHealthCheck(["--repo", empty], "agent", empty, env)).toBe(true);
    // A coding agent's start: in a Git repository, at a folder with code to read, or in an ordinary environment.
    expect(isPreSetupHealthCheck(["--repo", inRepository], "agent", inRepository, env)).toBe(false);
    expect(isPreSetupHealthCheck(["--repo", filled], "agent", filled, env)).toBe(false);
    expect(isPreSetupHealthCheck(["--repo", empty], "agent", empty, { ...env, RERAIL_AUTOSTART: undefined })).toBe(false);
    // An answer, another option, or a terminal is set-up's.
    expect(isPreSetupHealthCheck(["--repo", empty, "--answer", "start"], "agent", empty, env)).toBe(false);
    expect(isPreSetupHealthCheck(["--repo", empty, "--repository"], "agent", empty, env)).toBe(false);
    expect(isPreSetupHealthCheck(["--repo", empty], "terminal", empty, env)).toBe(false);
  });
});

describe("an update's health check and the sessions question", () => {
  it("is not asked only in the scratch place with no coding agent found; anywhere an agent is found, it is", async () => {
    const root = await scratch("rerail-update-health-");
    const empty = join(root, "empty");
    const filled = join(root, "filled");
    await Promise.all([mkdir(empty), mkdir(filled)]);
    await writeFile(join(filled, "index.ts"), "export {};\n");
    const env = { RERAIL_HOME: join(root, "home"), RERAIL_STORAGE_ROOT: join(root, "sessions"), RERAIL_AUTOSTART: "0" };
    const check = ["--repo", empty, "--answer", "pick-myself"];
    const none = () => [];

    expect(skipsSessionsApproval(check, "agent", empty, none, env)).toBe(true);
    // The same check on a computer with a coding agent would connect it: it asks.
    expect(skipsSessionsApproval(check, "agent", empty, () => ["claude-code"], env)).toBe(false);
    // Any other answer, folder, lane or environment is set-up's, which asks.
    expect(skipsSessionsApproval(["--repo", empty, "--answer", "start"], "agent", empty, none, env)).toBe(false);
    expect(skipsSessionsApproval(["--repo", filled, "--answer", "pick-myself"], "agent", filled, none, env)).toBe(false);
    expect(skipsSessionsApproval(check, "terminal", empty, none, env)).toBe(false);
    expect(skipsSessionsApproval(check, "agent", empty, none, { ...env, RERAIL_AUTOSTART: undefined })).toBe(false);
  });
});
