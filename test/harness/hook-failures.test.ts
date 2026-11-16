import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { randomUUID } from "node:crypto";
import { once } from "node:events";
import { appendFileSync, chmodSync, closeSync, constants, existsSync, lstatSync, mkdirSync, openSync, readdirSync, readFileSync, readlinkSync, renameSync, utimesSync, writeFileSync, writeSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { createServer as createHttpServer, type IncomingMessage, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { setImmediate } from "node:timers/promises";
import { afterEach, describe, expect, it, vi, type TestContext } from "vitest";

const autostart = vi.hoisted(() => ({ calls: 0 }));
vi.mock("../../src/runtime/daemon-control.js", async () => {
  const actual = await vi.importActual<typeof import("../../src/runtime/daemon-control.js")>("../../src/runtime/daemon-control.js");
  return { ...actual, autostartDaemon: () => { autostart.calls += 1; } };
});

import { HOOK_FAILURES_FILE, hookFailureLines, readHookFailures, recordHookFailure, type HookFailure, type HookFailureLog } from "../../src/harness/hook-failures.js";
import { DaemonUnreachableError, runHostHook } from "../../src/harness/hook-host.js";
import { listenAsEngine } from "../support/platform/engine-endpoint.js";
import { linkFolder, symlinkOrSkip } from "../support/platform/links.js";
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

// Most tests below start Node processes with tsx, which takes seconds on a loaded machine, and one waits up to 20 s for
// two of them to meet: past the 5 s a test gets on Linux and macOS.
const PROCESS_TEST_TIMEOUT_MS = 60_000;

describe("the hook failure log", { timeout: PROCESS_TEST_TIMEOUT_MS }, () => {
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
  // The number of hook events a line of `rerail status` says could not be recorded; 0 when it says nothing.
  const countOf = (line: string | undefined): number => Number(/^Hooks: (\d+) hook events? could not be recorded /u.exec(line ?? "")?.[1] ?? 0);
  // That number for `rerailHome`.
  const statusCount = (rerailHome: string): number => countOf(statusSays(rerailHome, 0, afterEveryFailure)[0]);

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

  // The two kinds of link a name of the log can be, each made at `path` to `target`. On Windows the link to a folder is
  // a junction.
  const LINKS = [
    { link: "a symbolic link to a file", make: symlinkOrSkip },
    { link: "a link to a folder", make: (_context: TestContext, target: string, path: string) => linkFolder(target, path) },
  ];

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

  // On Windows each process's clock is the system time it read at its start, which moves on once per timer tick, at
  // most 15.625 ms apart: two hooks' clocks can differ by that much. Here the clocks are fixed: the hook that takes the
  // log reads 1_800_000_000_000, and a hook whose clock runs 16 ms ahead writes into the log as it is taken.
  it("names a time later than every failure in the part it no longer has when another hook's clock runs a tick ahead", async () => {
    const rerailHome = await home();
    writeFullLog(rerailHome);
    const clock = 1_800_000_000_000;
    await recordWithWrapped(rerailHome, undefined, {
      renameSync: `rename => (from, to) => {
        if (to.endsWith(".old")) fs.appendFileSync(from, ${lineSource(clock + 16, "a tick ahead")});
        rename(from, to);
      }`,
    }, clock);
    // The next move drops the part that holds the failure a tick ahead.
    appendFileSync(join(rerailHome, HOOK_FAILURES_FILE), `${fullLine}\n`.repeat(full));
    recordHookFailure({ host: "claude-code", event: "Stop", reason: "after" }, { home: rerailHome, now: clock + 60_000, env: {} });
    const log = readHookFailures(rerailHome, 0);
    expect(log.keptSince).toBeGreaterThan(clock + 16);
    expect(log.failures.map(entry => entry.reason)).toEqual(["the daemon did not answer", "after"]);
  });

  // A move a day after a stopped hook's log last changed. Windows soon hands a freed pid to another process: this test's
  // parent process stands for one that has the stopped hook's pid now.
  it("removes a log a stopped hook left at the next move once it has not changed for a day, whatever process has its pid now", async () => {
    const rerailHome = await home();
    const day = 24 * 60 * 60 * 1_000;
    const takenLog = (pid: number): string => `${HOOK_FAILURES_FILE}.${pid}.${randomUUID()}.old`;
    // No process holds this pid: Linux's pids stay below 2^22, and Windows' are multiples of 4.
    const left = [takenLog(2 ** 31 - 1), takenLog(process.ppid)];
    for (const name of left) writeFileSync(join(rerailHome, name), `${fullLine}\n`);
    const changed = Date.now();
    const moveAt = (at: number): string[] => {
      writeFullLog(rerailHome);
      recordLast(rerailHome, at);
      return logFiles(rerailHome);
    };
    const parts = [HOOK_FAILURES_FILE, `${HOOK_FAILURES_FILE}.1`];
    expect({ withinTheDay: moveAt(changed + day - 60_000), past: moveAt(changed + day + 60_000) }).toEqual({
      withinTheDay: [...parts, ...left].sort(),
      past: parts,
    });
  });

  // A link under a stopped hook's own name, its target gone, as a person's link that a hook took in the moment the person
  // made it at the live name, and was stopped holding. The move removes only a regular file, the one kind a hook leaves
  // there: the link stays, as on `main` it would have stayed at the live name, and the move reads nothing through it.
  // `fs.statSync` on it would follow it, and on Windows a link to a network share that does not answer makes that wait
  // about 21 s at every move.
  it.for(LINKS)("keeps $link under a stopped hook's own name, never reading through it or removing it", async ({ make }, context) => {
    const rerailHome = await home();
    const day = 24 * 60 * 60 * 1_000;
    const link = takenName(rerailHome, 2 ** 31 - 1);
    make(context, join(rerailHome, "gone", "log"), link);
    const changed = Date.now();
    const followedAt = `stat => (path, ...rest) => {
      if (String(path) === ${JSON.stringify(link)}) fs.writeSync(1, "followed ");
      return stat(path, ...rest);
    }`;
    const moveAt = async (at: number): Promise<{ followed: string; files: string[] }> => {
      writeFullLog(rerailHome);
      const followed = await recordWithWrapped(rerailHome, at, { statSync: followedAt });
      return { followed, files: logFiles(rerailHome) };
    };
    const parts = [HOOK_FAILURES_FILE, `${HOOK_FAILURES_FILE}.1`];
    expect({ withinTheDay: await moveAt(changed + day - 60_000), past: await moveAt(changed + day + 60_000) }).toEqual({
      withinTheDay: { followed: "", files: [...parts, basename(link)].sort() },
      past: { followed: "", files: [...parts, basename(link)].sort() },
    });
  });

  // A log of `bytes` a person keeps outside the Rerail home, linked at the live name: through `hops` links in a row, each
  // in a folder of its own.
  const linkLogElsewhere = async (context: TestContext, rerailHome: string, bytes: number, hops = 1): Promise<{ elsewhere: string; text: string }> => {
    const elsewhere = join(await home(), HOOK_FAILURES_FILE);
    const text = `${fullLine}\n`.repeat(Math.ceil(bytes / (fullLine.length + 1)));
    writeFileSync(elsewhere, text);
    let target = elsewhere;
    for (let hop = 1; hop < hops; hop += 1) {
      const between = join(await home(), `link-${hop}`);
      symlinkOrSkip(context, target, between, "file");
      target = between;
    }
    symlinkOrSkip(context, target, join(rerailHome, HOOK_FAILURES_FILE), "file");
    return { elsewhere, text };
  };
  // A file of `lines` failures a person keeps outside the Rerail home, and a link to it the person has made in the Rerail
  // home under a name no log has, `made`, ready to be renamed over the live name.
  const linkMadeLater = async (context: TestContext, rerailHome: string, lines: number): Promise<{ elsewhere: string; text: string; made: string }> => {
    const elsewhere = join(await home(), HOOK_FAILURES_FILE);
    const text = `${fullLine}\n`.repeat(lines);
    writeFileSync(elsewhere, text);
    const made = join(rerailHome, "made-by-a-person");
    symlinkOrSkip(context, elsewhere, made, "file");
    return { elsewhere, text, made };
  };
  // The wraps of a hook's process under which every open, read or append of the file `elsewhere`, by any name, prints
  // its call and fails, as when the share it is on stops answering or a program holds a write lease on it.
  const refusedThrough = (elsewhere: string): Wraps => {
    const refused = (call: string): string => `${call} => {
      const target = fs.realpathSync(${JSON.stringify(elsewhere)});
      return (path, ...rest) => {
        let reaches = null;
        try { reaches = fs.realpathSync(String(path)); } catch {}
        if (reaches !== target) return ${call}(path, ...rest);
        fs.writeSync(1, "${call} ");
        throw Object.assign(new Error("EIO: i/o error, ${call}"), { code: "EIO" });
      };
    }`;
    return { openSync: refused("open"), readFileSync: refused("read"), appendFileSync: refused("append") };
  };
  const kindOf = (path: string): string => {
    const stats = lstatSync(path, { throwIfNoEntry: false });
    if (stats === undefined) return "missing";
    return stats.isSymbolicLink() ? "link" : stats.isFile() ? "file" : "other";
  };
  // What the live name and `.1` are once a hook moved a log linked at the live name, the files of the log, and the
  // number status counts. Status reads the live name only when it is a regular file: a link left there can lead to a
  // named pipe, whose open would hold this test's own process until a program wrote to it.
  const afterTheMove = (rerailHome: string): { live: string; movedAside: string; files: string[]; counted: number | null } => {
    const live = kindOf(join(rerailHome, HOOK_FAILURES_FILE));
    return {
      live, movedAside: kindOf(join(rerailHome, `${HOOK_FAILURES_FILE}.1`)), files: logFiles(rerailHome),
      counted: live === "file" ? statusCount(rerailHome) : null,
    };
  };
  // The link itself moved to `.1`, where status does not follow it, and each failure since is in a new live log.
  const movedAsTheLink = (counted: number): ReturnType<typeof afterTheMove> => ({
    live: "file", movedAside: "link", files: [HOOK_FAILURES_FILE, `${HOOK_FAILURES_FILE}.1`], counted,
  });

  // A person made the live name a link to a log they keep elsewhere, and the log is full: the next hook moves the link to
  // `.1` as it is, and a new log begins in the Rerail home. Status follows no link at `.1`, so from then on it counts the
  // failures since the move, in the words it uses when there is no `.1`. The person's file stays as it was.
  it("counts the failures since a hook moved a full log a person linked at the live name to .1 as the link", async context => {
    const rerailHome = await home();
    const { elsewhere, text } = await linkLogElsewhere(context, rerailHome, 256 * 1024 + 1);
    const before = statusCount(rerailHome);
    recordLast(rerailHome, 2);
    recordLast(rerailHome, 3);
    expect({ before, ...afterTheMove(rerailHome), said: statusSays(rerailHome, 0, afterEveryFailure), leftAsItWas: readFileSync(elsewhere, "utf8") === text }).toEqual({
      before: full, ...movedAsTheLink(2),
      said: [expect.stringMatching(/^Hooks: 2 hook events could not be recorded in the last 24 hours; /u)],
      leftAsItWas: true,
    });
  });

  // As above, and every open of the person's file, by any name, fails, as when the share it is on stops answering, or
  // its disk is nearly full and a copy would fill it. The hook reads nothing through the link: its size check stats the
  // live name, as `main`'s does, and the link moves on as it is, the first link of a chain too. So the hook keeps its own
  // failure in the Rerail home.
  it.for([{ how: "a link", hops: 1 }, { how: "a link to a link", hops: 2 }])("moves a full log a person linked at the live name to .1 without opening or reading it through $how", async ({ hops }, context) => {
    const rerailHome = await home();
    const { elsewhere, text } = await linkLogElsewhere(context, rerailHome, 256 * 1024 + 1, hops);
    const opened = await recordWithWrapped(rerailHome, 2, refusedThrough(elsewhere));
    expect({ opened, ...afterTheMove(rerailHome), leftAsItWas: readFileSync(elsewhere, "utf8") === text }).toEqual({
      opened: "", ...movedAsTheLink(1), leftAsItWas: true,
    });
  });

  // Ways a file Rerail can look at refuses to be read, on each system: each denies `path` and gives the step that lifts
  // the denial. Windows test processes run with no backup privilege, so an access list binds them as it binds a person's
  // programs. A file held with no sharing (libuv's UV_FS_O_EXLOCK, 0x10000000, which `fs.constants` does not name)
  // refuses every other open, this process's own among them, and so does mode 0000; mode 0200 and a denied read take
  // writes.
  interface Denial { how: string; deny: (path: string) => () => void }
  const icacls = (...args: string[]): void => { execFileSync("icacls", args, { stdio: "ignore", windowsHide: true }); };
  const readDenied: Denial = { how: "its access list denies reading it", deny: path => {
    icacls(path, "/deny", "*S-1-1-0:(RD)");
    return () => icacls(path, "/remove:d", "*S-1-1-0");
  } };
  const heldWithNoSharing: Denial = { how: "another program holds it with no sharing", deny: path => {
    const held = openSync(path, constants.O_RDWR | 0x1000_0000);
    return () => closeSync(held);
  } };
  const withMode = (mode: number): Denial => ({ how: `its mode is 0${mode.toString(8).padStart(3, "0")}`, deny: path => {
    chmodSync(path, mode);
    return () => chmodSync(path, 0o600);
  } });
  const UNREADABLE = process.platform === "win32" ? [readDenied, heldWithNoSharing] : [withMode(0o200), withMode(0o000)];
  // What `act` gives while `denial` keeps `path` from being read; the denial is lifted after.
  const whileDenied = <T>(path: string, { deny }: Denial, act: () => T): T => {
    const allow = deny(path);
    try {
      return act();
    } finally {
      allow();
    }
  };
  const skipAsRoot = (context: TestContext): void => {
    if (process.getuid?.() === 0) context.skip("root reads a file whatever its mode");
  };

  // A person linked the live name to a full log that Rerail can look at but not read. The hook reads nothing through the
  // link, so this changes nothing: the link moves to `.1` as it is, and each failure since goes into a new log of the
  // Rerail home, where status counts it. Given back to the live name, the link would take every later failure into the
  // person's file, or lose it where the file takes no writes, and status, unable to read the log, would say nothing.
  it.for(UNREADABLE)("counts every later failure in the Rerail home when a full log linked at the live name cannot be read, as $how", async (denial, context) => {
    skipAsRoot(context);
    const rerailHome = await home();
    const { elsewhere, text } = await linkLogElsewhere(context, rerailHome, 256 * 1024 + 1);
    const moved = whileDenied(elsewhere, denial, () => {
      recordLast(rerailHome, 2);
      recordLast(rerailHome, 3);
      return afterTheMove(rerailHome);
    });
    expect({ ...moved, leftAsItWas: readFileSync(elsewhere, "utf8") === text }).toEqual({ ...movedAsTheLink(2), leftAsItWas: true });
  });

  // Ways `.1` can refuse a link moved there: a folder a person or another program put there, and on Windows a file another
  // program holds past the 2 s a rename retries. Each blocks `path` and gives the step that lifts the block.
  const UNREACHABLE = [
    { how: "is a folder", block: (path: string): (() => void) => { mkdirSync(path); return () => undefined; } },
    ...(process.platform === "win32" ? [{ how: "is held by another program past the retries", block: (path: string): (() => void) => {
      writeFileSync(path, "");
      return heldWithNoSharing.deny(path);
    } }] : []),
  ];
  // What a hook leaves when it found a full log a person linked at the live name and wrote nothing: the link at the live
  // name, what is at `.1`, and the person's file as it was. Status reads that file through the link, as `main`'s does.
  const leftInPlace = (rerailHome: string, elsewhere: string, text: string): Record<string, unknown> => {
    const live = join(rerailHome, HOOK_FAILURES_FILE);
    const kind = kindOf(live);
    return {
      live: kind, leadsTo: kind === "link" ? readlinkSync(live) : null, files: logFiles(rerailHome), counted: statusCount(rerailHome),
      leftAsItWas: readFileSync(elsewhere, "utf8") === text,
    };
  };

  // A wrap of `fs.renameSync` that prints `took ` when a hook takes the live name under a name of its own.
  const tookPrinted = `rename => (from, to) => {
    if (to.endsWith(".old")) fs.writeSync(1, "took ");
    return rename(from, to);
  }`;

  // As above, and `.1` cannot take the link. `main`'s move fails there and its hook writes nothing: the hook's failure is
  // lost, the link stays at the live name and nothing waits. The hook does the same here, and never takes the link, so
  // the live name is never free for another hook's log. Given back and written through, the link would take the hook's
  // failure, and the coding agent's event with it, to whatever it leads to: a file under a write lease, or on a share
  // that stopped answering, holds the hook until the agent stops it. Every open, read or append of the person's file
  // fails here and prints its call.
  it.for(UNREACHABLE)("leaves a full log a person linked at the live name in place, opening nothing through it, when .1 $how", async ({ block }, context) => {
    const rerailHome = await home();
    const { elsewhere, text } = await linkLogElsewhere(context, rerailHome, 256 * 1024 + 1);
    const lift = block(join(rerailHome, `${HOOK_FAILURES_FILE}.1`));
    const opened = await recordWithWrapped(rerailHome, 2, { ...refusedThrough(elsewhere), renameSync: tookPrinted }).finally(lift);
    expect({ opened, ...leftInPlace(rerailHome, elsewhere, text) }).toEqual({
      opened: "", live: "link", leadsTo: elsewhere, files: [HOOK_FAILURES_FILE, `${HOOK_FAILURES_FILE}.1`], counted: full, leftAsItWas: true,
    });
  });

  // As above, and another hook begins a live log, as one that finds no log at the live name does, in the moment the move
  // to `.1` fails. A hook that held the link under its own name then would find that log in the way of its give-back and
  // copy the person's file into it through the link, then remove the link. The link is never taken, so the live name is
  // never free for that log: nothing is opened through the link, and the link stays.
  it("opens nothing through a full log linked at the live name when another hook begins a live log as its move to .1 fails", async context => {
    const rerailHome = await home();
    const live = join(rerailHome, HOOK_FAILURES_FILE);
    const { elsewhere, text } = await linkLogElsewhere(context, rerailHome, 256 * 1024 + 1);
    const opened = await recordWithWrapped(rerailHome, 2, {
      ...refusedThrough(elsewhere),
      renameSync: `rename => (from, to) => {
        if (to.endsWith(".old")) fs.writeSync(1, "took ");
        if (!to.endsWith(".1")) return rename(from, to);
        if (fs.lstatSync(${JSON.stringify(live)}, { throwIfNoEntry: false }) === undefined) fs.appendFileSync(${JSON.stringify(live)}, ${lineSource(3, "began a live log")});
        throw Object.assign(new Error("EISDIR: illegal operation on a directory, rename"), { code: "EISDIR" });
      }`,
    });
    expect({ opened, ...leftInPlace(rerailHome, elsewhere, text) }).toEqual({
      opened: "", live: "link", leadsTo: elsewhere, files: [HOOK_FAILURES_FILE], counted: full, leftAsItWas: true,
    });
  });

  // A person who keeps the log elsewhere makes the live name a link to it at the moment a hook, which found the Rerail
  // home's own log full there, takes that name, so the hook takes the link. It puts the link back as it was and moves it
  // as `main` moves a link; with a folder at `.1` that move fails, and the hook writes nothing, as `main`'s does. Given
  // back as a log is and written through, the link would take the hook's failure into the person's file.
  it("puts back a link a person makes at the live name as a hook takes the log, and writes nothing through it", async context => {
    const rerailHome = await home();
    const live = join(rerailHome, HOOK_FAILURES_FILE);
    writeFullLog(rerailHome);
    mkdirSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`));
    const { elsewhere, text, made } = await linkMadeLater(context, rerailHome, full);
    const opened = await recordWithWrapped(rerailHome, 2, {
      ...refusedThrough(elsewhere),
      renameSync: `rename => (from, to) => {
        if (from === ${JSON.stringify(live)} && to.endsWith(".old") && !globalThis.made) {
          globalThis.made = true;
          rename(${JSON.stringify(made)}, from);
        }
        return rename(from, to);
      }`,
    });
    expect({ opened, ...leftInPlace(rerailHome, elsewhere, text) }).toEqual({
      opened: "", live: "link", leadsTo: elsewhere, files: [HOOK_FAILURES_FILE, `${HOOK_FAILURES_FILE}.1`], counted: full, leftAsItWas: true,
    });
  });

  // A person makes the live name a link to a file they keep elsewhere while a hook holds the log it took and gives it
  // back, here because `.1` refuses it. The give-back finds the link in its way: it copies nothing into the person's
  // file and writes nothing through the link, and the log stays under the hook's name, where status counts it.
  it("copies nothing through a link a person makes at the live name while a hook gives the log back", async context => {
    const rerailHome = await home();
    const live = join(rerailHome, HOOK_FAILURES_FILE);
    writeFullLog(rerailHome);
    const { elsewhere, text, made } = await linkMadeLater(context, rerailHome, 3);
    const opened = await recordWithWrapped(rerailHome, 2, {
      ...refusedThrough(elsewhere),
      renameSync: `rename => (from, to) => {
        if (!to.endsWith(".1")) return rename(from, to);
        throw Object.assign(new Error("EPERM: operation not permitted, rename"), { code: "EPERM" });
      }`,
      linkSync: `link => (existing, path) => {
        if (path === ${JSON.stringify(live)} && !globalThis.made) {
          globalThis.made = true;
          fs.renameSync(${JSON.stringify(made)}, path);
        }
        return link(existing, path);
      }`,
    });
    expect({ opened, ...leftInPlace(rerailHome, elsewhere, text) }).toEqual({
      opened: "", live: "link", leadsTo: elsewhere, files: [HOOK_FAILURES_FILE, expect.stringMatching(/^hook-failures\.jsonl\.\d+\.[0-9a-f-]+\.old$/u)],
      counted: full + 3, leftAsItWas: true,
    });
  });

  // As above, but the person makes the link once a give-back by copy has begun, as it waits for the copy lock or reads
  // the log it took: here another hook began a live log as the move to `.1` was refused, and the person's link replaces
  // that log as the give-back reads. The give-back looks at the live name again under its lock, right before the lines
  // leave the hook's name: it copies nothing into the person's file, and the hook writes nothing through the link.
  it("copies nothing through a link a person makes at the live name once a give-back by copy has begun", async context => {
    const rerailHome = await home();
    const live = join(rerailHome, HOOK_FAILURES_FILE);
    writeFullLog(rerailHome);
    const { elsewhere, text, made } = await linkMadeLater(context, rerailHome, 3);
    const opened = await recordWithWrapped(rerailHome, 2, {
      ...refusedThrough(elsewhere),
      renameSync: `rename => (from, to) => {
        if (!to.endsWith(".1")) return rename(from, to);
        fs.appendFileSync(${JSON.stringify(live)}, ${lineSource(3, "began a live log")});
        throw Object.assign(new Error("EPERM: operation not permitted, rename"), { code: "EPERM" });
      }`,
      fstatSync: `fstat => (...args) => {
        if (!globalThis.made) {
          globalThis.made = true;
          fs.renameSync(${JSON.stringify(made)}, ${JSON.stringify(live)});
        }
        return fstat(...args);
      }`,
    });
    expect({ opened, ...leftInPlace(rerailHome, elsewhere, text) }).toEqual({
      opened: "", live: "link", leadsTo: elsewhere, files: [HOOK_FAILURES_FILE, expect.stringMatching(/^hook-failures\.jsonl\.\d+\.[0-9a-f-]+\.old$/u)],
      counted: full + 3, leftAsItWas: true,
    });
  });

  // Another hook moves a full log a person linked at the live name and begins a new log there in the moment between this
  // hook's look, which found the link, and its rename, as `main`'s hooks can. This hook's rename then moves that new log,
  // a regular file of the Rerail home, over the link at `.1`, as `main`'s does, and status counts it there.
  it("moves the log another hook began in place of a linked live name to .1 over the link, and counts it there", async context => {
    const rerailHome = await home();
    const live = join(rerailHome, HOOK_FAILURES_FILE);
    const { elsewhere, text } = await linkLogElsewhere(context, rerailHome, 256 * 1024 + 1);
    const opened = await recordWithWrapped(rerailHome, 2, {
      ...refusedThrough(elsewhere),
      renameSync: `rename => (from, to) => {
        if (from === ${JSON.stringify(live)} && to.endsWith(".1") && !globalThis.begun) {
          globalThis.begun = true;
          rename(from, to);
          fs.appendFileSync(from, ${lineSource(5, "began a live log")} + ${lineSource(6, "began a live log")});
        }
        return rename(from, to);
      }`,
    });
    expect({ opened, ...afterTheMove(rerailHome), leftAsItWas: readFileSync(elsewhere, "utf8") === text }).toEqual({
      opened: "", live: "file", movedAside: "file", files: [HOOK_FAILURES_FILE, `${HOOK_FAILURES_FILE}.1`], counted: 3, leftAsItWas: true,
    });
  });

  // A person linked the live name to a file of over 1 MB, more than hooks write before one of them moves the log. The
  // link moves to `.1` as it is, whatever the size of what it leads to.
  it("moves a file of over 1 MB a person linked at the live name to .1 as the link", async context => {
    const rerailHome = await home();
    const { elsewhere, text } = await linkLogElsewhere(context, rerailHome, 1024 * 1024 + 1);
    recordLast(rerailHome, 2);
    expect({ ...afterTheMove(rerailHome), leftAsItWas: readFileSync(elsewhere, "utf8") === text }).toEqual({ ...movedAsTheLink(1), leftAsItWas: true });
  });

  // A running hook took the full log a moment ago, though no hook wrote to it for a day, and a hook held up since it
  // opened the log before the take writes into it after another hook moved a log aside. Removed for its write time, the
  // log would take that failure with it. The take comes 100 ms after the change time the log had before it, and the move
  // runs a day and 50 ms past that change time: only a change time the take set keeps the log.
  it("keeps the failure a held-up hook writes into a log a running hook took, though the log was last written a day ago", async () => {
    const rerailHome = await home();
    const live = join(rerailHome, HOOK_FAILURES_FILE);
    writeFullLog(rerailHome);
    const dayAgo = (Date.now() - 25 * 60 * 60 * 1_000) / 1_000;
    utimesSync(live, dayAgo, dayAgo);
    const beforeTheTake = lstatSync(live).ctimeMs;
    await new Promise(resolveWait => { setTimeout(resolveWait, 100); });
    const heldUp = openSync(live, "a");
    try {
      renameSync(live, join(rerailHome, `${HOOK_FAILURES_FILE}.${process.ppid}.${randomUUID()}.old`));
      writeFullLog(rerailHome);
      recordLast(rerailHome, beforeTheTake + 24 * 60 * 60 * 1_000 + 50);
      writeSync(heldUp, `${JSON.stringify(failure(Date.now(), "HeldUp"))}\n`);
    } finally {
      closeSync(heldUp);
    }
    expect(readHookFailures(rerailHome, Date.now() - 60_000).failures.map(entry => entry.event)).toContain("HeldUp");
  });

  // A file system that keeps no change time, as FAT does not, here gives every log a hook took a change time of 0. The
  // move then reads the write time: a log a running hook wrote into a minute ago is kept, and one last written over a day
  // ago is removed. Read by its change time alone, the first would go too, with what the running hook still writes.
  it("keeps a log a hook wrote into within the day where the file system keeps no change time, and removes an older one", async () => {
    const rerailHome = await home();
    // The removal reads no pid: `recent` stays for its write time alone, whatever process has its pid.
    const [recent, old] = [takenName(rerailHome, process.pid), takenName(rerailHome, 2 ** 31 - 1)];
    writeFileSync(old, `${fullLine}\n`);
    const dayAgo = (Date.now() - 25 * 60 * 60 * 1_000) / 1_000;
    utimesSync(old, dayAgo, dayAgo);
    writeFileSync(recent, `${fullLine}\n`);
    utimesSync(recent, (Date.now() - 60_000) / 1_000, (Date.now() - 60_000) / 1_000);
    // Both stat calls, so the test holds whichever one the move reads.
    const noChangeTime = (call: string): string => `${call} => (path, ...rest) => {
      const stats = ${call}(path, ...rest);
      if (String(path).endsWith(".old") && rest[0]?.bigint !== true) stats.ctimeMs = 0;
      return stats;
    }`;
    writeFullLog(rerailHome);
    await recordWithWrapped(rerailHome, Date.now(), { statSync: noChangeTime("stat"), lstatSync: noChangeTime("lstat") });
    expect(logFiles(rerailHome)).toEqual([HOOK_FAILURES_FILE, `${HOOK_FAILURES_FILE}.1`, basename(recent)].sort());
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

  // #503's writer moves the full log to `.1` with no start line, and a hook was stopped holding an older log.
  it("says since when it counts from the oldest failure of a .1 with no start, not from an older log a hook was stopped holding", async () => {
    const rerailHome = await home();
    const lines = (...ats: number[]): string => ats.map(at => `${JSON.stringify(failure(at))}\n`).join("");
    writeFileSync(join(rerailHome, `${HOOK_FAILURES_FILE}.${2 ** 31 - 1}.${randomUUID()}.old`), lines(2_000));
    writeFileSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`), lines(5_000, 6_000));
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), lines(7_000));
    expect(statusSays(rerailHome, 0, afterEveryFailure)).toEqual([expect.stringMatching(
      /^Hooks: 3 hook events could not be recorded since 1970-01-01T00:00:05\.000Z, as far back as the log goes; /u,
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

  // A hook gives the full log back by copy, as another hook began a live log since it took it, and the read of its lines
  // fails, as on an I/O error: they stay under the hook's name, where status counts them.
  it("keeps the lines of a log it gives back by copy when it cannot read them", async () => {
    const rerailHome = await home();
    writeFullLog(rerailHome);
    await recordWithWrapped(rerailHome, 2, {
      renameSync: `rename => (from, to) => {
        if (to.endsWith(".1")) throw Object.assign(new Error("EPERM: operation not permitted, rename"), { code: "EPERM" });
        rename(from, to);
        if (to.endsWith(".old")) fs.appendFileSync(from, ${lineSource(3, "began a live log")});
      }`,
      openSync: `open => (path, ...rest) => {
        const descriptor = open(path, ...rest);
        if (String(path).endsWith(".old")) globalThis.taken = descriptor;
        return descriptor;
      }`,
      readSync: `read => (descriptor, ...rest) => {
        if (descriptor === globalThis.taken) throw Object.assign(new Error("EIO: i/o error, read"), { code: "EIO" });
        return read(descriptor, ...rest);
      }`,
    });
    expect(logFiles(rerailHome)).toEqual([HOOK_FAILURES_FILE, expect.stringMatching(/^hook-failures\.jsonl\.\d+\.[0-9a-f-]+\.old$/u)]);
    expect(statusCount(rerailHome)).toBe(full + 2);
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

  // A log under the name of a hook that took it, `<log>.<pid>.<id>.old`.
  const takenName = (rerailHome: string, pid: number): string => join(rerailHome, `${HOOK_FAILURES_FILE}.${pid}.${randomUUID()}.old`);

  // On Windows a rename opens the file by name and renames what it opened, so a hook that opened the full log before
  // another took it moves it on from that hook's name to its own. Here it does so as status lists the Rerail home.
  it("counts the log a hook moves on from another hook's name while status lists the home", async () => {
    const rerailHome = await home();
    writeFullLog(rerailHome);
    const [first, second] = [takenName(rerailHome, 1_000), takenName(rerailHome, 1_004)];
    renameSync(join(rerailHome, HOOK_FAILURES_FILE), first);
    appendFileSync(join(rerailHome, HOOK_FAILURES_FILE), `${JSON.stringify(failure(2))}\n`);
    const said = await statusWhileListing(rerailHome, `readdir => (...args) => {
      const names = readdir(...args);
      if (String(args[0]) === HOME && fs.existsSync(${JSON.stringify(first)})) fs.renameSync(${JSON.stringify(first)}, ${JSON.stringify(second)});
      return names;
    }`);
    expect(countOf(said)).toBe(full + 1);
  });

  // As above, but the hook moves the log on after status found it under the first hook's name and before status opens it.
  it("counts the log a hook moves on from another hook's name between status finding it and reading it", async () => {
    const rerailHome = await home();
    writeFullLog(rerailHome);
    const [first, second] = [takenName(rerailHome, 1_000), takenName(rerailHome, 1_004)];
    renameSync(join(rerailHome, HOOK_FAILURES_FILE), first);
    appendFileSync(join(rerailHome, HOOK_FAILURES_FILE), `${JSON.stringify(failure(2))}\n`);
    const { said } = await statusWith(rerailHome, {
      lstatSync: `lstat => (path, ...rest) => {
        const stats = lstat(path, ...rest);
        if (String(path) === ${JSON.stringify(first)} && !globalThis.moved) {
          globalThis.moved = true;
          fs.renameSync(${JSON.stringify(first)}, ${JSON.stringify(second)});
        }
        return stats;
      }`,
    });
    expect(countOf(said)).toBe(full + 1);
  });

  // Status reads the live log before it lists the others: a log hooks take as it lists them, and move on from name to
  // name, was met already.
  it("counts the live log hooks take and move from name to name at each listing while status reads", async () => {
    const rerailHome = await home();
    writeFullLog(rerailHome);
    const said = await statusWhileListing(rerailHome, `readdir => (...args) => {
      const names = readdir(...args);
      if (String(args[0]) !== HOME) return names;
      const from = globalThis.logAt ?? HOME + "/${HOOK_FAILURES_FILE}";
      globalThis.logAt = HOME + "/${HOOK_FAILURES_FILE}." + process.pid + "." + crypto.randomUUID() + ".old";
      fs.renameSync(from, globalThis.logAt);
      return names;
    }`);
    expect(countOf(said)).toBe(full);
  });

  // On Windows every hook that opened the full log by name as it filled moves it on in turn, so a burst of hooks keeps it
  // moving from one hook's name to the next for many milliseconds. Here it moves on at each listing for the first 20 ms.
  it("counts a log hooks keep moving from name to name for a while as status lists the home", async () => {
    const rerailHome = await home();
    writeFullLog(rerailHome);
    renameSync(join(rerailHome, HOOK_FAILURES_FILE), takenName(rerailHome, 1_000));
    const said = await statusWhileListing(rerailHome, `readdir => (...args) => {
      if (String(args[0]) !== HOME) return readdir(...args);
      globalThis.firstListing ??= Date.now();
      const names = readdir(...args);
      const held = names.find(name => name.endsWith(".old"));
      if (Date.now() - globalThis.firstListing < 20) fs.renameSync(HOME + "/" + held, HOME + "/${HOOK_FAILURES_FILE}." + process.pid + "." + crypto.randomUUID() + ".old");
      return names;
    }`);
    expect(countOf(said)).toBe(full);
  });

  // A part of the log as a hook that moved the one before aside begins it: its start, then failures at `ats`.
  const partText = (startedAt: number, ...ats: number[]): string => [{ startedAt }, ...ats.map(at => failure(at))].map(line => `${JSON.stringify(line)}\n`).join("");

  // A hook on Windows can move `.1` on to its own name for a moment, and back. Here it does so as status lists the home:
  // away just after the first listing, back just before the third, so the second finds it under the hook's name. The
  // hook still runs: this test's own process stands for it.
  it("says since when it counts when a hook holds .1 under its own name as status lists the home", async () => {
    const rerailHome = await home();
    writeFileSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`), partText(5_000, 5_000, 6_000));
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), partText(9_000, 9_000));
    const [movedAside, held] = [join(rerailHome, `${HOOK_FAILURES_FILE}.1`), takenName(rerailHome, process.pid)];
    const said = await statusWhileListing(rerailHome, `readdir => (...args) => {
      if (String(args[0]) !== HOME) return readdir(...args);
      globalThis.listings = (globalThis.listings ?? 0) + 1;
      if (globalThis.listings === 3) fs.renameSync(${JSON.stringify(held)}, ${JSON.stringify(movedAside)});
      const names = readdir(...args);
      if (globalThis.listings === 1) fs.renameSync(${JSON.stringify(movedAside)}, ${JSON.stringify(held)});
      return names;
    }`);
    expect(said).toMatch(/^Hooks: 3 hook events could not be recorded since 1970-01-01T00:00:05\.000Z, as far back as the log goes; /u);
  });

  // As above, with a `.1` last written 2.5 s before status reads, as a `.1` in use is: only the take's rename makes it
  // a log that changed lately, so this fails if the wait reads the write time where it must read the change time.
  it("says since when it counts when a hook holds a .1 last written seconds ago under its own name as status lists the home", async () => {
    const rerailHome = await home();
    writeFileSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`), partText(5_000, 5_000, 6_000));
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), partText(9_000, 9_000));
    await new Promise(resolveWait => { setTimeout(resolveWait, 2_500); });
    const [movedAside, held] = [join(rerailHome, `${HOOK_FAILURES_FILE}.1`), takenName(rerailHome, process.pid)];
    const said = await statusWhileListing(rerailHome, `readdir => (...args) => {
      if (String(args[0]) !== HOME) return readdir(...args);
      globalThis.listings = (globalThis.listings ?? 0) + 1;
      if (globalThis.listings === 3) fs.renameSync(${JSON.stringify(held)}, ${JSON.stringify(movedAside)});
      const names = readdir(...args);
      if (globalThis.listings === 1) fs.renameSync(${JSON.stringify(movedAside)}, ${JSON.stringify(held)});
      return names;
    }`);
    expect(said).toMatch(/^Hooks: 3 hook events could not be recorded since 1970-01-01T00:00:05\.000Z, as far back as the log goes; /u);
  });

  // A hook stopped while it held `.1` under its own name leaves no `.1`: nothing moves the log it left until the next
  // move, so status does not wait for it.
  it("reads the log at once when a hook that has ended holds what was .1", async () => {
    const rerailHome = await home();
    writeFileSync(join(rerailHome, `${HOOK_FAILURES_FILE}.${2 ** 31 - 1}.${randomUUID()}.old`), partText(5_000, 5_000, 6_000));
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), partText(9_000, 9_000));
    const { said, listings } = await statusWith(rerailHome, {});
    expect({ count: countOf(said), listings }).toEqual({ count: 3, listings: 1 });
  });

  // As above, but another process has the stopped hook's pid since, as Windows soon hands out a freed pid again: this
  // test's own process stands for it, and status reads an hour after the log last changed.
  it("reads the log at once when the log a stopped hook left in place of .1 names a pid another process has now", async () => {
    const rerailHome = await home();
    writeFileSync(takenName(rerailHome, process.pid), partText(5_000, 5_000, 6_000));
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), partText(9_000, 9_000));
    const { said, listings } = await statusWith(rerailHome, {}, Date.now() + 60 * 60_000);
    expect({ count: countOf(said), listings }).toEqual({ count: 3, listings: 1 });
  });

  // A name of the log left as a link whose target is gone, as a link to a drive no longer there, at `.1` or under the
  // name of a hook that still runs (this test's own process stands for it). It holds no part of the log, and status reads
  // it from the name itself: `fs.statSync` on it would follow it, and on Windows a link to a network share that does not
  // answer makes that wait about 21 s. On Windows the link to a folder is a junction, which needs no rights to make and
  // which no hook can move a log over.
  it.for(LINKS.flatMap(link => [
    { ...link, where: ".1", name: (rerailHome: string) => join(rerailHome, `${HOOK_FAILURES_FILE}.1`) },
    { ...link, where: "a running hook's own name", name: (rerailHome: string) => takenName(rerailHome, process.pid) },
  ]))("reads the log at once, never through the link, when $where is $link whose target is gone", async ({ make, name }, context) => {
    const rerailHome = await home();
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), partText(9_000, 9_000, 9_500));
    const link = name(rerailHome);
    make(context, join(rerailHome, "gone", "log"), link);
    const { said, listings, followed } = await statusWith(rerailHome, { statSync: `stat => (path, ...rest) => {
      if (String(path) === ${JSON.stringify(link)}) globalThis.followed = (globalThis.followed ?? 0) + 1;
      return stat(path, ...rest);
    }` });
    expect({ count: countOf(said), listings, followed }).toEqual({ count: 2, listings: 1, followed: 0 });
  });

  // A name of the log a person or another program made a named pipe that no program writes to, or a folder, at `.1` or
  // under the name of a hook that still runs (this test's own process stands for it). Neither holds a part of the log.
  // A pipe opened to read waits for a writer, and a folder at `.1` read as a part that holds nothing would make the line
  // count from the live part's start. Windows makes no named pipe in a folder.
  const NOT_FILES = [
    {
      kind: "a named pipe",
      make: (context: TestContext, path: string): void => {
        if (process.platform === "win32") context.skip("Windows makes no named pipe in a folder");
        execFileSync("mkfifo", [path]);
      },
    },
    { kind: "a folder", make: (_context: TestContext, path: string): void => { mkdirSync(path); } },
  ];
  it.for(NOT_FILES.flatMap(kind => [
    { ...kind, where: ".1", name: (rerailHome: string) => join(rerailHome, `${HOOK_FAILURES_FILE}.1`) },
    { ...kind, where: "a running hook's own name", name: (rerailHome: string) => takenName(rerailHome, process.pid) },
  ]))("reads the log at once, opening nothing else, when $where is $kind", async ({ make, name }, context) => {
    const rerailHome = await home();
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), partText(9_000, 9_000, 9_500));
    make(context, name(rerailHome));
    const { said, listings } = await statusWith(rerailHome, {});
    expect({ said, listings }).toEqual({
      said: expect.stringMatching(/^Hooks: 2 hook events could not be recorded in the last 24 hours; /u),
      listings: 1,
    });
  });

  // The size check found a full log through the live name's link, and by the time the hook moves the link, it leads to
  // a named pipe no program writes to, or to a folder, as when a person points it elsewhere in that moment; here the
  // size check is told the log is full. The link moves on to `.1` as it is, as `main` moves it right after its one size
  // check. A second size check through the link would find no full log and give the link back, and the hook's own
  // failure would follow it: into the folder, where it is lost, or into the pipe, where the hook waits for a reader. On
  // Windows the link to a folder is a junction.
  it.for(NOT_FILES)("moves the live name's link to .1 at once when it leads to $kind by the time the hook moves it", async ({ kind, make }, context) => {
    const rerailHome = await home();
    const live = join(rerailHome, HOOK_FAILURES_FILE);
    const target = join(await home(), "target");
    make(context, target);
    if (kind === "a folder") linkFolder(target, live);
    else symlinkOrSkip(context, target, live, "file");
    await recordWithWrapped(rerailHome, 2, { statSync: `stat => (path, ...rest) => {
      const stats = stat(path, ...rest);
      if (String(path) === ${JSON.stringify(live)}) stats.size = ${300 * 1024};
      return stats;
    }` });
    expect(afterTheMove(rerailHome)).toEqual(movedAsTheLink(1));
  });

  // Another program holds `.1` for a moment, as a backup or a virus scan can on Windows, while a hook moves a full log a
  // person linked at the live name. The link's move retries, as `main`'s one rename does, so the hook keeps its failure
  // in the Rerail home. Only Windows retries a refused rename.
  it("moves a full log linked at the live name to .1 once another program lets go of .1", async context => {
    if (process.platform !== "win32") context.skip("only Windows retries a refused rename");
    const rerailHome = await home();
    const { elsewhere, text } = await linkLogElsewhere(context, rerailHome, 256 * 1024 + 1);
    const refused = await recordWithRename(rerailHome, 2, `rename => (from, to) => {
      if (!to.endsWith(".1") || globalThis.refused) return rename(from, to);
      globalThis.refused = true;
      fs.writeSync(1, "refused ");
      throw Object.assign(new Error("EPERM: operation not permitted, rename"), { code: "EPERM" });
    }`);
    expect({ refused, ...afterTheMove(rerailHome), leftAsItWas: readFileSync(elsewhere, "utf8") === text }).toEqual({
      refused: "refused ", ...movedAsTheLink(1), leftAsItWas: true,
    });
  });

  // A hook that took a log and finds it small gives it back: it makes the live name a second name of the file, a hard
  // link, then removes its own. Status finds no live log, then lists the home between those two steps, so it meets the
  // log only under those two names, each a file of its own and no link.
  it("counts the log a hook gives back as status lists the home, while the live name and the hook's own name are one file", async () => {
    const rerailHome = await home();
    const [live, taken] = [join(rerailHome, HOOK_FAILURES_FILE), takenName(rerailHome, process.pid)];
    writeFileSync(taken, partText(9_000, 9_000, 9_500));
    const { said, listings } = await statusWith(rerailHome, { readdirSync: `readdir => (...args) => {
      if (String(args[0]) === HOME && !fs.existsSync(${JSON.stringify(live)})) fs.linkSync(${JSON.stringify(taken)}, ${JSON.stringify(live)});
      return readdir(...args);
    }` });
    expect({ count: countOf(said), listings }).toEqual({ count: 2, listings: 1 });
  });

  // A hook that still runs takes `.1` under its own name just before status lists the home, and a second one moves it on
  // from there to its own name before it goes back to `.1`. Both hooks still run: this test's own process stands for them.
  it("says since when it counts when running hooks move .1 from name to name from before status lists the home", async () => {
    const rerailHome = await home();
    writeFileSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`), partText(5_000, 5_000, 6_000));
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), partText(9_000, 9_000));
    const [movedAside, first, second] = [join(rerailHome, `${HOOK_FAILURES_FILE}.1`), takenName(rerailHome, process.pid), takenName(rerailHome, process.pid)];
    const said = await statusWhileListing(rerailHome, `readdir => (...args) => {
      if (String(args[0]) !== HOME) return readdir(...args);
      globalThis.listings = (globalThis.listings ?? 0) + 1;
      const move = { 1: [${JSON.stringify(movedAside)}, ${JSON.stringify(first)}], 2: [${JSON.stringify(first)}, ${JSON.stringify(second)}], 3: [${JSON.stringify(second)}, ${JSON.stringify(movedAside)}] }[globalThis.listings];
      if (move !== undefined) fs.renameSync(move[0], move[1]);
      return readdir(...args);
    }`);
    expect(said).toMatch(/^Hooks: 3 hook events could not be recorded since 1970-01-01T00:00:05\.000Z, as far back as the log goes; /u);
  });

  // Another program can hold `.1` open on Windows so that no other program can read it, as a backup can.
  it("says since when it counts from the live part, at once, when .1 cannot be read", async () => {
    const rerailHome = await home();
    writeFileSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`), partText(5_000, 5_000, 6_000));
    // The live part's start, not its oldest failure, is the time from which every failure is in it.
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), partText(9_000, 9_500, 10_000));
    const { said, listings } = await statusWith(rerailHome, {
      openSync: `open => (path, ...rest) => {
        if (String(path).endsWith(".1")) throw Object.assign(new Error("EBUSY: resource busy or locked, open"), { code: "EBUSY" });
        return open(path, ...rest);
      }`,
    });
    expect({ said, listings }).toEqual({
      said: expect.stringMatching(/^Hooks: 2 hook events could not be recorded since 1970-01-01T00:00:09\.000Z, as far back as the log goes; /u),
      listings: 1,
    });
  });

  // As above, but `.1` opens and then cannot be read, as on an I/O error or, on Windows, a range of it another program
  // locked. The first call on `.1`'s descriptor after the open, fstat, fails here; a failed read takes the same path.
  it("says since when it counts from the live part, at once, when .1 opens but cannot be read", async () => {
    const rerailHome = await home();
    writeFileSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`), partText(5_000, 5_000, 6_000));
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), partText(9_000, 9_500, 10_000));
    const { said, listings } = await statusWith(rerailHome, {
      openSync: `open => (path, ...rest) => {
        const descriptor = open(path, ...rest);
        if (String(path).endsWith(".1")) globalThis.movedAside = descriptor;
        return descriptor;
      }`,
      fstatSync: `fstat => (descriptor, ...rest) => {
        if (descriptor === globalThis.movedAside) throw Object.assign(new Error("EIO: i/o error, fstat"), { code: "EIO" });
        return fstat(descriptor, ...rest);
      }`,
    });
    expect({ said, listings }).toEqual({
      said: expect.stringMatching(/^Hooks: 2 hook events could not be recorded since 1970-01-01T00:00:09\.000Z, as far back as the log goes; /u),
      listings: 1,
    });
  });

  // A hook takes the full log while it is also the live log under a second name, cannot move it to `.1`, and gives it
  // back by copy. As it looks for the log's other names, another hook takes the live log: the first must find the log
  // under the name the other took, and copy nothing.
  it("copies nothing back when the log's other name moves on as the hook looks for it", async () => {
    const rerailHome = await home();
    writeFullLog(rerailHome);
    const [live, takenByAnother] = [join(rerailHome, HOOK_FAILURES_FILE), takenName(rerailHome, 1_000)];
    await recordWithWrapped(rerailHome, 2, {
      renameSync: `rename => (from, to) => {
        if (to.endsWith(".1")) throw Object.assign(new Error("EPERM: operation not permitted, rename"), { code: "EPERM" });
        rename(from, to);
        if (to.endsWith(".old")) fs.linkSync(to, from);
      }`,
      // Once the hook has removed its own name, so the copy is under way.
      unlinkSync: `unlink => path => {
        unlink(path);
        if (String(path).endsWith(".old")) globalThis.copying = true;
      }`,
      readdirSync: `readdir => (...args) => {
        const names = readdir(...args);
        if (String(args[0]) === HOME && globalThis.copying && !globalThis.taken) {
          globalThis.taken = true;
          fs.linkSync(${JSON.stringify(live)}, ${JSON.stringify(takenByAnother)});
          fs.unlinkSync(${JSON.stringify(live)});
        }
        return names;
      }`,
    });
    expect(logFiles(rerailHome)).toEqual([HOOK_FAILURES_FILE, expect.stringMatching(/^hook-failures\.jsonl\.1000\.[0-9a-f-]+\.old$/u)]);
    expect(statusCount(rerailHome)).toBe(full + 1);
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
  const child = tracked(spawn(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], {
    cwd: join(import.meta.dirname, "..", ".."), stdio: ["ignore", "ignore", "inherit", "ipc"], windowsHide: true,
  }));
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

/** The `node:fs` functions a test can wrap in a process of its own, each by the source of a function of the real one. */
type Wraps = Partial<Record<"renameSync" | "linkSync" | "appendFileSync" | "unlinkSync" | "readdirSync" | "openSync" | "readSync" | "readFileSync" | "statSync" | "lstatSync" | "fstatSync", string>>;

/**
 * {@link recordWithRename} with each `node:fs` function `wraps` names made by the function source given for it, and
 * `Date.now` fixed at `clock` when given.
 */
async function recordWithWrapped(rerailHome: string, at: number | undefined, wraps: Wraps, clock?: number): Promise<string> {
  const record = `hookFailures.recordHookFailure({ host: "claude-code", event: "PostToolUse", reason: "the daemon did not answer" }, { home: HOME, now: ${at}, env: {} });`;
  return await outputOf(rerailHome, record, wraps, clock);
}

/**
 * What `rerail status` says of the hook failures in `rerailHome` (an event recorded after all of them), how many times it
 * listed the Rerail home, and the count a wrap kept in `globalThis.followed` (0 when none did), when a process of its own
 * reads them with each `node:fs` function `wraps` names made by the function source given for it: a hook moving the log,
 * or another program holding a file, as status reads it. `HOME` in those sources is the Rerail home. `Date.now` is fixed
 * at `clock` when given.
 */
async function statusWith(rerailHome: string, wraps: Wraps, clock?: number): Promise<{ said: string; listings: number; followed: number }> {
  const say = `const list = fs.readdirSync;
let listings = 0;
fs.readdirSync = (...args) => { if (String(args[0]) === HOME) listings += 1; return list(...args); };
syncBuiltinESMExports();
const said = hookFailures.hookFailureLines(hookFailures.readHookFailures(HOME, 0), Number.MAX_SAFE_INTEGER).join("");
fs.writeSync(1, JSON.stringify({ said, listings, followed: globalThis.followed ?? 0 }));`;
  return JSON.parse(await outputOf(rerailHome, say, wraps, clock)) as { said: string; listings: number; followed: number };
}

/** What {@link statusWith} says when `fs.readdirSync` is the function `wrapListing` makes of the real one. */
async function statusWhileListing(rerailHome: string, wrapListing: string): Promise<string> {
  return (await statusWith(rerailHome, { readdirSync: wrapListing })).said;
}

/**
 * What `code` wrote to its standard output, run in a process of its own once `wraps` and `clock` are in place, with
 * `hookFailures` the hook failure module and `HOME` the Rerail home `rerailHome`.
 */
async function outputOf(rerailHome: string, code: string, wraps: Wraps, clock?: number): Promise<string> {
  const program = `import fs from "node:fs";
import { syncBuiltinESMExports } from "node:module";
const HOME = ${JSON.stringify(rerailHome)};
${clock === undefined ? "" : `Date.now = () => ${clock};`}
${Object.entries(wraps).map(([name, wrap]) => `fs.${name} = (${wrap})(fs.${name});`).join("\n")}
syncBuiltinESMExports();
const hookFailures = await import(${JSON.stringify(sourceModule("src/harness/hook-failures.ts"))});
${code}
`;
  const child = tracked(spawn(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], {
    cwd: join(import.meta.dirname, "..", ".."), stdio: ["ignore", "pipe", "inherit"], windowsHide: true,
  }));
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
