import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, statSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { createServer as createHttpServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

const autostart = vi.hoisted(() => ({ calls: 0 }));
vi.mock("../../src/runtime/daemon-control.js", async () => {
  const actual = await vi.importActual<typeof import("../../src/runtime/daemon-control.js")>("../../src/runtime/daemon-control.js");
  return { ...actual, autostartDaemon: () => { autostart.calls += 1; } };
});

import {
  HOOK_FAILURES_FILE, hookFailureLines, lateCheckLines, readHookFailures, recordHookFailure, recordLateCheck, untakenWaitingLines, type HookFailure, type HookFailureLog,
} from "../../src/harness/hook-failures.js";
import { pendingPath, persistPending, withSessionLock } from "../../src/harness/hook-adapter.js";
import { runClaudeHook } from "../../src/hosts/claude/index.js";
import { startTestRepository } from "../support/started-repository.js";
import { DaemonUnreachableError, hookRerailStarted, runHostHook } from "../../src/harness/hook-host.js";
import { leaveUntaken, LOCKS_DIR, loadNativeBinding, UNTAKEN_EVENTS_DIR, UNTAKEN_MOST_EVENTS, UNTAKEN_WATCH_MARK, untakenFolderId } from "../../src/engine/index.js";
import { daemonLeftEventUntaken, engineRequestBody, parseEngineRequest } from "../../src/runtime/index.js";
import { fakeMachine } from "../support/platform/daemon-machine.js";
import { listenAsEngine } from "../support/platform/engine-endpoint.js";
import { holdHomeAsDaemon } from "../support/processes/home-lock.js";
import { removeFolder } from "../support/platform/remove-folder.js";
import { sourceModule } from "../support/processes/lock-race.js";

const homes: string[] = [];
async function home(): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), "rerail-hook-failures-"));
  homes.push(path);
  return path;
}

// The processes tests started that still run: a test that ends early, as at its time limit, leaves its own, and they end
// before the folders they use are removed.
const running = new Set<ChildProcess>();
function tracked<T extends ChildProcess>(child: T): T {
  running.add(child);
  child.once("exit", () => running.delete(child));
  return child;
}

afterEach(async () => {
  vi.unstubAllEnvs();
  autostart.calls = 0;
  await Promise.all([...running].map(child => {
    const exited = once(child, "exit");
    child.kill();
    return exited;
  }));
  await Promise.all(homes.splice(0).map(path => removeFolder(path)));
});

const failure = (at: number, event = "MessageDisplay"): HookFailure => ({
  at, host: "claude-code", event, agent: "claude-code_2-1-311_harness", rerail: "0.1.3", reason: "Claude hook omitted index",
});

/** A log that reaches back over the whole period. */
const whole = (...failures: HookFailure[]): HookFailureLog => ({ failures, keptSince: null, lateChecks: [] });

describe("what rerail status says of hook failures", () => {
  it("says hooks run but nothing was recorded when every failure is newer than the last recorded event", () => {
    expect(hookFailureLines(whole(failure(2_000), failure(3_000)), 1_000)).toEqual([
      "Hooks run but nothing was recorded since 1970-01-01T00:00:02.000Z: 2 hook events failed; the last, MessageDisplay from "
        + "claude-code_2-1-311_harness with Rerail 0.1.3 at 1970-01-01T00:00:03.000Z: Claude hook omitted index.",
    ]);
    expect(hookFailureLines(whole(failure(2_000)), null)[0]).toMatch(/^Hooks run but nothing was recorded since /u);
  });

  it("names the failures without that lead when events were recorded after all of them, and says nothing without any", () => {
    expect(hookFailureLines(whole(failure(2_000), failure(4_000, "Stop")), 5_000)).toEqual([
      "Hooks: 2 hook events could not be recorded in the last 24 hours; the last, Stop from claude-code_2-1-311_harness "
        + "with Rerail 0.1.3 at 1970-01-01T00:00:04.000Z: Claude hook omitted index.",
    ]);
    expect(hookFailureLines(whole(), 3_000)).toEqual([]);
  });

  it("leads with the failures since the last recorded event, then counts the period, when events were recorded between failures", () => {
    expect(hookFailureLines(whole(failure(2_000), failure(4_000, "Stop"), failure(5_000, "Stop")), 3_000)).toEqual([
      "Hooks run but nothing was recorded since 1970-01-01T00:00:04.000Z: 2 hook events failed, of 3 that could not be "
        + "recorded in the last 24 hours; the last, Stop from claude-code_2-1-311_harness with Rerail 0.1.3 at "
        + "1970-01-01T00:00:05.000Z: Claude hook omitted index.",
    ]);
  });

  it("says since when it counts when the log no longer holds the failures before that", () => {
    const log = { failures: [failure(2_000), failure(4_000, "Stop")], keptSince: 2_000 };
    expect(hookFailureLines(log, 5_000)).toEqual([
      "Hooks: 2 hook events could not be recorded since 1970-01-01T00:00:02.000Z, as far back as the log goes; the last, Stop "
        + "from claude-code_2-1-311_harness with Rerail 0.1.3 at 1970-01-01T00:00:04.000Z: Claude hook omitted index.",
    ]);
    expect(hookFailureLines(log, 3_000)[0]).toMatch(
      /^Hooks run but nothing was recorded since 1970-01-01T00:00:04\.000Z: 1 hook event failed, of 2 that could not be recorded since 1970-01-01T00:00:02\.000Z, as far back as the log goes; the last, Stop /u,
    );
    // Every failure the log holds came after the last recorded event, and hooks may have failed before the oldest.
    expect(hookFailureLines(log, 1_000)[0]).toMatch(
      /^Hooks run but nothing was recorded since 1970-01-01T00:00:02\.000Z, as far back as the log goes: 2 hook events failed; the last, Stop /u,
    );
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

  it("reads past lines that hold no failure, and fills in the fields a failure's line lacks or does not hold as text", async () => {
    const rerailHome = await home();
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), [
      "",
      '{"at":1000,"host":"codex","event":"Stop","reas',
      '{"at":2000,"host":"codex","reason":"no event"}',
      '{"startedAt":2500}',
      '{"at":3000,"event":"Stop","reason":"no host, agent or version"}',
      '{"at":4000,"host":{},"event":"Stop","agent":5,"rerail":["0.1.3"],"reason":"host, agent and version not text"}',
      "",
    ].join("\n"));
    expect(readHookFailures(rerailHome, 0).failures).toEqual([
      { at: 3_000, host: "unknown", event: "Stop", agent: null, rerail: "unknown", reason: "no host, agent or version" },
      { at: 4_000, host: "unknown", event: "Stop", agent: null, rerail: "unknown", reason: "host, agent and version not text" },
    ]);
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
  // An event recorded after every failure below: the line counts them without the lead.
  const afterEveryFailure = 60_000;
  // The number of hook events a line of `rerail status` says could not be recorded; 0 when it says nothing.
  const countOf = (line: string | undefined): number => Number(/^Hooks: (\d+) hook events? could not be recorded /u.exec(line ?? "")?.[1] ?? 0);

  it("counts the failures it moved aside with the newest ones, over the whole period when those reach back that far", async () => {
    const rerailHome = await home();
    // 1 s is older than the period; the fourth failure moves the first three aside.
    for (const at of [1_000, 5_000, 6_000]) recordLarge(rerailHome, at);
    recordLast(rerailHome, 7_000);
    expect(existsSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`))).toBe(true);
    expect(statusSays(rerailHome, 2_000, afterEveryFailure)).toEqual([expect.stringMatching(
      /^Hooks: 3 hook events could not be recorded in the last 24 hours; the last, PostToolUse from claude-code with Rerail \S+ at 1970-01-01T00:00:07\.000Z: the daemon did not answer\.$/u,
    )]);
  });

  // Hooks failed earlier in the period and events were recorded after that; then the daemon stopped answering, and the
  // first failure of that outage moved the earlier failures aside.
  it("leads with an outage that began after the last recorded event once older failures moved aside", async () => {
    const rerailHome = await home();
    for (const at of [1_000, 2_000, 3_000]) recordLarge(rerailHome, at);
    recordLast(rerailHome, 4_000);
    expect(statusSays(rerailHome, 2_000, 3_500)).toEqual([expect.stringMatching(
      /^Hooks run but nothing was recorded since 1970-01-01T00:00:04\.000Z: 1 hook event failed, of 3 that could not be recorded in the last 24 hours; the last, PostToolUse .* at 1970-01-01T00:00:04\.000Z: the daemon did not answer\.$/u,
    )]);
  });

  it("says since when it counts once the failures before those it keeps are gone", async () => {
    const rerailHome = await home();
    // The log moves aside at the 4th, 7th and 10th failure; the 1st to 6th are gone.
    for (let second = 1; second <= 9; second += 1) recordLarge(rerailHome, second * 1_000);
    recordLast(rerailHome, 10_000);
    expect(statusSays(rerailHome, 0, afterEveryFailure)).toEqual([expect.stringMatching(
      /^Hooks: 4 hook events could not be recorded since 1970-01-01T00:00:07\.000Z, as far back as the log goes; the last, PostToolUse .* at 1970-01-01T00:00:10\.000Z: the daemon did not answer\.$/u,
    )]);
  });

  // A hook reads the clock before it writes. One that read it before another hook moved the log aside, and wrote after,
  // puts a failure older than the move in the new part, while the part moved aside holds failures newer than it.
  it("leaves out no failure since the time it names when a hook writes a failure from before the move after it", async () => {
    const rerailHome = await home();
    // The log moves aside at 4 s and at 8 s; the failure at 6.5 s comes after the move at 8 s, and the move at 12 s
    // drops the part that holds the one at 7 s.
    for (const at of [1_000, 2_000, 3_000]) recordLarge(rerailHome, at);
    recordLast(rerailHome, 4_000);
    for (const at of [5_000, 6_000, 7_000]) recordLarge(rerailHome, at);
    recordLast(rerailHome, 8_000);
    recordLast(rerailHome, 6_500);
    for (const at of [9_000, 10_000, 11_000]) recordLarge(rerailHome, at);
    recordLast(rerailHome, 12_000);
    expect(statusSays(rerailHome, 0, afterEveryFailure)).toEqual([expect.stringMatching(
      /^Hooks: 5 hook events could not be recorded since 1970-01-01T00:00:08\.000Z, as far back as the log goes; /u,
    )]);
  });

  const lines = (...ats: number[]): string => ats.map(at => `${JSON.stringify(failure(at))}\n`).join("");

  // The writer before this one moved the full log to `.1` with no start line.
  it("says since when it counts from the oldest failure of a .1 with no start", async () => {
    const rerailHome = await home();
    writeFileSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`), lines(5_000, 6_000));
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), lines(7_000));
    expect(statusSays(rerailHome, 0, afterEveryFailure)).toEqual([expect.stringMatching(
      /^Hooks: 3 hook events could not be recorded since 1970-01-01T00:00:05\.000Z, as far back as the log goes; /u,
    )]);
  });

  it("says since when it counts from the live part's start when .1 cannot be read", async () => {
    const rerailHome = await home();
    // A folder at `.1` cannot be read as a file on any system.
    mkdirSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`));
    // The live part's start, not its oldest failure, is the time from which every failure is in it.
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), `${JSON.stringify({ startedAt: 9_000 })}\n${lines(8_500, 9_500, 10_000)}`);
    expect(statusSays(rerailHome, 0, afterEveryFailure)).toEqual([expect.stringMatching(
      /^Hooks: 2 hook events could not be recorded since 1970-01-01T00:00:09\.000Z, as far back as the log goes; /u,
    )]);
  });

  // A log of failures at 1 ms, just past its size: the next failure finds it full and moves it aside.
  const fullLine = JSON.stringify({ at: 1, host: "claude-code", event: "PostToolUse", agent: null, rerail: "0.1.3", reason: "x".repeat(1_000) });
  const full = Math.ceil((256 * 1024 + 1) / (fullLine.length + 1));
  const writeFullLog = (rerailHome: string): void => {
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), `${fullLine}\n`.repeat(full));
  };
  // The failure lines in the files of the log in `rerailHome`: a line with an event.
  const failuresWritten = (rerailHome: string): number => readdirSync(rerailHome)
    .filter(name => name.startsWith(HOOK_FAILURES_FILE))
    .flatMap(name => readFileSync(join(rerailHome, name), "utf8").split("\n"))
    .filter(line => line.includes('"event":')).length;

  it("leaves the move to the hook that holds its lock, and appends to the full log meanwhile", async () => {
    const rerailHome = await home();
    writeFullLog(rerailHome);
    mkdirSync(join(rerailHome, LOCKS_DIR), { recursive: true });
    const held = loadNativeBinding().tryLockExclusive(join(rerailHome, LOCKS_DIR, "hook-failures.lock"));
    expect(held).not.toBeNull();
    try {
      recordLast(rerailHome, 2);
      expect(existsSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`))).toBe(false);
      expect(readHookFailures(rerailHome, 0).failures).toHaveLength(full + 1);
    } finally {
      held!.release();
    }
    recordLast(rerailHome, 3);
    expect(readFileSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`), "utf8").split("\n").filter(Boolean)).toHaveLength(full + 1);
    expect(readHookFailures(rerailHome, 0).failures).toHaveLength(full + 2);
  });

  // A busy daemon fails every hook at once, 25 to 37 at a time on a loaded Windows machine. Each round, every process
  // records one failure into a log already past its size, so each finds it full; one moves it aside, and the others
  // append to whichever log is live.
  it("keeps every failure when twenty hooks running at once each find the log full", async () => {
    const processes = 20;
    const rounds = 4;
    const hooks = await startHookProcesses(processes);
    try {
      for (let round = 0; round < rounds; round += 1) {
        const rerailHome = await home();
        writeFullLog(rerailHome);
        await Promise.all(hooks.map((hook, index) => hook.fail(rerailHome, 2 + index)));
        // The full log moved aside once, whole, and status counts it with every hook's own failure.
        expect({ round, written: failuresWritten(rerailHome) }).toEqual({ round, written: full + processes });
        expect({ round, counted: countOf(statusSays(rerailHome, 0, afterEveryFailure)[0]) }).toEqual({ round, counted: full + processes });
      }
    } finally {
      await stopHookProcesses(hooks);
    }
  }, 120_000);

  // The log's limits (`hook-failures.ts`): moved aside past 256 KB, and no failure written past 1 MB while it cannot be.
  // Hooks that read its size at the same moment each write one more line: at most a line of ~1.2 KB each here.
  const MOVED_PAST = 256 * 1024;
  const CAP = 1024 * 1024;
  const slack = (processes: number): number => processes * 2 * 1024;
  const sizeOfLog = (rerailHome: string, name = HOOK_FAILURES_FILE): number => statSync(join(rerailHome, name)).size;

  it("keeps both parts of the log within their size when the engine does not load and hooks move it unlocked", async () => {
    const processes = 8;
    const rerailHome = await home();
    const hooks = await startHookProcesses(processes, { RERAIL_ENGINE_PATH: join(rerailHome, "missing.node") });
    try {
      // 8 hooks at once, 400 failures of 1 KB each: 3.7 MB written in all.
      await Promise.all(hooks.map(hook => hook.fail(rerailHome, 2, { times: 400, reasonBytes: 1_024 })));
    } finally {
      await stopHookProcesses(hooks);
    }
    // No hook took the move's lock: each moved the log without it.
    expect(existsSync(join(rerailHome, LOCKS_DIR, "hook-failures.lock"))).toBe(false);
    expect(readdirSync(rerailHome).filter(name => name.startsWith(HOOK_FAILURES_FILE)).sort()).toEqual([HOOK_FAILURES_FILE, `${HOOK_FAILURES_FILE}.1`]);
    expect(sizeOfLog(rerailHome)).toBeLessThanOrEqual(MOVED_PAST + slack(processes));
    expect(sizeOfLog(rerailHome, `${HOOK_FAILURES_FILE}.1`)).toBeLessThanOrEqual(MOVED_PAST + slack(processes));
  }, 120_000);

  it("stops growing the live log at its cap when .1 cannot be replaced", async () => {
    const rerailHome = await home();
    // A folder that holds a file: no system renames a file over it, and the retry gives up at once.
    mkdirSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`));
    writeFileSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`, "kept"), "");
    for (let index = 0; index < 1_500; index += 1) {
      recordHookFailure({ host: "claude-code", event: "PostToolUse", reason: "x".repeat(1_024) }, { home: rerailHome, now: 2, env: {} });
    }
    expect(sizeOfLog(rerailHome)).toBeGreaterThanOrEqual(CAP);
    expect(sizeOfLog(rerailHome)).toBeLessThanOrEqual(CAP + slack(1));
  });

  it("keeps the failure of a hook that found the log at its cap when another hook moved it before this one could", async () => {
    const rerailHome = await home();
    const line = `${JSON.stringify(failure(1))}\n`;
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), line.repeat(Math.ceil(CAP / line.length) + 1));
    expect(sizeOfLog(rerailHome)).toBeGreaterThanOrEqual(CAP);
    // The hook's first look at the size finds the log at its cap; right after it, another hook moves the log to `.1`.
    const program = `import fs from "node:fs";
import { syncBuiltinESMExports } from "node:module";
const stat = fs.statSync;
let first = true;
fs.statSync = (path, ...rest) => {
  const stats = stat(path, ...rest);
  if (first && String(path).endsWith(${JSON.stringify(HOOK_FAILURES_FILE)})) {
    first = false;
    fs.renameSync(path, path + ".1");
  }
  return stats;
};
syncBuiltinESMExports();
// Imported once the wrap is in place, so the module's own statSync is the wrapped one: a static import would load first.
const { recordHookFailure } = await import(${JSON.stringify(sourceModule("src/harness/hook-failures.ts"))});
recordHookFailure({ host: "claude-code", event: "PostToolUse", reason: "kept" }, { home: ${JSON.stringify(rerailHome)}, now: 5, env: {} });
`;
    const child = tracked(spawn(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], {
      cwd: join(import.meta.dirname, "..", ".."), stdio: ["ignore", "ignore", "inherit"], windowsHide: true,
    }));
    const [code] = await once(child, "exit") as [number | null];
    expect(code).toBe(0);
    expect(existsSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`))).toBe(true);
    expect(readHookFailures(rerailHome, 0).failures.filter(entry => entry.reason === "kept")).toHaveLength(1);
  });

  it("counts the log once when a hook moves it to .1 between status reading the live log and .1", async () => {
    const rerailHome = await home();
    const text = (...ats: number[]): string => ats.map(at => `${JSON.stringify(failure(at))}\n`).join("");
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), text(1_000, 2_000));
    // Right after status opens the live log, a hook moves it to `.1` and writes the next part's first failure.
    const program = `import fs from "node:fs";
import { syncBuiltinESMExports } from "node:module";
const open = fs.openSync;
let moved = false;
fs.openSync = (path, ...rest) => {
  const descriptor = open(path, ...rest);
  if (!moved && String(path).endsWith(${JSON.stringify(HOOK_FAILURES_FILE)})) {
    moved = true;
    fs.renameSync(path, path + ".1");
    fs.appendFileSync(path, ${JSON.stringify(text(3_000))});
  }
  return descriptor;
};
syncBuiltinESMExports();
// Imported once the wrap is in place, so the module's own openSync is the wrapped one: a static import would load first.
const { readHookFailures } = await import(${JSON.stringify(sourceModule("src/harness/hook-failures.ts"))});
fs.writeSync(1, JSON.stringify(readHookFailures(${JSON.stringify(rerailHome)}, 0).failures.map(entry => entry.at)));
`;
    const child = tracked(spawn(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], {
      cwd: join(import.meta.dirname, "..", ".."), stdio: ["ignore", "pipe", "inherit"], windowsHide: true,
    }));
    let output = "";
    child.stdout.setEncoding("utf8").on("data", (chunk: string) => { output += chunk; });
    const [code] = await once(child, "close") as [number | null];
    expect(code).toBe(0);
    expect(JSON.parse(output)).toEqual([1_000, 2_000, 3_000]);
  });

  // The system frees a lock once its holder ends, however it ends; here the test holds it for as long as the hooks run.
  it("stops growing the live log at its cap while another process holds the move's lock", async () => {
    const processes = 20;
    const rerailHome = await home();
    mkdirSync(join(rerailHome, LOCKS_DIR), { recursive: true });
    const held = loadNativeBinding().tryLockExclusive(join(rerailHome, LOCKS_DIR, "hook-failures.lock"));
    expect(held).not.toBeNull();
    const hooks = await startHookProcesses(processes);
    try {
      // 20 hooks at once, 100 failures of 1 KB each: 2.3 MB written in all.
      await Promise.all(hooks.map(hook => hook.fail(rerailHome, 2, { times: 100, reasonBytes: 1_024 })));
    } finally {
      held!.release();
      await stopHookProcesses(hooks);
    }
    expect(existsSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`))).toBe(false);
    expect(sizeOfLog(rerailHome)).toBeGreaterThanOrEqual(CAP);
    expect(sizeOfLog(rerailHome)).toBeLessThanOrEqual(CAP + slack(processes));
  }, 120_000);
});

interface HookProcess {
  readonly child: ChildProcess;
  /**
   * Records a failure at `at` into `rerailHome`, `times` times (once by default), each with a reason `reasonBytes` longer
   * than the plain one; resolves once all are written.
   */
  fail(rerailHome: string, at: number, options?: { times?: number; reasonBytes?: number }): Promise<void>;
}

/** A process that records hook failures each time it is told to, as a hook process does, with `env` over this one's. */
async function startHookProcess(env: NodeJS.ProcessEnv = {}): Promise<HookProcess> {
  const program = `import { recordHookFailure } from ${JSON.stringify(sourceModule("src/harness/hook-failures.ts"))};
process.on("message", ({ home, at, times, reasonBytes }) => {
  const reason = "the daemon did not answer" + "x".repeat(reasonBytes);
  for (let index = 0; index < times; index += 1) recordHookFailure({ host: "claude-code", event: "PostToolUse", reason }, { home, now: at, env: {} });
  process.send("recorded");
});
process.send("ready");
`;
  const child = tracked(spawn(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], {
    cwd: join(import.meta.dirname, "..", ".."), env: { ...process.env, ...env }, stdio: ["ignore", "ignore", "inherit", "ipc"], windowsHide: true,
  }));
  const reply = (): Promise<void> => new Promise((resolve, reject) => {
    const onExit = (code: number | null): void => reject(new Error(`the hook process exited (${code}) before it answered`));
    child.once("exit", onExit);
    child.once("message", () => { child.off("exit", onExit); resolve(); });
  });
  await reply();
  return {
    child,
    fail: (rerailHome, at, { times = 1, reasonBytes = 0 } = {}) => {
      const recorded = reply();
      child.send({ home: rerailHome, at, times, reasonBytes });
      return recorded;
    },
  };
}

/** Starts `count` hook processes; fails with the reason of each that did not start, once the others are stopped. */
async function startHookProcesses(count: number, env?: NodeJS.ProcessEnv): Promise<HookProcess[]> {
  const started = await Promise.allSettled(Array.from({ length: count }, () => startHookProcess(env)));
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
    // A live daemon: this process, holding the home, the engine socket daemon.json names and proving the capability it
    // records, with `answer` at the URL it records.
    const front = createHttpServer(answer);
    const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-alive-${process.pid}-${Date.now()}` : join(rerailHome, "engine.sock");
    await new Promise<void>(resolve => front.listen(0, "127.0.0.1", resolve));
    const closeEngine = await listenAsEngine(socketPath, "token");
    const releaseHome = holdHomeAsDaemon(rerailHome);
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
      releaseHome();
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

describe("a hook beside a daemon that runs but does not read", () => {
  /** A port nothing listens on: a probe there is refused at once, as a daemon whose pid lives but which does not answer. */
  async function closedPort(): Promise<number> {
    const server = createHttpServer();
    await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
    const { port } = server.address() as AddressInfo;
    await new Promise(resolve => server.close(resolve));
    return port;
  }

  /**
   * This process as the home's daemon, which watches the home's untaken folder: it holds the home, as a daemon does, so
   * a lookup keeps the record of this daemon that does not answer; its engine socket proves the capability daemon.json
   * records, and `port` is at its URL. Returns what lets go of the home and ends the engine.
   */
  async function recordedDaemon(rerailHome: string, port: number): Promise<() => Promise<void>> {
    vi.stubEnv("RERAIL_HOME", rerailHome);
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-untaken-${process.pid}-${Date.now()}` : join(rerailHome, "engine.sock");
    const closeEngine = await listenAsEngine(socketPath, "token");
    const releaseHome = holdHomeAsDaemon(rerailHome);
    mkdirSync(join(rerailHome, UNTAKEN_EVENTS_DIR));
    // Marked as a daemon marks the folder its watch opens on.
    writeFileSync(join(rerailHome, UNTAKEN_EVENTS_DIR, UNTAKEN_WATCH_MARK), "test-watch");
    writeFileSync(join(rerailHome, "daemon.json"), JSON.stringify({
      pid: process.pid, host: "127.0.0.1", port, url: `http://127.0.0.1:${port}/`, version: "test", ui: "bundled", startedAt: 1,
      packageRoot: rerailHome, databasePath: join(rerailHome, "rerail.sqlite"), kernelSocketPath: socketPath, accessToken: "token",
      untakenFolder: untakenFolderId(rerailHome),
    }));
    return async () => {
      releaseHome();
      await closeEngine();
    };
  }

  /** A front at the daemon's URL that never answers, and counts what reaches it. */
  async function silentFront(): Promise<{ server: Server; requests: () => number }> {
    let requests = 0;
    const server = createHttpServer(() => { requests += 1; });
    await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
    return { server, requests: () => requests };
  }

  async function closeFront(server: Server): Promise<void> {
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
  }

  const prompt = (eventId: string) => ({
    kind: "user-prompt" as const, eventId, sessionId: "s", text: `${eventId}, with the key sk-ant-api03-${"a".repeat(90)}`,
  });

  /** The names of the events left in `rerailHome`'s folder, the daemon's marks left out. */
  function eventNames(rerailHome: string): string[] {
    return readdirSync(join(rerailHome, UNTAKEN_EVENTS_DIR)).filter(name => name.endsWith(".json"));
  }

  /** The events left untaken in `rerailHome`, oldest first. */
  function untaken(rerailHome: string): string[] {
    const folder = join(rerailHome, UNTAKEN_EVENTS_DIR);
    return eventNames(rerailHome).sort().map(name => parseEngineRequest(JSON.parse(readFileSync(join(folder, name), "utf8"))).event.eventId);
  }

  it("leaves its event for the daemon once it has waited out its whole wait, scrubbed, and logs nothing lost", async () => {
    const rerailHome = await home();
    const closeEngine = await recordedDaemon(rerailHome, await closedPort());
    try {
      const handle = vi.fn(async (runtime: { ingest: (event: ReturnType<typeof prompt>) => Promise<unknown> }) => ({ answered: await runtime.ingest(prompt("first")) }));
      const options = { storageRoot: join(rerailHome, "sessions"), daemonDeadline: Date.now(), arrivedAt: 1_000 };
      expect(await runHostHook("claude-code", rerailHome, "s", false, options, handle, "UserPromptSubmit")).toEqual({ answered: [] });
      expect(untaken(rerailHome)).toEqual(["first"]);
      expect(readFileSync(join(rerailHome, UNTAKEN_EVENTS_DIR, eventNames(rerailHome)[0]!), "utf8")).not.toContain("sk-ant-api03-");
      expect(readHookFailures(rerailHome, 0).failures).toEqual([]);
    } finally {
      await closeEngine();
    }
  });

  it("asks a daemon that has left an earlier event untaken nothing, and leaves its own behind that one", async () => {
    const rerailHome = await home();
    const front = await silentFront();
    const closeEngine = await recordedDaemon(rerailHome, (front.server.address() as AddressInfo).port);
    try {
      await leaveUntaken(rerailHome, process.pid, engineRequestBody(prompt("first")), 1_000);
      const handle = vi.fn(async (runtime: { ingest: (event: ReturnType<typeof prompt>) => Promise<unknown> }) => ({ answered: await runtime.ingest(prompt("second")) }));
      // A deadline 2 s off, under the case's bound: a hook that asked the silent daemon would wait it out, and be counted.
      const options = { storageRoot: join(rerailHome, "sessions"), daemonDeadline: Date.now() + 2_000, arrivedAt: 2_000 };
      expect(await runHostHook("claude-code", rerailHome, "s", false, options, handle, "UserPromptSubmit")).toEqual({ answered: [] });
      expect(await hookRerailStarted("claude-code", rerailHome)).toBe(false);
      expect(front.requests()).toBe(0);
      expect(untaken(rerailHome)).toEqual(["first", "second"]);
      expect(readHookFailures(rerailHome, 0).failures).toEqual([]);
    } finally {
      await Promise.all([closeFront(front.server), closeEngine()]);
    }
  });

  it("still asks the daemon at a session's start, which is the hook that tells the person when none answers", async () => {
    const rerailHome = await home();
    const closeEngine = await recordedDaemon(rerailHome, await closedPort());
    try {
      await leaveUntaken(rerailHome, process.pid, engineRequestBody(prompt("first")), 1_000);
      const handle = vi.fn(async () => ({}));
      const options = { storageRoot: join(rerailHome, "sessions"), daemonDeadline: Date.now() };
      await expect(runHostHook("claude-code", rerailHome, "s", true, options, handle, "SessionStart")).rejects.toBeInstanceOf(DaemonUnreachableError);
      expect(handle).not.toHaveBeenCalled();
      expect(untaken(rerailHome)).toEqual(["first"]);
    } finally {
      await closeEngine();
    }
  });

  it("skips no wait for an event left in a folder the daemon does not watch: the one it watched was moved", async () => {
    const rerailHome = await home();
    const closeEngine = await recordedDaemon(rerailHome, await closedPort());
    try {
      await leaveUntaken(rerailHome, process.pid, engineRequestBody(prompt("first")), 1_000);
      expect(await daemonLeftEventUntaken()).toBeDefined();
      renameSync(join(rerailHome, UNTAKEN_EVENTS_DIR), join(rerailHome, "moved"));
      await leaveUntaken(rerailHome, process.pid, engineRequestBody(prompt("second")), 2_000);
      expect(await daemonLeftEventUntaken()).toBeUndefined();
    } finally {
      await closeEngine();
    }
  });

  // On Linux a refused connect to the recorded socket says the pid belongs to another process now; macOS refuses a
  // connect to a full queue too, and Windows asks the daemon's own pipe.
  it.runIf(process.platform === "linux")("skips no wait for a recorded daemon whose engine socket no process listens on", async () => {
    const rerailHome = await home();
    const closeEngine = await recordedDaemon(rerailHome, await closedPort());
    await leaveUntaken(rerailHome, process.pid, engineRequestBody(prompt("first")), 1_000);
    expect(await daemonLeftEventUntaken()).toBeDefined();
    await closeEngine();
    expect(await daemonLeftEventUntaken()).toBeUndefined();
  });

  // macOS asks the pid alone, so there no later check covers a pid the machine says is gone.
  it("skips no wait for a recorded daemon whose pid the machine says is gone", async () => {
    const rerailHome = await home();
    mkdirSync(join(rerailHome, UNTAKEN_EVENTS_DIR));
    // Marked as a daemon marks the folder its watch opens on.
    writeFileSync(join(rerailHome, UNTAKEN_EVENTS_DIR, UNTAKEN_WATCH_MARK), "test-watch");
    writeFileSync(join(rerailHome, "daemon.json"), JSON.stringify({
      pid: 4242, host: "127.0.0.1", port: 1, url: "http://127.0.0.1:1/", version: "test", ui: "bundled", startedAt: 1, packageRoot: rerailHome,
      databasePath: join(rerailHome, "rerail.sqlite"), kernelSocketPath: "/tmp/rerail-gone.sock", accessToken: "token", untakenFolder: untakenFolderId(rerailHome),
    }));
    await leaveUntaken(rerailHome, 4242, engineRequestBody(prompt("first")), 1_000);
    const env = { RERAIL_HOME: rerailHome };
    expect(await daemonLeftEventUntaken(fakeMachine("darwin", { env }))).toBeUndefined();
    expect(await daemonLeftEventUntaken(fakeMachine("darwin", { env, kill: () => true }))).toMatchObject({ pid: 4242 });
  });

  it("logs its event lost as before, and leaves nothing, beside a daemon whose state file names no folder of left events", async () => {
    const rerailHome = await home();
    const closeEngine = await recordedDaemon(rerailHome, await closedPort());
    try {
      // What an older daemon writes, or one whose watch could not start: no untakenFolder, and no one takes the folder.
      const older = JSON.parse(readFileSync(join(rerailHome, "daemon.json"), "utf8")) as Record<string, unknown>;
      delete older.untakenFolder;
      writeFileSync(join(rerailHome, "daemon.json"), JSON.stringify(older));
      const handle = vi.fn(async (runtime: { ingest: (event: ReturnType<typeof prompt>) => Promise<unknown> }) => ({ answered: await runtime.ingest(prompt("first")) }));
      const options = { storageRoot: join(rerailHome, "sessions"), daemonDeadline: Date.now(), arrivedAt: 1_000 };
      expect(await runHostHook("claude-code", rerailHome, "s", false, options, handle, "UserPromptSubmit")).toEqual({});
      expect(handle).not.toHaveBeenCalled();
      expect(eventNames(rerailHome)).toEqual([]);
      expect(readHookFailures(rerailHome, 0).failures).toEqual([expect.objectContaining({
        event: "UserPromptSubmit", reason: `the Rerail daemon (pid ${process.pid}) is running but did not answer within 12 s, so this event was not recorded`,
      })]);
    } finally {
      await closeEngine();
    }
  });

  it("leaves a tool's result without its output: the file holds the withheld marker, never what the tool printed", async () => {
    const rerailHome = await home();
    const closeEngine = await recordedDaemon(rerailHome, await closedPort());
    try {
      const result = { kind: "result" as const, eventId: "result", sessionId: "s", toolCallId: "t1", toolName: "Bash", output: { stdout: "PRINTED BY THE TOOL" } };
      const handle = vi.fn(async (runtime: { ingest: (event: typeof result) => Promise<unknown> }) => ({ answered: await runtime.ingest(result) }));
      const options = { storageRoot: join(rerailHome, "sessions"), daemonDeadline: Date.now(), arrivedAt: 1_000 };
      expect(await runHostHook("claude-code", rerailHome, "s", false, options, handle, "PostToolUse")).toEqual({ answered: [] });
      const [name] = eventNames(rerailHome);
      const text = readFileSync(join(rerailHome, UNTAKEN_EVENTS_DIR, name!), "utf8");
      expect(parseEngineRequest(JSON.parse(text)).event).toMatchObject({ kind: "result", toolCallId: "t1", output: { withheld: "tool output" } });
      expect(text).not.toContain("PRINTED BY THE TOOL");
    } finally {
      await closeEngine();
    }
  });

  it("names the limit of one event, and no wait, when its event is too large to leave", async () => {
    const rerailHome = await home();
    const front = await silentFront();
    const closeEngine = await recordedDaemon(rerailHome, (front.server.address() as AddressInfo).port);
    try {
      await leaveUntaken(rerailHome, process.pid, engineRequestBody(prompt("first")), 1_000);
      const large = { ...prompt("large"), text: "x".repeat(1024 * 1024) };
      const handle = async (runtime: { ingest: (event: ReturnType<typeof prompt>) => Promise<unknown> }) => ({ answered: await runtime.ingest(large) });
      const options = { storageRoot: join(rerailHome, "sessions"), daemonDeadline: Date.now() + 60_000, arrivedAt: 2_000 };
      expect(await runHostHook("claude-code", rerailHome, "s", false, options, handle, "UserPromptSubmit")).toEqual({ answered: [] });
      expect(front.requests()).toBe(0);
      expect(untaken(rerailHome)).toEqual(["first"]);
      // Before, it was logged as a 12 s wait for the daemon, which this hook never made.
      expect(readHookFailures(rerailHome, 0).failures).toEqual([expect.objectContaining({
        event: "UserPromptSubmit",
        reason: `the Rerail daemon (pid ${process.pid}) is running but not reading, and the event is larger than the 1 MiB one event may hold, so this event was not recorded`,
      })]);
    } finally {
      await Promise.all([closeFront(front.server), closeEngine()]);
    }
  });

  it("says what kept it from leaving its event when the folder cannot take it", async () => {
    const rerailHome = await home();
    const closeEngine = await recordedDaemon(rerailHome, await closedPort());
    try {
      // A file where the folder was: the hook that waited out its wait cannot write its event there.
      await removeFolder(join(rerailHome, UNTAKEN_EVENTS_DIR));
      writeFileSync(join(rerailHome, UNTAKEN_EVENTS_DIR), "");
      const handle = async (runtime: { ingest: (event: ReturnType<typeof prompt>) => Promise<unknown> }) => ({ answered: await runtime.ingest(prompt("first")) });
      const options = { storageRoot: join(rerailHome, "sessions"), daemonDeadline: Date.now(), arrivedAt: 1_000 };
      expect(await runHostHook("claude-code", rerailHome, "s", false, options, handle, "UserPromptSubmit")).toEqual({ answered: [] });
      expect(readHookFailures(rerailHome, 0).failures).toEqual([expect.objectContaining({
        event: "UserPromptSubmit", reason: expect.stringContaining(`the Rerail daemon (pid ${process.pid}) is running but not reading, and the event could not be left for it (`),
      })]);
    } finally {
      await closeEngine();
    }
  });

  it("holds no session lock: it leaves its event while another hook of its session holds the lock, and keeps none waiting", async () => {
    const rerailHome = await home();
    const front = await silentFront();
    const closeEngine = await recordedDaemon(rerailHome, (front.server.address() as AddressInfo).port);
    const storageRoot = join(rerailHome, "sessions");
    const holding = Promise.withResolvers<void>();
    const release = Promise.withResolvers<void>();
    const holder = withSessionLock(storageRoot, "claude-code", "s", async () => {
      holding.resolve();
      await release.promise;
    });
    try {
      await holding.promise;
      await leaveUntaken(rerailHome, process.pid, engineRequestBody(prompt("first")), 1_000);
      const handle = async (runtime: { ingest: (event: ReturnType<typeof prompt>) => Promise<unknown> }) => ({ answered: await runtime.ingest(prompt("second")) });
      // Before, the hook took the lock to run its handler and waited for the holder, up to the lock's 30 s.
      expect(await runHostHook("claude-code", rerailHome, "s", false, { storageRoot, daemonDeadline: Date.now() + 60_000, arrivedAt: 2_000 }, handle, "UserPromptSubmit")).toEqual({ answered: [] });
      expect(untaken(rerailHome)).toEqual(["first", "second"]);
    } finally {
      release.resolve();
      await holder;
      await Promise.all([closeFront(front.server), closeEngine()]);
    }
  });

  it("touches none of its session's files: a steer held for the session stays for the next hook the daemon answers", async () => {
    const rerailHome = await home();
    const front = await silentFront();
    const closeEngine = await recordedDaemon(rerailHome, (front.server.address() as AddressInfo).port);
    const storageRoot = join(rerailHome, "sessions");
    try {
      const held = { sessionId: "s", requested: "steer" as const, mechanism: "additional-context" as const, content: "held steer", ruleIds: [], rules: [] };
      const path = pendingPath(storageRoot, "claude-code", "s");
      await persistPending(path, [held]);
      const before = readFileSync(path, "utf8");
      await leaveUntaken(rerailHome, process.pid, engineRequestBody(prompt("first")), 1_000);
      const input = {
        hook_event_name: "PostToolUse", session_id: "s", cwd: rerailHome, transcript_path: join(rerailHome, "t.jsonl"),
        tool_name: "Bash", tool_input: { command: "ls" }, tool_use_id: "toolu_1", tool_response: { stdout: "", stderr: "" },
      };
      const databasePath = join(rerailHome, "rerail.sqlite");
      startTestRepository(rerailHome, databasePath);
      // Before, the hook routed the session's held steers as it would with a daemon's answer: it took them into its
      // own output and emptied their file, under the lock.
      expect(await runClaudeHook(input, { storageRoot, databasePath })).toEqual({});
      expect(readFileSync(path, "utf8")).toBe(before);
    } finally {
      await Promise.all([closeFront(front.server), closeEngine()]);
    }
  });

  it("loses its event, and says why, once the folder holds the most events Rerail keeps for the daemon", async () => {
    const rerailHome = await home();
    const front = await silentFront();
    const closeEngine = await recordedDaemon(rerailHome, (front.server.address() as AddressInfo).port);
    try {
      for (let count = 0; count < UNTAKEN_MOST_EVENTS; count += 1) {
        writeFileSync(join(rerailHome, UNTAKEN_EVENTS_DIR, `${String(1_000 + count).padStart(13, "0")}-${process.pid}-1-${count}-2.json`), "{}");
      }
      const handle = async (runtime: { ingest: (event: ReturnType<typeof prompt>) => Promise<unknown> }) => ({ answered: await runtime.ingest(prompt("past the bound")) });
      const options = { storageRoot: join(rerailHome, "sessions"), daemonDeadline: Date.now() + 60_000, arrivedAt: 9_000 };
      expect(await runHostHook("claude-code", rerailHome, "s", false, options, handle, "UserPromptSubmit")).toEqual({ answered: [] });
      expect(front.requests()).toBe(0);
      expect(eventNames(rerailHome)).toHaveLength(UNTAKEN_MOST_EVENTS);
      expect(readHookFailures(rerailHome, 0).failures).toEqual([expect.objectContaining({
        event: "UserPromptSubmit", reason: expect.stringContaining(`${UNTAKEN_MOST_EVENTS} events already wait for it, the most Rerail keeps, so this event was not recorded`),
      })]);
    } finally {
      await Promise.all([closeFront(front.server), closeEngine()]);
    }
  });

  it("is named by rerail status while it waits, and once the daemon checked it late", async () => {
    const rerailHome = await home();
    await leaveUntaken(rerailHome, 41, engineRequestBody(prompt("first")), Date.UTC(2026, 10, 17, 12));
    await leaveUntaken(rerailHome, 41, engineRequestBody(prompt("second")), Date.UTC(2026, 10, 17, 12, 1));
    expect(untakenWaitingLines(rerailHome, 41)).toEqual([
      "Hooks: 2 hook events wait in untaken-events for the Rerail daemon, which has not read since 2026-11-17T12:00:00.000Z; it checks them as soon as it reads again, and its next answers carry what it finds.",
    ]);
    expect(untakenWaitingLines(rerailHome, 42)).toEqual([]);
    recordLateCheck({ events: 2, from: Date.UTC(2026, 10, 17, 12), to: Date.UTC(2026, 10, 17, 12, 1), failed: 0 }, { home: rerailHome, now: Date.UTC(2026, 10, 17, 12, 2) });
    recordLateCheck({ events: 3, from: Date.UTC(2026, 10, 17, 13), to: Date.UTC(2026, 10, 17, 13, 5), failed: 1 }, { home: rerailHome, now: Date.UTC(2026, 10, 17, 13, 6) });
    const log = readHookFailures(rerailHome, 0);
    // A late check is no failure: the line that counts events Rerail could not record says nothing of it.
    expect(hookFailureLines(log, null)).toEqual([]);
    expect(lateCheckLines(log)).toEqual([
      "Hooks: 5 hook events came from 2026-11-17T12:00:00.000Z to 2026-11-17T13:05:00.000Z while the Rerail daemon was not reading, in the last 24 hours; it checked them late, once it read again, and what it found went to the agent with its next answers; 1 event of them could not be checked and was lost.",
    ]);
  });

  it("reads a state file it cannot read as no event left untaken, and goes on to ask as before", async () => {
    const rerailHome = await home();
    vi.stubEnv("RERAIL_HOME", rerailHome);
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    // A folder where daemon.json belongs: reading it fails with EISDIR on every system, as a file the user cannot read does.
    mkdirSync(join(rerailHome, "daemon.json"));
    await expect(hookRerailStarted("claude-code", rerailHome)).resolves.toBe(false);
  });
});
