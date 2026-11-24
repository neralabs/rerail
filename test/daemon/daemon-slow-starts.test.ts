import { execFileSync, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { existsSync, writeFileSync } from "node:fs";
import { chmod, mkdir, mkdtemp, realpath, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { afterEach, describe, expect, it } from "vitest";
import { bundledModelPath, codeRuleSchema } from "../../src/engine/index.js";
import { DAEMON_STATE_FILE, ingestEngineEvent, readDaemonState, type DaemonState } from "../../src/runtime/index.js";
import { removeFolder } from "../support/platform/remove-folder.js";
import { setUpTestRepository } from "../support/repository-rules.js";
import { daemonProcess, exitMessage, until, watchedEnvironment } from "../support/processes/daemon-home.js";

const ACCESS_HEADER = "x-rerail-access";

const RULE = codeRuleSchema.parse({
  format: "rerail.policy/v1",
  id: "test:needle",
  revision: 1,
  mode: "steer",
  scope: { include: ["src/**/*.py"], exclude: [] },
  clauses: [{
    id: "needle",
    violation: { code: "NEEDLE", anchor: "witness.span", message: "No NEEDLE" },
    forbid: { regex: { pattern: "NEEDLE", source: "file.content" } },
  }],
});

const REAL_GIT = process.platform === "win32" ? "git" : execFileSync("which", ["git"], { encoding: "utf8" }).trim();

/**
 * Which Git the gate holds once armed. On Windows, every start of a Git that does not block: Node makes a process there
 * on the thread that asks, and the gate holds that thread inside Node's start, as a slow process start does. Elsewhere
 * a start returns once the child runs, so the gate holds the run of the reads the worktree's walk makes for Git's
 * verdicts, which do not block either. Git that blocks (`spawnSync`) holds its caller's thread by design, and the gate
 * leaves it alone.
 */
const HELD: readonly string[] = process.platform === "win32" ? [] : ["cat-file"];

interface Harness {
  readonly root: string;
  readonly home: string;
  readonly repo: string;
  readonly gate: string;
  readonly daemon: ChildProcess;
  readonly state: DaemonState;
}

const cleanups: Array<() => Promise<void>> = [];

afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function git(cwd: string, ...args: string[]): void {
  execFileSync(REAL_GIT, ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", ...args], { cwd, stdio: "ignore" });
}

/**
 * A Windows preload that, once `<gate>.arm` exists, holds each start of `git` Node makes for a child that does not block
 * (`ChildProcess.prototype.spawn`, on whichever thread makes it) until `<gate>.release` appears, at most 60 s, and
 * writes `<gate>.held` as it does.
 */
async function writeWindowsGatePreload(home: string, gate: string): Promise<string> {
  const script = join(home, "git-gate.cjs");
  await writeFile(script, `const cp = require("node:child_process");
const fs = require("node:fs");
const [arm, held, release] = ${JSON.stringify([`${gate}.arm`, `${gate}.held`, `${gate}.release`])};
const pause = new Int32Array(new SharedArrayBuffer(4));
const spawnOf = cp.ChildProcess.prototype.spawn;
cp.ChildProcess.prototype.spawn = function (options) {
  const file = String((options && options.file) || "");
  if (/(?:^|[\\\\/])git(?:\\.exe)?$/iu.test(file) && fs.existsSync(arm) && !fs.existsSync(release)) {
    try { fs.writeFileSync(held, "held\\n"); } catch {}
    const limit = Date.now() + 60_000;
    while (!fs.existsSync(release) && Date.now() < limit) Atomics.wait(pause, 0, 0, 10);
  }
  return spawnOf.call(this, options);
};
`);
  return script;
}

/**
 * A POSIX `git` on `PATH` that, once `<gate>.arm` exists, holds each run whose arguments hold every term of
 * {@link HELD} until `<gate>.release` appears, at most 60 s, and writes `<gate>.held` as it does.
 */
async function writePosixGitWrapper(home: string, gate: string): Promise<string> {
  const bin = join(home, "bin");
  await mkdir(bin, { recursive: true });
  const wrapper = join(bin, "git");
  const checks = HELD.map(term => `case "$*" in *${JSON.stringify(term)}*) ;; *) match=0 ;; esac`).join("\n");
  await writeFile(wrapper, `#!/bin/sh
if [ -f ${JSON.stringify(`${gate}.arm`)} ] && [ ! -f ${JSON.stringify(`${gate}.release`)} ]; then
  match=1
${checks}
  if [ "$match" = 1 ]; then
    printf 'held\\n' > ${JSON.stringify(`${gate}.held`)}
    waited=0
    while [ ! -f ${JSON.stringify(`${gate}.release`)} ] && [ "$waited" -lt 3000 ]; do
      sleep 0.02
      waited=$((waited + 1))
    done
  fi
fi
exec ${JSON.stringify(REAL_GIT)} "$@"
`);
  await chmod(wrapper, 0o755);
  return bin;
}

/** The daemon's environment with the system's Git gate in it. */
async function gatedEnvironment(home: string, gate: string): Promise<NodeJS.ProcessEnv> {
  const env = watchedEnvironment(home);
  if (process.platform === "win32") {
    const preload = await writeWindowsGatePreload(home, gate);
    env.NODE_OPTIONS = [env.NODE_OPTIONS, `--require ${JSON.stringify(preload.replaceAll("\\", "/"))}`].filter(Boolean).join(" ");
    return env;
  }
  env.PATH = [await writePosixGitWrapper(home, gate), env.PATH].filter(Boolean).join(":");
  return env;
}

/** Stops `daemon` once the gate is open, so no thread of it is left held, then removes the test's folder. */
async function stopHarness(daemon: ChildProcess, gate: string, root: string): Promise<void> {
  try {
    writeFileSync(`${gate}.release`, "");
  } catch { /* The folder may be gone already. */ }
  if (daemon.exitCode === null && daemon.signalCode === null) {
    const closed = once(daemon, "close");
    daemon.kill("SIGTERM");
    const ended = await Promise.race([closed.then(() => true), delay(5_000).then(() => false)]);
    if (!ended) {
      daemon.kill("SIGKILL");
      await closed;
    }
  }
  await removeFolder(root);
}

/** A repository with one commit and no remote, and a daemon of this checkout's build serving it behind the Git gate. */
async function startHarness(): Promise<Harness> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-slow-starts-")));
  const [home, repo] = [join(root, "home"), join(root, "repo")];
  const gate = join(home, "gate");
  await mkdir(join(repo, "src"), { recursive: true });
  await mkdir(home, { recursive: true });
  await writeFile(join(repo, "src", "a.py"), "x = 1\n");
  git(repo, "init", "-q", "-b", "main");
  git(repo, "add", ".");
  git(repo, "commit", "-q", "-m", "seed");
  setUpTestRepository(repo, { library: [{ kind: "code", rule: RULE }] }, join(home, "rerail.sqlite"));
  const daemon = daemonProcess(home, await gatedEnvironment(home, gate), ["--port", "0"]);
  cleanups.push(async () => await stopHarness(daemon, gate, root));
  let state: DaemonState | undefined;
  await until(async () => {
    if (daemon.exitCode !== null) throw new Error(`daemon exited (${daemon.exitCode}): ${await exitMessage(daemon)}`);
    state = await readDaemonState(join(home, DAEMON_STATE_FILE));
    return state !== undefined;
  }, 30_000, "daemon state file");
  return { root, home, repo, gate, daemon, state: state! };
}

function endpointOf(state: DaemonState) {
  return { socketPath: state.kernelSocketPath, accessToken: state.accessToken! };
}

function configurationOf(harness: Pick<Harness, "home" | "repo">): NonNullable<Parameters<typeof ingestEngineEvent>[2]> {
  return {
    storageRoot: join(harness.repo, ".rerail", "sessions"),
    modelPath: bundledModelPath(),
    databasePath: join(harness.home, "rerail.sqlite"),
    workspaceRoot: harness.repo,
  };
}

function sessionOf(sessionId: string, repo: string) {
  return { host: "claude-code" as const, sessionId, cwd: repo };
}

/** `work`, or a rejection naming `label` once `ms` pass first. */
async function within<T>(work: Promise<T>, ms: number, label: string): Promise<T> {
  const timer = new AbortController();
  const late = delay(ms, undefined, { signal: timer.signal }).then(() => {
    throw new Error(`${label}: did not answer within ${ms} ms`);
  });
  try {
    return await Promise.race([work, late]);
  } finally {
    timer.abort();
  }
}

/** The daemon's `/healthz`, answered healthy within `timeoutMs`; a probe it leaves unanswered fails naming `label`. */
async function checkHealth(state: DaemonState, timeoutMs: number, label: string): Promise<void> {
  const headers = state.accessToken === undefined ? {} : { [ACCESS_HEADER]: state.accessToken };
  const response = await fetch(new URL("/healthz", state.url), { headers, signal: AbortSignal.timeout(timeoutMs) }).catch((error: unknown) => {
    throw new Error(`${label}: /healthz did not answer within ${timeoutMs} ms`, { cause: error });
  });
  expect(response.status).toBe(200);
  expect((await response.json() as { status: string }).status).toBe("ok");
}

/** Opens session `sessionId` in the repository, sends its first prompt and a turn's end, and waits for its rules to run. */
async function openSession(harness: Harness, sessionId: string): Promise<void> {
  const [endpoint, configuration, session] = [endpointOf(harness.state), configurationOf(harness), sessionOf(sessionId, harness.repo)];
  await ingestEngineEvent(endpoint, { kind: "session-start", eventId: `${sessionId}-start`, session }, configuration);
  await ingestEngineEvent(endpoint, { kind: "user-prompt", eventId: `${sessionId}-prompt`, sessionId, session, text: "Check edits." }, configuration);
  await waitForRulesReady(harness, sessionId);
  await ingestEngineEvent(endpoint, { kind: "turn-end", eventId: `${sessionId}-warm`, sessionId, session, text: "warm" }, configuration);
}

/** Sends Writes that break the rule until one is steered: the session's rules run then. */
async function waitForRulesReady(harness: Harness, sessionId: string): Promise<void> {
  let attempt = 0;
  await until(async () => {
    attempt += 1;
    const toolCallId = `${sessionId}-probe-${attempt}`;
    const file = join(harness.repo, "src", `${sessionId}_probe_${attempt}.py`);
    return (await writeHook({ harness, sessionId, file, toolCallId, content: `NEEDLE = ${attempt}\n`, timeoutMs: 15_000 })).length > 0;
  }, 15_000, `${sessionId} code-rule evaluator ready`);
}

/** A Write on `sessionId` as Claude Code's hooks send it (its call, the file written, its result), each hook within `timeoutMs`. */
async function writeHook(input: {
  readonly harness: Harness;
  readonly sessionId: string;
  readonly file: string;
  readonly toolCallId: string;
  readonly content: string;
  readonly timeoutMs: number;
}): Promise<readonly unknown[]> {
  const { harness, sessionId, file, toolCallId, content, timeoutMs } = input;
  const [endpoint, configuration, session] = [endpointOf(harness.state), configurationOf(harness), sessionOf(sessionId, harness.repo)];
  await within(ingestEngineEvent(endpoint, {
    kind: "call", eventId: `${toolCallId}-call`, sessionId, session, toolCallId, toolName: "Write", input: { file_path: file, content },
  }, configuration), timeoutMs, `${toolCallId} call`);
  await writeFile(file, content);
  return await within(ingestEngineEvent(endpoint, {
    kind: "result", eventId: `${toolCallId}-result`, sessionId, session, toolCallId, toolName: "Write", output: { type: "create", filePath: file, content },
  }, configuration), timeoutMs, `${toolCallId} result`);
}

async function recordedEvent(state: DaemonState, sessionId: string, eventId: string): Promise<boolean> {
  const headers = state.accessToken === undefined ? {} : { [ACCESS_HEADER]: state.accessToken };
  const response = await fetch(new URL(`/api/sessions/${encodeURIComponent(sessionId)}/events`, state.url), { headers, signal: AbortSignal.timeout(5_000) });
  expect(response.status).toBe(200);
  const body = await response.json() as { events: ReadonlyArray<{ readonly eventId: string }> };
  return body.events.some(event => event.eventId === eventId);
}

describe("the daemon while process starts are slow", () => {
  it("answers health probes and another session's hooks while a new worktree's Git reads are held", async () => {
    const harness = await startHarness();
    const configuration = configurationOf(harness);
    const s1 = sessionOf("s1", harness.repo);
    await openSession(harness, "s1");
    await openSession(harness, "s2");
    await checkHealth(harness.state, 2_500, "the probe before the gate");

    // A worktree added with a change in it: s1's next turn walks it and asks Git for its verdicts.
    const wt2 = join(harness.root, "wt2");
    git(harness.repo, "worktree", "add", "-q", "-b", "wt2", wt2);
    await writeFile(join(wt2, "src", "a.py"), "x = 2\n");
    await writeFile(`${harness.gate}.arm`, "");
    const triggered = ingestEngineEvent(endpointOf(harness.state), { kind: "turn-end", eventId: "s1-trigger", sessionId: "s1", session: s1, text: "trigger" }, configuration);
    triggered.catch(() => undefined);

    await until(() => existsSync(`${harness.gate}.held`), 20_000, "a Git read held at the gate");
    await checkHealth(harness.state, 2_500, "the probe while a Git read is held");
    await writeHook({ harness, sessionId: "s2", file: join(harness.repo, "src", "live.py"), toolCallId: "while-held", content: "value = 42\n", timeoutMs: 4_000 });

    await writeFile(`${harness.gate}.release`, "");
    await within(triggered, 20_000, "s1's turn end once the gate opens");
    expect(await recordedEvent(harness.state, "s2", "while-held-result")).toBe(true);
  }, 90_000);
});
