import { spawn, type ChildProcess } from "node:child_process";
import { randomUUID } from "node:crypto";
import { once } from "node:events";
import { appendFileSync, closeSync, existsSync, openSync, readdirSync, readFileSync, utimesSync, writeFileSync, writeSync } from "node:fs";
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

  it("reads past lines that hold no failure, and fills in the fields a failure's line lacks", async () => {
    const rerailHome = await home();
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), [
      "",
      '{"at":1000,"host":"codex","event":"Stop","reas',
      '{"at":2000,"host":"codex","reason":"no event"}',
      '{"startedAt":2500}',
      '{"at":3000,"event":"Stop","reason":"no host, agent or version"}',
      "",
    ].join("\n"));
    expect(readHookFailures(rerailHome, 0).failures).toEqual([
      { at: 3_000, host: "unknown", event: "Stop", agent: null, rerail: "unknown", reason: "no host, agent or version" },
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
  // An event recorded after every failure below: the line counts them without the lead.
  const afterEveryFailure = 60_000;
  // The number of hook events `rerail status` says could not be recorded in `rerailHome`; 0 when it says nothing.
  const statusCount = (rerailHome: string): number =>
    Number(/^Hooks: (\d+) hook events? could not be recorded /u.exec(statusSays(rerailHome, 0, afterEveryFailure)[0] ?? "")?.[1] ?? 0);

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

  // A log of failures at 1 ms, just past its size: the next failure finds it full and moves it aside.
  const fullLine = JSON.stringify({ at: 1, host: "claude-code", event: "PostToolUse", agent: null, rerail: "0.1.3", reason: "x".repeat(1_000) });
  const full = Math.ceil((256 * 1024 + 1) / (fullLine.length + 1));
  const writeFullLog = (rerailHome: string): void => {
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), `${fullLine}\n`.repeat(full));
  };

  // The files of the log in `rerailHome`, by name.
  const logFiles = (rerailHome: string): string[] => readdirSync(rerailHome).filter(name => name.startsWith(HOOK_FAILURES_FILE)).sort();
  // The failure lines in those files, whoever reads them: a line with an event.
  const failuresWritten = (rerailHome: string): number => logFiles(rerailHome)
    .flatMap(name => readFileSync(join(rerailHome, name), "utf8").split("\n"))
    .filter(line => line.includes('"event":')).length;

  // A busy daemon fails every hook at once, 25 to 37 at a time on a loaded Windows machine. Each round, every process
  // records one failure into a log already past its size, so each finds it full and moves it aside.
  it("keeps every failure when twenty hooks running at once each find the log full", async () => {
    const processes = 20;
    const rounds = 4;
    const hooks = await startHookProcesses(processes);
    try {
      for (let round = 0; round < rounds; round += 1) {
        const rerailHome = await home();
        writeFullLog(rerailHome);
        await Promise.all(hooks.map((hook, index) => hook.fail(rerailHome, 2 + index)));
        // The full log moved aside whole, and every hook's own failure after it, and status counts them all.
        expect({ round, written: failuresWritten(rerailHome) }).toEqual({ round, written: full + processes });
        expect(readFileSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`), "utf8").split("\n").filter(Boolean)).toHaveLength(full);
        expect({ round, counted: statusCount(rerailHome) }).toEqual({ round, counted: full + processes });
      }
    } finally {
      await stopHookProcesses(hooks);
    }
  }, 120_000);
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

  // A hook writes a failure into the full log, with the clock's time, in the moment another hook takes the log.
  it("names a time later than every failure in the part it no longer has", async () => {
    const rerailHome = await home();
    writeFullLog(rerailHome);
    const writtenAsTaken = Number(await recordWithRename(rerailHome, undefined, `rename => (from, to) => {
      const at = Date.now();
      if (to.endsWith(".old")) fs.appendFileSync(from, JSON.stringify({ at, host: "codex", event: "Stop", reason: "as it was taken" }) + "\\n");
      rename(from, to);
      if (to.endsWith(".old")) fs.writeSync(1, String(at));
    }`));
    // The next move drops that log, and the part the hook began moves to `.1`.
    appendFileSync(join(rerailHome, HOOK_FAILURES_FILE), `${fullLine}\n`.repeat(full));
    recordHookFailure({ host: "claude-code", event: "Stop", reason: "after" }, { home: rerailHome, env: {} });
    expect(readHookFailures(rerailHome, 0).keptSince).toBeGreaterThan(writtenAsTaken);
  });

  it("removes a log a stopped hook left at the next move, once it holds nothing from the period status counts", async () => {
    const rerailHome = await home();
    const day = 24 * 60 * 60 * 1_000;
    const now = 3 * day;
    const takenLog = (pid: number): string => `${HOOK_FAILURES_FILE}.${pid}.${randomUUID()}.old`;
    // No process holds this pid: Linux's pids stay below 2^22, and Windows' are multiples of 4.
    const stopped = 2 ** 31 - 1;
    const left = { stoppedLongAgo: takenLog(stopped), stoppedToday: takenLog(stopped), running: takenLog(process.ppid) };
    for (const [name, writtenAt] of [[left.stoppedLongAgo, now - day - 1_000], [left.stoppedToday, now - day + 60_000], [left.running, now - 2 * day]] as const) {
      writeFileSync(join(rerailHome, name), `${fullLine}\n`);
      utimesSync(join(rerailHome, name), writtenAt / 1_000, writtenAt / 1_000);
    }
    writeFullLog(rerailHome);
    recordLast(rerailHome, now);
    expect(logFiles(rerailHome)).toEqual([HOOK_FAILURES_FILE, `${HOOK_FAILURES_FILE}.1`, left.running, left.stoppedToday].sort());
  });

  it("says since when it counts from the log moved aside, not from an older log a hook was stopped holding", async () => {
    const rerailHome = await home();
    writeFullLog(rerailHome);
    await recordWithRename(rerailHome, 2, stopAfterTheTake);
    // The log moves aside at the 7th and the 10th failure; the 4th to 6th are gone, and the stopped hook's log is older.
    for (let second = 4; second <= 9; second += 1) recordLarge(rerailHome, second * 1_000);
    recordLast(rerailHome, 10_000);
    expect(statusSays(rerailHome, 0, afterEveryFailure)).toEqual([expect.stringMatching(
      /^Hooks: 4 hook events could not be recorded since 1970-01-01T00:00:07\.000Z, as far back as the log goes; /u,
    )]);
  });

  // Another program can hold `.1` open on Windows, such as an editor or a backup. A hook that waited 2 s for it held the
  // full log under its own name all that time.
  it("gives the full log back at once when it cannot move it to .1", async () => {
    const rerailHome = await home();
    writeFullLog(rerailHome);
    const refused = await recordWithRename(rerailHome, 2, `rename => (from, to) => {
      if (!to.endsWith(".1")) return rename(from, to);
      fs.writeSync(1, "refused ");
      throw Object.assign(new Error("EPERM: operation not permitted, rename"), { code: "EPERM" });
    }`);
    expect(refused).toBe("refused ");
    expect(logFiles(rerailHome)).toEqual([HOOK_FAILURES_FILE]);
    expect(readHookFailures(rerailHome, 0).failures).toHaveLength(full + 1);
  });

  // A failure line, as the source of a string, for the programs below.
  const lineSource = (at: number, reason: string): string => JSON.stringify(`${JSON.stringify({ at, host: "codex", event: "Stop", reason })}\n`);
  // The reasons of the failures the log keeps past the full log's.
  const reasonsAfterTheFullLog = (rerailHome: string): string[] => readHookFailures(rerailHome, 0).failures.slice(full).map(entry => entry.reason);

  // A hook opens the full log to append and writes only once another hook has moved it aside: its line lands in the
  // part moved aside.
  it("counts the failure a hook writes into the log as another hook moves it aside", async () => {
    const rerailHome = await home();
    writeFullLog(rerailHome);
    const writing = openSync(join(rerailHome, HOOK_FAILURES_FILE), "a");
    try {
      recordLast(rerailHome, 3);
      writeSync(writing, `${JSON.stringify(failure(2))}\n`);
    } finally {
      closeSync(writing);
    }
    expect(statusCount(rerailHome)).toBe(full + 2);
  });

  // A hook opens the log to append and is held up before it writes, as a busy machine can hold a process. Meanwhile
  // another hook moves the full log aside and begins the next part, and a hook that found the log full before that takes
  // the new part, finds it small and gives it back. The held-up hook writes as its process ends.
  it("keeps the failure a held-up hook writes into the log another hook took and gave back", async () => {
    const rerailHome = await home();
    writeFullLog(rerailHome);
    await recordWithRename(rerailHome, 2, `rename => (from, to) => {
      if (to.endsWith(".old") && fs.statSync(from).size > 256 * 1024) {
        rename(from, from + ".1");
        fs.appendFileSync(from, ${lineSource(3, "began the next part")});
        const heldUp = fs.openSync(from, "a");
        process.on("exit", () => fs.writeSync(heldUp, ${lineSource(4, "held up")}));
      }
      rename(from, to);
    }`);
    expect(logFiles(rerailHome)).toEqual([HOOK_FAILURES_FILE, `${HOOK_FAILURES_FILE}.1`]);
    expect(reasonsAfterTheFullLog(rerailHome)).toEqual(["the daemon did not answer", "began the next part", "held up"]);
  });

  // As above, but another hook begins a live log before the part goes back, so its lines go back instead, and the
  // held-up hook writes after the give-back has read them.
  it("gives back what a held-up hook writes into the log after the give-back read it", async () => {
    const rerailHome = await home();
    writeFullLog(rerailHome);
    await recordWithRename(rerailHome, 2, `rename => (from, to) => {
      if (!to.endsWith(".old") || fs.statSync(from).size <= 256 * 1024) return rename(from, to);
      rename(from, from + ".1");
      fs.appendFileSync(from, ${lineSource(3, "began the next part")});
      const heldUp = fs.openSync(from, "a");
      rename(from, to);
      fs.appendFileSync(from, ${lineSource(4, "began a live log")});
      globalThis.heldUp = heldUp;
    }`, `append => (path, data, options) => {
      if (globalThis.heldUp !== undefined) fs.writeSync(globalThis.heldUp, ${lineSource(5, "held up")});
      globalThis.heldUp = undefined;
      return append(path, data, options);
    }`);
    expect(logFiles(rerailHome)).toEqual([HOOK_FAILURES_FILE, `${HOOK_FAILURES_FILE}.1`]);
    expect(reasonsAfterTheFullLog(rerailHome)).toEqual(["the daemon did not answer", "began the next part", "began a live log", "held up"]);
  });

  // A hook gives the full log back while another program holds `.1`, and a program that holds the log without delete
  // sharing refuses the removal of the hook's own name: the log is the live one under two names. The next hook takes
  // the live name, cannot move it to `.1` either, and finds a live log another hook began since its take, so it gives
  // the lines back by copy. They are still under the first hook's name, where status counts them.
  it("counts once the lines of a log another name still holds when a hook gives it back by copy", async () => {
    const rerailHome = await home();
    writeFullLog(rerailHome);
    const refuse = (call: string): string => `throw Object.assign(new Error("EPERM: operation not permitted, ${call}"), { code: "EPERM" });`;
    await recordWithWrapped(rerailHome, 2, {
      renameSync: `rename => (from, to) => { if (!to.endsWith(".1")) return rename(from, to); ${refuse("rename")} }`,
      unlinkSync: `unlink => path => { if (!path.endsWith(".old")) return unlink(path); ${refuse("unlink")} }`,
    });
    expect(logFiles(rerailHome)).toEqual([HOOK_FAILURES_FILE, expect.stringMatching(/^hook-failures\.jsonl\.\d+\.[0-9a-f-]+\.old$/u)]);
    await recordWithRename(rerailHome, 4, `rename => (from, to) => {
      if (!to.endsWith(".1")) return rename(from, to);
      fs.appendFileSync(${JSON.stringify(join(rerailHome, HOOK_FAILURES_FILE))}, ${lineSource(3, "began a live log")});
      ${refuse("rename")}
    }`);
    expect(reasonsAfterTheFullLog(rerailHome)).toEqual(["the daemon did not answer", "began a live log", "the daemon did not answer"]);
    expect(statusCount(rerailHome)).toBe(full + 3);
  });

  // On Windows a rename opens the file by name and renames what it opened, so a hook can move a log on from the name a
  // give-back left it under while another hook takes the live name the give-back made: two hooks then hold one log under
  // their own names. Here each hook takes the same part, waits until the other has it too, finds the full live log
  // another hook began, and gives the part back by copy at the same moment as the other.
  it("copies a log two hooks hold under their own names back once when both give it back at the same moment", async () => {
    const rerailHome = await home();
    writeFullLog(rerailHome);
    const part = join(await home(), "part");
    writeFileSync(part, `${JSON.stringify(failure(3))}\n${JSON.stringify(failure(4))}\n`);
    const meet = await home();
    // Marks `step` for this process, then waits up to `ms` for the other process to mark it too.
    const both = (step: string, ms: number): string => `fs.writeFileSync(${JSON.stringify(meet)} + "/${step}-" + process.pid, "");
      for (const until = Date.now() + ${ms}; fs.readdirSync(${JSON.stringify(meet)}).filter(name => name.startsWith("${step}-")).length < 2 && Date.now() < until;) {
        Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 5);
      }`;
    const wraps = {
      renameSync: `rename => (from, to) => { if (!to.endsWith(".old")) return rename(from, to); fs.linkSync(${JSON.stringify(part)}, to); ${both("took", 20_000)} }`,
      unlinkSync: `unlink => path => { unlink(path); if (path.endsWith(".old")) { ${both("gave", 300)} } }`,
    };
    await Promise.all([recordWithWrapped(rerailHome, 5, wraps), recordWithWrapped(rerailHome, 6, wraps)]);
    expect(readdirSync(meet).filter(name => name.startsWith("took-"))).toHaveLength(2);
    // The full log, the part's two failures once, and each hook's own.
    expect(statusCount(rerailHome)).toBe(full + 4);
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
          counts.add(statusCount(rerailHome));
          await setImmediate();
        }
        await recording;
        expect({ round, counts: [...counts].filter(count => count !== full && count !== full + 1) }).toEqual({ round, counts: [] });
        expect(statusCount(rerailHome)).toBe(full + 1);
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
 * Records one failure at `at` (the clock's time when undefined) into `rerailHome` in a process of its own whose
 * `fs.renameSync` is the function that `wrapRename`, a function's source, makes of the real one: a hook stopped, or a
 * rename refused, at one step of a move. `wrapAppend` does the same for `fs.appendFileSync`. Resolves with what the
 * process wrote to its standard output, once it has ended.
 */
async function recordWithRename(rerailHome: string, at: number | undefined, wrapRename: string, wrapAppend = "append => append"): Promise<string> {
  return await recordWithWrapped(rerailHome, at, { renameSync: wrapRename, appendFileSync: wrapAppend });
}

/** {@link recordWithRename} with each `node:fs` function `wraps` names made by the function source given for it. */
async function recordWithWrapped(rerailHome: string, at: number | undefined, wraps: Partial<Record<"renameSync" | "appendFileSync" | "unlinkSync", string>>): Promise<string> {
  const program = `import fs from "node:fs";
import { syncBuiltinESMExports } from "node:module";
${Object.entries(wraps).map(([name, wrap]) => `fs.${name} = (${wrap})(fs.${name});`).join("\n")}
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
