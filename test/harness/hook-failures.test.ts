import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { createServer as createHttpServer, type IncomingMessage, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setImmediate } from "node:timers/promises";
import { afterEach, describe, expect, it, vi } from "vitest";

const autostart = vi.hoisted(() => ({ calls: 0 }));
vi.mock("../../src/runtime/daemon-control.js", async () => {
  const actual = await vi.importActual<typeof import("../../src/runtime/daemon-control.js")>("../../src/runtime/daemon-control.js");
  return { ...actual, autostartDaemon: () => { autostart.calls += 1; } };
});

import { HOOK_FAILURES_FILE, hookFailureLines, readHookFailures, recordHookFailure, type HookFailure, type HookFailureLog } from "../../src/harness/hook-failures.js";
import { DaemonUnreachableError, runHostHook } from "../../src/harness/hook-host.js";
import { listenAsEngine } from "../support/platform/engine-endpoint.js";
import { removeFolder } from "../support/platform/remove-folder.js";
import { sourceModule } from "../support/processes/lock-race.js";

const homes: string[] = [];
async function home(): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), "rerail-hook-failures-"));
  homes.push(path);
  return path;
}

afterEach(async () => {
  vi.unstubAllEnvs();
  autostart.calls = 0;
  await Promise.all(homes.splice(0).map(path => removeFolder(path)));
});

const failure = (at: number, event = "MessageDisplay"): HookFailure => ({
  at, host: "claude-code", event, agent: "claude-code_2-1-311_harness", rerail: "0.1.3", reason: "Claude hook omitted index",
});

/** A log that reaches back over the whole period. */
const whole = (...failures: HookFailure[]): HookFailureLog => ({ failures, keptSince: null });

describe("what rerail status says of hook failures", () => {
  it("says hooks run but nothing was recorded when every failure is newer than the last recorded event", () => {
    expect(hookFailureLines(whole(failure(2_000), failure(3_000)), 1_000)).toEqual([
      "Hooks run but nothing was recorded since 1970-01-01T00:00:02.000Z: 2 hook events failed; the last, MessageDisplay from "
        + "claude-code_2-1-311_harness with Rerail 0.1.3 at 1970-01-01T00:00:03.000Z: Claude hook omitted index.",
    ]);
    expect(hookFailureLines(whole(failure(2_000)), null)[0]).toMatch(/^Hooks run but nothing was recorded since /u);
  });

  it("names the failures without that lead when events were recorded after some of them, and says nothing without any", () => {
    expect(hookFailureLines(whole(failure(2_000), failure(4_000, "Stop")), 3_000)).toEqual([
      "Hooks: 2 hook events could not be recorded in the last 24 hours; the last, Stop from claude-code_2-1-311_harness "
        + "with Rerail 0.1.3 at 1970-01-01T00:00:04.000Z: Claude hook omitted index.",
    ]);
    expect(hookFailureLines(whole(), 3_000)).toEqual([]);
  });

  it("says since when it counts when the log no longer holds the failures before that", () => {
    expect(hookFailureLines({ failures: [failure(2_000), failure(4_000, "Stop")], keptSince: 2_000 }, 3_000)).toEqual([
      "Hooks: 2 hook events could not be recorded since 1970-01-01T00:00:02.000Z, as far back as the log goes; the last, Stop "
        + "from claude-code_2-1-311_harness with Rerail 0.1.3 at 1970-01-01T00:00:04.000Z: Claude hook omitted index.",
    ]);
  });
});

describe("the hook failure log", () => {
  it("keeps each failure with the agent's own name and version, and reads back only those since the given time", async () => {
    const rerailHome = await home();
    recordHookFailure({ host: "claude-code", event: "Stop", reason: "old" }, { home: rerailHome, now: 1_000, env: {} });
    recordHookFailure({ host: "claude-code", event: "MessageDisplay", reason: "new" }, { home: rerailHome, now: 5_000, env: { AI_AGENT: "claude-code_2-1-311_harness" } });
    expect(readHookFailures(rerailHome, 2_000).failures).toEqual([
      expect.objectContaining({ at: 5_000, host: "claude-code", event: "MessageDisplay", agent: "claude-code_2-1-311_harness", reason: "new" }),
    ]);
  });

  it("moves a full log aside instead of growing without end", async () => {
    const rerailHome = await home();
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), `${"x".repeat(300 * 1024)}\n`);
    recordHookFailure({ host: "claude-code", event: "Stop", reason: "after" }, { home: rerailHome, now: 1, env: {} });
    expect(readFileSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`), "utf8").length).toBeGreaterThan(300 * 1024);
    expect(readHookFailures(rerailHome, 0).failures.map(entry => entry.reason)).toEqual(["after"]);
  });

  // A failure of about 100 KB: the log passes its size every third one, and the next moves it aside.
  const recordLarge = (rerailHome: string, at: number): void => {
    recordHookFailure({ host: "claude-code", event: "PostToolUse", reason: `${"x".repeat(100 * 1024)} ${at}` }, { home: rerailHome, now: at, env: {} });
  };
  const recordLast = (rerailHome: string, at: number): void => {
    recordHookFailure({ host: "claude-code", event: "PostToolUse", reason: "the daemon did not answer" }, { home: rerailHome, now: at, env: {} });
  };
  const statusSays = (rerailHome: string, since: number, lastRecordedAt: number): string[] =>
    hookFailureLines(readHookFailures(rerailHome, since), lastRecordedAt);

  it("counts the failures it moved aside with the newest ones, over the whole period when those reach back that far", async () => {
    const rerailHome = await home();
    // 1 s is older than the period; the fourth failure moves the first three aside.
    for (const at of [1_000, 5_000, 6_000]) recordLarge(rerailHome, at);
    recordLast(rerailHome, 7_000);
    expect(existsSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`))).toBe(true);
    expect(statusSays(rerailHome, 2_000, 5_500)).toEqual([expect.stringMatching(
      /^Hooks: 3 hook events could not be recorded in the last 24 hours; the last, PostToolUse from claude-code with Rerail \S+ at 1970-01-01T00:00:07\.000Z: the daemon did not answer\.$/u,
    )]);
  });

  it("says since when it counts once the failures before those it keeps are gone", async () => {
    const rerailHome = await home();
    // The log moves aside at the 4th, 7th and 10th failure; the 1st to 6th are gone.
    for (let second = 1; second <= 9; second += 1) recordLarge(rerailHome, second * 1_000);
    recordLast(rerailHome, 10_000);
    expect(statusSays(rerailHome, 0, 8_500)).toEqual([expect.stringMatching(
      /^Hooks: 4 hook events could not be recorded since 1970-01-01T00:00:07\.000Z, as far back as the log goes; the last, PostToolUse .* at 1970-01-01T00:00:10\.000Z: the daemon did not answer\.$/u,
    )]);
  });

  // A log of failures at 1 ms, just past its size: the next failure finds it full and moves it aside.
  const fullLine = JSON.stringify({ at: 1, host: "claude-code", event: "PostToolUse", agent: null, rerail: "0.1.3", reason: "x".repeat(1_000) });
  const full = Math.ceil((256 * 1024 + 1) / (fullLine.length + 1));
  const writeFullLog = (rerailHome: string): void => {
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), `${fullLine}\n`.repeat(full));
  };

  // A busy daemon fails every hook at once, 25 to 37 at a time on a loaded Windows machine. Each round, every process
  // records one failure into a log already past its size, so each finds it full and moves it aside.
  it("keeps every failure when hooks running at once each find the log full", async () => {
    const processes = 8;
    const rounds = 4;
    const hooks = await startHookProcesses(processes);
    try {
      for (let round = 0; round < rounds; round += 1) {
        const rerailHome = await home();
        writeFullLog(rerailHome);
        await Promise.all(hooks.map((hook, index) => hook.fail(rerailHome, 2 + index)));
        // The full log moved aside whole, and every hook's own failure after it.
        expect({ round, kept: readHookFailures(rerailHome, 0).failures.length }).toEqual({ round, kept: full + processes });
        expect(readFileSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`), "utf8").split("\n").filter(Boolean)).toHaveLength(full);
      }
    } finally {
      await stopHookProcesses(hooks);
    }
  });

  // The files of the log in `rerailHome`, by name.
  const logFiles = (rerailHome: string): string[] => readdirSync(rerailHome).filter(name => name.startsWith(HOOK_FAILURES_FILE)).sort();
  // A hook the coding agent stops the moment it has taken the full log under its own name, as it stops a hook past its
  // time limit.
  const stopAfterTheTake = `rename => (from, to) => {
    rename(from, to);
    if (to.endsWith(".old")) process.kill(process.pid, "SIGKILL");
  }`;

  it("counts the full log a hook was stopped holding as it moved it aside", async () => {
    const rerailHome = await home();
    writeFullLog(rerailHome);
    await recordWithRename(rerailHome, 2, stopAfterTheTake);
    // The log is left under the stopped hook's own name, and the hook's own failure was never written.
    expect(logFiles(rerailHome)).toEqual([expect.stringMatching(/^hook-failures\.jsonl\.\d+\.[0-9a-f-]+\.old$/u)]);
    expect(readHookFailures(rerailHome, 0).failures).toHaveLength(full);
    recordLast(rerailHome, 3);
    expect(readHookFailures(rerailHome, 0).failures).toHaveLength(full + 1);
  });

  it("says since when it counts from the log moved aside, not from an older log a hook was stopped holding", async () => {
    const rerailHome = await home();
    writeFullLog(rerailHome);
    await recordWithRename(rerailHome, 2, stopAfterTheTake);
    // The log moves aside at the 7th and the 10th failure; the 4th to 6th are gone, and the stopped hook's log is older.
    for (let second = 4; second <= 9; second += 1) recordLarge(rerailHome, second * 1_000);
    recordLast(rerailHome, 10_000);
    expect(statusSays(rerailHome, 0, 8_500)).toEqual([expect.stringMatching(
      /^Hooks: 4 hook events could not be recorded since 1970-01-01T00:00:07\.000Z, as far back as the log goes; /u,
    )]);
  });

  // `rerail status` can read the log while a hook moves it aside. The log holds the full log's failures until the hook
  // writes its own, then one more, so every read must count one of those two.
  it("counts every failure once when status reads the log while a hook moves it aside", async () => {
    const hooks = await startHookProcesses(1);
    try {
      for (let round = 0; round < 20; round += 1) {
        const rerailHome = await home();
        writeFullLog(rerailHome);
        let recorded = false;
        const recording = hooks[0]!.fail(rerailHome, 2).then(() => { recorded = true; });
        const counts = new Set<number>();
        while (!recorded) {
          counts.add(readHookFailures(rerailHome, 0).failures.length);
          await setImmediate();
        }
        await recording;
        expect({ round, counts: [...counts].filter(count => count !== full && count !== full + 1) }).toEqual({ round, counts: [] });
        expect(readHookFailures(rerailHome, 0).failures).toHaveLength(full + 1);
      }
    } finally {
      await stopHookProcesses(hooks);
    }
  });
});

interface HookProcess {
  readonly child: ChildProcess;
  /** Records one failure at `at` into `rerailHome`; resolves once it is written. */
  fail(rerailHome: string, at: number): Promise<void>;
}

/** A process that records a hook failure each time it is told to, as a hook process does. */
async function startHookProcess(): Promise<HookProcess> {
  const program = `import { recordHookFailure } from ${JSON.stringify(sourceModule("src/harness/hook-failures.ts"))};
process.on("message", ({ home, at }) => {
  recordHookFailure({ host: "claude-code", event: "PostToolUse", reason: "the daemon did not answer" }, { home, now: at, env: {} });
  process.send("recorded");
});
process.send("ready");
`;
  const child = spawn(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], {
    cwd: join(import.meta.dirname, "..", ".."), stdio: ["ignore", "ignore", "inherit", "ipc"], windowsHide: true,
  });
  const reply = (): Promise<void> => new Promise((resolve, reject) => {
    const onExit = (code: number | null): void => reject(new Error(`the hook process exited (${code}) before it answered`));
    child.once("exit", onExit);
    child.once("message", () => { child.off("exit", onExit); resolve(); });
  });
  await reply();
  return {
    child,
    fail: (rerailHome, at) => {
      const recorded = reply();
      child.send({ home: rerailHome, at });
      return recorded;
    },
  };
}

/** Starts `count` hook processes; fails with the reason of each that did not start, once the others are stopped. */
async function startHookProcesses(count: number): Promise<HookProcess[]> {
  const started = await Promise.allSettled(Array.from({ length: count }, () => startHookProcess()));
  const hooks = started.flatMap(outcome => (outcome.status === "fulfilled" ? [outcome.value] : []));
  const failed = started.flatMap(outcome => (outcome.status === "rejected" ? [String(outcome.reason)] : []));
  if (failed.length > 0) {
    await stopHookProcesses(hooks);
    expect(failed).toEqual([]);
  }
  return hooks;
}

async function stopHookProcesses(hooks: readonly HookProcess[]): Promise<void> {
  await Promise.all(hooks.map(({ child }) => {
    // A process that already exited, such as one that crashed in a round, sends no exit event to wait for.
    if (child.exitCode !== null || child.signalCode !== null) return undefined;
    const exited = once(child, "exit");
    child.kill();
    return exited;
  }));
}

/**
 * Records one failure at `at` into `rerailHome` in a process of its own whose `fs.renameSync` is the function that
 * `wrapRename`, a function's source, makes of the real one: a hook stopped, or a rename refused, at one step of a move.
 * Resolves with what the process wrote to its standard output, once it has ended.
 */
async function recordWithRename(rerailHome: string, at: number, wrapRename: string): Promise<string> {
  const program = `import fs from "node:fs";
import { syncBuiltinESMExports } from "node:module";
fs.renameSync = (${wrapRename})(fs.renameSync);
syncBuiltinESMExports();
const { recordHookFailure } = await import(${JSON.stringify(sourceModule("src/harness/hook-failures.ts"))});
recordHookFailure({ host: "claude-code", event: "PostToolUse", reason: "the daemon did not answer" }, { home: ${JSON.stringify(rerailHome)}, now: ${at}, env: {} });
`;
  const child = spawn(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], {
    cwd: join(import.meta.dirname, "..", ".."), stdio: ["ignore", "pipe", "inherit"], windowsHide: true,
  });
  let output = "";
  child.stdout.setEncoding("utf8").on("data", (chunk: string) => { output += chunk; });
  await once(child, "close");
  return output;
}

describe("a hook that cannot record its event", () => {
  it("logs the event as lost and starts a daemon in the background when none is running", async () => {
    const rerailHome = await home();
    vi.stubEnv("RERAIL_HOME", rerailHome);
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    // A person's session, where hooks may start a daemon; the test run turns that off everywhere else.
    vi.stubEnv("RERAIL_AUTOSTART", "1");
    const handle = vi.fn(async () => ({}));
    expect(await runHostHook("claude-code", rerailHome, "s", false, { storageRoot: join(rerailHome, "sessions") }, handle, "PostToolUse")).toEqual({});
    expect(handle).not.toHaveBeenCalled();
    expect(autostart.calls).toBe(1);
    expect(readHookFailures(rerailHome, 0).failures).toEqual([
      expect.objectContaining({ host: "claude-code", event: "PostToolUse", reason: "no Rerail daemon answered, so this event was not recorded" }),
    ]);
  });

  it.each([
    {
      daemon: "refuses it",
      answer: (_request: IncomingMessage, response: ServerResponse) => { response.writeHead(401).end(); },
      reason: "refused the access capability in daemon.json",
    },
    // A daemon whose one event loop a walk or a long garbage collection holds.
    { daemon: "does not answer", answer: () => undefined, reason: "is running but did not answer within 12 s" },
  ])("logs the event as lost and starts no daemon when the recorded daemon is alive but $daemon", async ({ answer, reason }) => {
    const rerailHome = await home();
    vi.stubEnv("RERAIL_HOME", rerailHome);
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    // A live daemon: this process, holding the engine socket daemon.json names and proving the capability it records,
    // with `answer` at the URL it records.
    const front = createHttpServer(answer);
    const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-alive-${process.pid}-${Date.now()}` : join(rerailHome, "engine.sock");
    await new Promise<void>(resolve => front.listen(0, "127.0.0.1", resolve));
    const closeEngine = await listenAsEngine(socketPath, "token");
    try {
      const port = (front.address() as AddressInfo).port;
      writeFileSync(join(rerailHome, "daemon.json"), JSON.stringify({
        pid: process.pid, host: "127.0.0.1", port, url: `http://127.0.0.1:${port}/`, version: "test", ui: "bundled",
        startedAt: 1, packageRoot: rerailHome, databasePath: join(rerailHome, "rerail.sqlite"), kernelSocketPath: socketPath, accessToken: "token",
      }));
      const handle = vi.fn(async () => ({}));
      // A deadline already reached: the hook gives up after the first probe's 2 s instead of waiting 10 s more.
      const options = { storageRoot: join(rerailHome, "sessions"), daemonDeadline: Date.now() };
      expect(await runHostHook("claude-code", rerailHome, "s", false, options, handle, "PreToolUse")).toEqual({});
      expect(handle).not.toHaveBeenCalled();
      // A start finds the same daemon and refuses to replace it: the background start would cost a process and do nothing.
      expect(autostart.calls).toBe(0);
      expect(readHookFailures(rerailHome, 0).failures).toEqual([expect.objectContaining({
        event: "PreToolUse", reason: `the Rerail daemon (pid ${process.pid}) ${reason}, so this event was not recorded`,
      })]);
    } finally {
      front.closeAllConnections();
      await Promise.all([new Promise(resolve => front.close(resolve)), closeEngine()]);
    }
  });

  // RERAIL_AUTOSTART=0 asks hooks to start no daemon. A hook that then finds none did what the person asked, so it is no
  // failure; `rerail status` says the daemon is not running and that RERAIL_AUTOSTART=0 keeps hooks from starting it.
  it.each([
    ["a later hook", false],
    ["the session-start hook", true],
  ])("logs nothing when %s finds no daemon and autostart is off", async (_name, startsSession) => {
    const rerailHome = await home();
    vi.stubEnv("RERAIL_HOME", rerailHome);
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    vi.stubEnv("RERAIL_AUTOSTART", "0");
    const handle = vi.fn(async () => ({}));
    const run = runHostHook("codex", rerailHome, "s", startsSession, { storageRoot: join(rerailHome, "sessions") }, handle, "SessionStart");
    if (startsSession) await expect(run).rejects.toBeInstanceOf(DaemonUnreachableError);
    else expect(await run).toEqual({});
    expect(handle).not.toHaveBeenCalled();
    expect(autostart.calls).toBe(0);
    expect(existsSync(join(rerailHome, HOOK_FAILURES_FILE))).toBe(false);
  });

  it("logs a failure inside the hook under the event's own name, then fails as before", async () => {
    const rerailHome = await home();
    vi.stubEnv("RERAIL_HOME", rerailHome);
    const options = { storageRoot: join(rerailHome, "sessions"), databasePath: join(rerailHome, "rerail.sqlite"), embedder: { fingerprint: () => ({ modelId: "t", revision: "1", dimensions: 2, weightsSha256: "t", tokenizerSha256: "t", kernelFormatVersion: 1 }), embed: async (texts: readonly string[]) => texts.map(() => new Float32Array([1, 0])) } };
    await expect(runHostHook("claude-code", rerailHome, "s", false, options, async () => { throw new Error("Claude hook omitted index"); }, "MessageDisplay"))
      .rejects.toThrow("Claude hook omitted index");
    expect(readHookFailures(rerailHome, 0).failures).toEqual([expect.objectContaining({ event: "MessageDisplay", reason: "Claude hook omitted index" })]);
  });
});
