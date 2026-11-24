/**
 * A daemon whose Git is slow on its two config asks. A session's first events have the daemon learn which config files
 * the engine's in-process reads take, by asking Rerail's Git two things; here each ask is held until the case lets it
 * go, and every other Git call runs at once. Off Windows a stand-in for Git, first on the daemon's PATH, holds each ask's
 * answer: Git is slow to answer. A daemon on Windows runs only a Git program, never a script, so there a preload in the
 * daemon holds each ask's start: Git is slow to start, as every process start is on a busy machine. The session's
 * hooks must not wait on them: each hook has its own time limit, and where Git is slow the two asks alone outlast it.
 */
import { execFileSync } from "node:child_process";
import { accessSync, chmodSync, constants, existsSync, readFileSync, writeFileSync } from "node:fs";
import { mkdir, mkdtemp, realpath, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { type AdapterEvent, defaultRerailDatabasePath } from "../../../src/engine/index.js";
import { startDaemon, stopDaemon } from "../../../src/runtime/daemon-control.js";
import { ingestEngineEvent, SqliteSessionReader } from "../../../src/runtime/index.js";
import { resolveSessionPaths } from "../../../src/runtime/session-paths.js";
import { packageRoot, until } from "../../support/processes/daemon-home.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { setUpTestRepository } from "../../support/repository-rules.js";

const folders: string[] = [];
const homes: string[] = [];
afterEach(async () => {
  for (const home of homes.splice(0)) await stopDaemon(home).catch(() => undefined);
  await Promise.all(folders.splice(0).map(async folder => await removeFolder(folder)));
});

/** A rule the daemon checks at a Bash call itself, and tells in that call's answer. */
const forcePushRule = {
  id: "test:slow-git-force-push",
  mode: "steer",
  match: { field: "input.command", regex: String.raw`\bgit\s+push\b[^\n]*--force\b` },
  sample: { source: "tool:Bash", boundary: "tool-call" },
  action: { content: "Ask before force-pushing." },
  repeat: { mode: "always" },
} as const;

/** Each of a session's first events answers within this, while Git holds the asks: a hook's limit is a few seconds. */
const HOOK_ANSWER_MS = 5_000;

async function folder(prefix: string): Promise<string> {
  const made = await realpath(await mkdtemp(join(tmpdir(), prefix)));
  folders.push(made);
  return made;
}

/**
 * The two asks held until `release` exists: `asks` gets a line as each reaches Git, `answered` one as each Git has
 * answered, and `env` is the daemon's environment that holds them.
 */
interface HeldAsks {
  readonly asks: string;
  readonly answered: string;
  readonly release: string;
  readonly env: NodeJS.ProcessEnv;
}

function heldAsks(tools: string): HeldAsks {
  const files = { asks: join(tools, "asks"), answered: join(tools, "answered"), release: join(tools, "release") };
  return { ...files, env: process.platform === "win32" ? heldStarts(tools, files) : slowGit(tools, files) };
}

/**
 * A stand-in for Git in `tools`, first on the daemon's PATH: a call that names the null device as its Git folder (the
 * two asks, which run outside any repository) adds a line to `asks`, waits until `release` exists, then runs Git and adds
 * a line to `answered`; every other call runs Git at once.
 */
function slowGit(tools: string, files: Omit<HeldAsks, "env">): NodeJS.ProcessEnv {
  const git = realGit();
  const script = [
    "#!/bin/sh",
    `case " $* " in *" --git-dir=/dev/null "*)`,
    `  echo "$$" >> '${files.asks}'`,
    `  while [ ! -e '${files.release}' ]; do sleep 0.05; done`,
    `  '${git}' "$@"; status=$?`,
    `  echo "$$" >> '${files.answered}'`,
    "  exit $status ;;",
    "esac",
    `exec '${git}' "$@"`,
    "",
  ].join("\n");
  writeFileSync(join(tools, "git"), script);
  chmodSync(join(tools, "git"), 0o755);
  return { ...process.env, PATH: `${tools}${delimiter}${process.env.PATH ?? ""}` };
}

/** The Git this process's `PATH` names first, which the stand-in runs. */
function realGit(): string {
  for (const folder of (process.env.PATH ?? "").split(delimiter)) {
    try {
      accessSync(join(folder, "git"), constants.X_OK);
      return join(folder, "git");
    } catch {
      // Not in this folder.
    }
  }
  throw new Error("no git on PATH");
}

/**
 * A preload in the daemon on Windows: a start of a Git that names the null device as its Git folder, on whichever thread
 * makes it and whether or not it blocks (`ChildProcess.prototype.spawn`, `spawnSync`), adds a line to `asks` and waits
 * there until `release` exists, at most 60 s; once that Git has exited it adds a line to `answered`. Every other start
 * runs at once.
 */
function heldStarts(tools: string, files: Omit<HeldAsks, "env">): NodeJS.ProcessEnv {
  const preload = join(tools, "git-asks-gate.cjs");
  writeFileSync(preload, `const cp = require("node:child_process");
const fs = require("node:fs");
const { syncBuiltinESMExports } = require("node:module");
const [asks, answered, release] = ${JSON.stringify([files.asks, files.answered, files.release])};
const pause = new Int32Array(new SharedArrayBuffer(4));
const isAsk = args => Array.isArray(args) && args.some(arg => String(arg).toLowerCase() === "--git-dir=nul");
const hold = () => {
  fs.appendFileSync(asks, process.pid + "\\n");
  const limit = Date.now() + 60_000;
  while (!fs.existsSync(release) && Date.now() < limit) Atomics.wait(pause, 0, 0, 10);
};
const answer = () => fs.appendFileSync(answered, process.pid + "\\n");
const spawnOf = cp.ChildProcess.prototype.spawn;
cp.ChildProcess.prototype.spawn = function (options) {
  if (!isAsk(options && options.args)) return spawnOf.call(this, options);
  hold();
  this.once("exit", answer);
  return spawnOf.call(this, options);
};
const spawnSyncOf = cp.spawnSync;
cp.spawnSync = function (file, args, options) {
  if (!isAsk(args)) return spawnSyncOf.call(this, file, args, options);
  hold();
  try {
    return spawnSyncOf.call(this, file, args, options);
  } finally {
    answer();
  }
};
syncBuiltinESMExports();
`);
  const options = [process.env.NODE_OPTIONS, `--require ${JSON.stringify(preload.replaceAll("\\", "/"))}`];
  return { ...process.env, NODE_OPTIONS: options.filter(Boolean).join(" ") };
}

const lines = (path: string): string[] => (existsSync(path) ? readFileSync(path, "utf8").split("\n").filter(line => line !== "") : []);

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

/** A set-up repository holding `src/x.py`, whose library rule steers a forced push, in the daemon's database. */
async function repository(databasePath: string): Promise<string> {
  const main = join(await folder("rerail-slow-git-repo-"), "main");
  await mkdir(join(main, "src"), { recursive: true });
  await writeFile(join(main, "src", "x.py"), "x = 1\n");
  git(main, "init", "--quiet");
  git(main, "add", ".");
  git(main, "commit", "--quiet", "-m", "seed");
  setUpTestRepository(main, { library: [{ kind: "conversation", rule: forcePushRule }] }, databasePath);
  return main;
}

describe("a daemon whose Git is slow on its config asks", () => {
  it("answers a session's first events while Git holds the asks, and asks no more once Git answers", { timeout: 90_000 }, async () => {
    const home = await folder("rerail-slow-git-home-");
    const tools = await folder("rerail-slow-git-tools-");
    const { asks, answered, release, env: held } = heldAsks(tools);
    const databasePath = defaultRerailDatabasePath({ RERAIL_HOME: home });
    const main = await repository(databasePath);
    // Conversation rules are an experiment, off in every test: on for this daemon, whose check is one of them.
    const env = { ...held, RERAIL_EXPERIMENTAL_CONVERSATION_RULES: "1" };
    homes.push(home);
    const { daemon } = await startDaemon({ home, packageRoot, ensureSqlite: async root => ({ prefix: root, rebuilt: false }) }, { dev: true, port: 0, env });
    const endpoint = { socketPath: daemon.state.kernelSocketPath, ...(daemon.state.accessToken === undefined ? {} : { accessToken: daemon.state.accessToken }) };

    /**
     * A session's first events, as its hooks send them: its start, a forced push the rule steers, told in the call's own
     * answer, and a Write of a file. Gives the rules told at the push, and the longest any event waited for its answer.
     */
    const session = async (sessionId: string, file: string): Promise<{ told: string[]; slowestMs: number }> => {
      const configuration = { ...resolveSessionPaths(main), databasePath, workspaceRoot: main };
      let slowestMs = 0;
      const send = async (event: AdapterEvent): Promise<string[]> => {
        const sent = performance.now();
        const deliveries = await ingestEngineEvent(endpoint, event, configuration);
        slowestMs = Math.max(slowestMs, performance.now() - sent);
        return deliveries.flatMap(delivery => delivery.ruleIds);
      };
      await send({ kind: "session-start", eventId: `${sessionId}-start`, session: { host: "claude-code", sessionId, cwd: main } });
      const told = await send({ kind: "call", eventId: `${sessionId}-push`, sessionId, toolCallId: `push-${sessionId}`, toolName: "Bash", input: { command: "git push --force origin main" } });
      const tool = { sessionId, toolCallId: `write-${sessionId}`, toolName: "Write" };
      await send({ kind: "call", eventId: `${sessionId}-write`, ...tool, input: { file_path: join(main, "src", file), content: "x = 2\n" } });
      await writeFile(join(main, "src", file), "x = 2\n");
      await send({ kind: "result", eventId: `${sessionId}-written`, ...tool, output: "" });
      return { told, slowestMs };
    };
    const reader = new SqliteSessionReader(databasePath);
    try {
      // While Git holds the asks the first session's events started: each is checked and answered without them.
      const first = await session("held", "held.py");
      expect(first.told).toContain(forcePushRule.id);
      expect(first.slowestMs).toBeLessThan(HOOK_ANSWER_MS);
      // The first ask reaches Git once the daemon has started it, which on Windows waits for a starter thread to load.
      await until(() => lines(asks).length > 0, 20_000, "the daemon's first ask reached Git");
      expect(lines(answered)).toEqual([]);
      writeFileSync(release, "");
      await until(() => lines(answered).length === 2, 30_000, "Git answered the two asks");
      // Once Git has answered: a new session's first events are checked, and ask Git nothing more.
      expect((await session("answered", "answered.py")).told).toContain(forcePushRule.id);
      expect(lines(asks)).toHaveLength(2);
      for (const sessionId of ["held", "answered"]) {
        expect(reader.events(sessionId).map(event => event.kind).filter(kind => ["session-start", "call", "result"].includes(kind))).toEqual(["session-start", "call", "call", "result"]);
      }
    } finally {
      reader.close();
      writeFileSync(release, "");
    }
  });
});
