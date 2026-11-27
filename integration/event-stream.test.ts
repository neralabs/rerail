import { afterEach, describe, expect, test } from "vitest";
import { spawn } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync, realpathSync } from "node:fs";
import { mkdir, mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { text } from "node:stream/consumers";
import { z } from "zod";
import { RERAIL_EVENT_PROTOCOL_PREFACE } from "../src/runtime/index.js";
import { linkFolder } from "../test/support/platform/links.js";
import { removeFolder } from "../test/support/platform/remove-folder.js";
import { startTestRepository } from "../test/support/started-repository.js";

const temporaryDirectories: string[] = [];
/** Folders a daemon ran in: removed once nothing of the test runs there, retried while Windows still holds a file. */
const daemonFolders: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).flatMap(path => [
    rm(path, { recursive: true, force: true }),
    rm(`${path}.home`, { recursive: true, force: true }),
    rm(`${path}.workspace`, { recursive: true, force: true }),
  ]));
  await Promise.all(daemonFolders.splice(0).map(async path => await removeFolder(path)));
});

/** The stream runs in a started repository's workspace, beside the storage root it writes. */
async function runFrames(storageRoot: string, frames: readonly string[]): Promise<readonly Record<string, unknown>[]> {
  const { promise, resolve: settle, reject } = Promise.withResolvers<readonly Record<string, unknown>[]>();
  const workspace = `${storageRoot}.workspace`;
  await mkdir(workspace, { recursive: true });
  startTestRepository(workspace, join(`${storageRoot}.home`, "rerail.sqlite"));
  const child = spawn(process.execPath, [resolve("dist/harness/event-stream.js")], {
    cwd: workspace,
    env: { ...process.env, RERAIL_STORAGE_ROOT: storageRoot, RERAIL_HOME: `${storageRoot}.home` },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const stdout: Buffer[] = [];
  const stderr: Buffer[] = [];
  child.stdout.on("data", (chunk) => stdout.push(Buffer.from(chunk)));
  child.stderr.on("data", (chunk) => stderr.push(Buffer.from(chunk)));
  child.once("error", reject);
  child.once("exit", (code) => {
    if (code !== 0) {
      reject(new Error(`event stream exited ${String(code)}: ${Buffer.concat(stderr).toString("utf8")}`));
      return;
    }
    const results = Buffer.concat(stdout)
      .toString("utf8")
      .trim()
      .split("\n")
      .filter(Boolean)
      .map((line) => JSON.parse(line) as Record<string, unknown>);
    settle(results);
  });
  child.stdin.end(`${[RERAIL_EVENT_PROTOCOL_PREFACE, ...frames].join("\n")}\n`);
  return await promise;
}

function frame(event: Readonly<Record<string, unknown>>): string {
  return JSON.stringify({ event });
}

interface Ran {
  code: number | null;
  stdout: string;
  stderr: string;
}

/** `node <cli> <args>` in `cwd`, `input` on its stdin, once it has exited. */
async function runCli(cli: string, args: readonly string[], cwd: string, env: NodeJS.ProcessEnv, input = ""): Promise<Ran> {
  const child = spawn(process.execPath, [cli, ...args], { cwd, env, stdio: ["pipe", "pipe", "pipe"] });
  const exited = new Promise<number | null>((done, fail) => {
    child.once("error", fail);
    child.once("close", done);
  });
  child.stdin.end(input);
  const [stdout, stderr, code] = await Promise.all([text(child.stdout), text(child.stderr), exited]);
  return { code, stdout, stderr };
}

/** The folders of the package an installed Rerail reads beside its `dist/` and `package.json`. */
const PACKAGE_FOLDERS = ["node_modules", "native", "model", "packs", "docs", "hooks", "skills"];

/**
 * This build laid out where set-up installs Rerail, `<home>/versions/<version>/node_modules/@neralabs/rerail`, so it runs
 * as the installed Rerail does (`isMainRerailBuild`): `dist/` and `package.json` copied, since a module is known by its
 * real path, and the package's other folders linked. Returns its CLI.
 */
function installedRerail(home: string): string {
  const packageRoot = resolve(".");
  const { version } = z.object({ version: z.string() }).parse(JSON.parse(readFileSync(join(packageRoot, "package.json"), "utf8")));
  const installed = join(home, "versions", version, "node_modules", "@neralabs", "rerail");
  mkdirSync(installed, { recursive: true });
  cpSync(join(packageRoot, "package.json"), join(installed, "package.json"));
  cpSync(join(packageRoot, "dist"), join(installed, "dist"), { recursive: true });
  for (const folder of PACKAGE_FOLDERS.filter(name => existsSync(join(packageRoot, name)))) linkFolder(join(packageRoot, folder), join(installed, folder));
  return join(installed, "dist", "cli", "cli.js");
}

/** The events of the issue's run: one of a session never started, a session's start, an event, its end, and one after it. */
function sessionFrames(cwd: string, name: string): string {
  const session = { host: "claude-code", sessionId: `started-${name}`, cwd };
  return `${[
    RERAIL_EVENT_PROTOCOL_PREFACE,
    frame({ kind: "user-prompt", eventId: "e1", sessionId: `never-${name}`, text: "hello" }),
    frame({ kind: "session-start", eventId: "e2", session }),
    frame({ kind: "user-prompt", eventId: "e3", sessionId: session.sessionId, text: "hello" }),
    frame({ kind: "session-end", eventId: "e4", sessionId: session.sessionId }),
    frame({ kind: "user-prompt", eventId: "e5", sessionId: session.sessionId, text: "again" }),
  ].join("\n")}\n`;
}

/** What the protocol answers those events: `inactive-session` for the two events of no running session. */
function inactiveSessionAnswers(name: string): unknown[] {
  return [
    { eventId: "e1", error: { code: "inactive-session", message: `Inactive session: never-${name}` } },
    { eventId: "e2", deliveries: [] },
    { eventId: "e3", deliveries: [] },
    { eventId: "e4", deliveries: [] },
    { eventId: "e5", error: { code: "inactive-session", message: `Inactive session: started-${name}` } },
  ];
}

function framesOf(ran: Ran): unknown[] {
  return ran.stdout.trim().split("\n").filter(Boolean).map(line => JSON.parse(line) as unknown);
}

describe("Rerail persistent event stream", () => {
  test("ingests lifecycle events without MCP and restores idempotency after restart", async () => {
    const storageRoot = await mkdtemp(join(tmpdir(), "rerail-event-stream-"));
    temporaryDirectories.push(storageRoot);
    const session = { host: "claude-code", sessionId: "stream-session", cwd: `${storageRoot}.workspace` };

    const firstRun = await runFrames(storageRoot, [
      frame({ kind: "session-start", eventId: "start", session }),
      frame({
        kind: "user-prompt",
        eventId: "prompt",
        sessionId: session.sessionId,
        session,
        text: "Persistent content",
      }),
    ]);
    expect(firstRun[0]).toEqual({ eventId: "start", deliveries: [] });
    expect(firstRun[1]).toEqual(expect.objectContaining({ eventId: "prompt" }));
    z.array(z.object({ requested: z.literal("log"), mechanism: z.literal("observe") }).passthrough())
      .parse(firstRun[1]?.deliveries);

    const replay = await runFrames(storageRoot, [
        frame({
          kind: "user-prompt",
          eventId: "prompt",
          sessionId: session.sessionId,
          session,
          text: "Duplicate replay",
        }),
        frame({ kind: "session-end", eventId: "end", sessionId: session.sessionId, session }),
    ]);
    expect(replay.map(response => response.eventId)).toEqual(["prompt", "end"]);
    z.array(z.object({ requested: z.literal("log"), mechanism: z.literal("observe") }).passthrough())
      .parse(replay[0]?.deliveries);
    expect(replay[1]?.deliveries).toEqual([]);


    const sessionDirectories = (await readdir(storageRoot, { withFileTypes: true }))
      .filter((entry) => entry.isDirectory())
      .map((entry) => entry.name);
    expect(sessionDirectories).toHaveLength(1);
    const ledger = await readFile(join(storageRoot, sessionDirectories[0]!, "events.jsonl"), "utf8");
    const entries = ledger
      .trim()
      .split("\n")
      .map((line) =>
        z.object({ type: z.string(), eventId: z.string().optional() }).passthrough().parse(JSON.parse(line)),
      );
    expect(entries.filter((entry) => entry.type === "event").map((entry) => entry.eventId)).toEqual([
      "start",
      "prompt",
      "end",
    ]);
  }, 30_000);

  test("contains malformed frames without stopping the stream", async () => {
    const storageRoot = await mkdtemp(join(tmpdir(), "rerail-event-errors-"));
    temporaryDirectories.push(storageRoot);
    const valid = frame({
      kind: "session-start",
      eventId: "start",
      session: { host: "codex", sessionId: "s1", cwd: `${storageRoot}.workspace` },
    });
    expect(await runFrames(storageRoot, ["not-json", valid])).toEqual([
      expect.objectContaining({ eventId: null, error: expect.objectContaining({ code: "invalid-event" }) }),
      { eventId: "start", deliveries: [] },
    ]);
  }, 30_000);

  // An installed Rerail's stream hands each event to the daemon, whose runtime throws `InactiveSessionError` for an event
  // of a session it does not hold; a build from a checkout runs that runtime in the stream's own process. A client of
  // either reads the same code, and starts the session or stops sending, where `invalid-event` says its frame was wrong.
  test("answers an event of a session not started as inactive-session through an installed Rerail and its daemon, as a checkout's build does", async () => {
    const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-event-installed-")));
    daemonFolders.push(root);
    const home = join(root, "home");
    const userHome = join(root, "user");
    const workspace = join(root, "workspace");
    for (const folder of [home, userHome, workspace]) await mkdir(folder);
    startTestRepository(workspace, join(home, "rerail.sqlite"));
    // A user home of its own: the installed Rerail keeps the machine's content store there. No event starts a daemon.
    const env: NodeJS.ProcessEnv = { ...process.env, RERAIL_HOME: home, HOME: userHome, USERPROFILE: userHome, BROWSER: "none", RERAIL_AUTOSTART: "0", RERAIL_EXPERIMENTAL_EVENT_STREAM: "1" };
    delete env.RERAIL_ENGINE_SOCKET;
    const installed = installedRerail(home);

    const start = await runCli(installed, ["daemon", "start", "--port", "0"], workspace, env);
    let throughDaemon: Ran;
    try {
      expect(start, start.stderr).toMatchObject({ code: 0 });
      throughDaemon = await runCli(installed, ["events"], workspace, env, sessionFrames(workspace, "installed"));
    } finally {
      await runCli(installed, ["daemon", "stop"], workspace, env);
    }
    const inProcess = await runCli(resolve("dist/cli/cli.js"), ["events"], workspace, env, sessionFrames(workspace, "checkout"));

    expect(throughDaemon, throughDaemon.stderr).toMatchObject({ code: 0 });
    expect(framesOf(throughDaemon)).toEqual(inactiveSessionAnswers("installed"));
    expect(inProcess, inProcess.stderr).toMatchObject({ code: 0 });
    expect(framesOf(inProcess)).toEqual(inactiveSessionAnswers("checkout"));
  }, 120_000);
});
