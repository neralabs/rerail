import { mkdtempSync, readFileSync, realpathSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ACTIVE_PROFILE_FILE } from "../../src/activation/index.js";
import { startUpdateChecks, UPDATE_CHECK_FILE, updateCheckApplies, type UpdateChecks } from "../../src/setup/update-check.js";
import { UPDATE_STATE_FILE, type PackageRegistry } from "../../src/setup/updater.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const DAY = 24 * 60 * 60_000;
const HOUR = 60 * 60_000;

const cleanups: Array<() => Promise<void> | void> = [];
afterEach(async () => {
  vi.useRealTimers();
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function home(): string {
  const folder = realpathSync(mkdtempSync(join(tmpdir(), "rerail-update-check-")));
  cleanups.push(() => removeFolder(folder));
  return folder;
}

/** npm as the check asks it: each answer in turn (an Error is thrown), with every channel asked about. */
function registry(...answers: Array<string | Error>): PackageRegistry & { asked: string[] } {
  const asked: string[] = [];
  return {
    asked,
    resolveVersion: async channel => {
      asked.push(channel);
      const answer = answers.shift() ?? new Error("no answer left");
      if (answer instanceof Error) throw answer;
      return answer;
    },
    installVersion: async () => { throw new Error("the update check never installs"); },
  };
}

/** A clock the test moves, with the timers moved by as much. */
function clock(): { now: () => number; advance: (ms: number) => Promise<void> } {
  vi.useFakeTimers({ toFake: ["setTimeout", "setInterval", "clearTimeout", "clearInterval"] });
  let at = Date.UTC(2026, 10, 23, 9);
  return {
    now: () => at,
    advance: async ms => { at += ms; await vi.advanceTimersByTimeAsync(ms); },
  };
}

function start(folder: string, npm: PackageRegistry, now: () => number, log: string[] = []): UpdateChecks {
  const checks = startUpdateChecks({ home: folder, registry: npm, now, firstLookMs: 1_000, lookEveryMs: HOUR, log: line => log.push(line) });
  cleanups.push(() => checks.stop());
  return checks;
}

describe("the daemon's daily update check", () => {
  it("asks npm a minute after the start, not before, and reports the answer with when it came", async () => {
    const folder = home(), time = clock(), npm = registry("0.3.0");
    const checks = start(folder, npm, time.now);
    expect(checks.newest()).toBeNull();
    expect(npm.asked).toEqual([]);

    await time.advance(1_000);
    expect(npm.asked).toEqual(["latest"]);
    expect(checks.newest()).toEqual({ latest: "0.3.0", checkedAt: time.now() });
  });

  it("asks once a day, however often the daemon restarts in between", async () => {
    const folder = home(), time = clock(), npm = registry("0.3.0", "0.3.1");
    let checks = start(folder, npm, time.now);
    await time.advance(1_000);
    const answeredAt = time.now();

    for (let restart = 0; restart < 3; restart += 1) {
      checks.stop();
      await time.advance(2 * HOUR);
      checks = start(folder, npm, time.now);
      await time.advance(1_000);
      // A restarted daemon reports the kept answer without asking again.
      expect(checks.newest()).toEqual({ latest: "0.3.0", checkedAt: answeredAt });
    }
    expect(npm.asked).toHaveLength(1);

    await time.advance(DAY);
    expect(npm.asked).toHaveLength(2);
    expect(checks.newest()?.latest).toBe("0.3.1");
  });

  it("keeps the last answer when npm fails, logs one line, and waits a day before asking again", async () => {
    const folder = home(), time = clock(), log: string[] = [];
    const npm = registry("0.3.0", new Error("could not reach the npm registry (ENOTFOUND)\nnpm's long detail"), "0.4.0");
    const checks = start(folder, npm, time.now, log);
    await time.advance(1_000);
    await time.advance(DAY);
    expect(npm.asked).toHaveLength(2);
    expect(checks.newest()?.latest).toBe("0.3.0");
    expect(log).toEqual([expect.stringContaining("could not reach the npm registry (ENOTFOUND)")]);
    expect(log[0]).not.toContain("long detail");

    // A restart right after the failure does not ask again: the failed ask was kept too.
    checks.stop();
    const restarted = start(folder, npm, time.now, log);
    await time.advance(1_000);
    expect(npm.asked).toHaveLength(2);
    expect(restarted.newest()?.latest).toBe("0.3.0");

    await time.advance(DAY);
    expect(restarted.newest()?.latest).toBe("0.4.0");
  });

  it("asks on the channel rerail update installs from, and asks again at the next look when it changes", async () => {
    const folder = home(), time = clock(), npm = registry("0.3.0-next.2", "0.3.0");
    writeFileSync(join(folder, UPDATE_STATE_FILE), JSON.stringify({ schemaVersion: 1, channel: "next", current: { version: "0.2.1", packageRoot: folder } }));
    const checks = start(folder, npm, time.now);
    await time.advance(1_000);
    expect(checks.newest()?.latest).toBe("0.3.0-next.2");

    writeFileSync(join(folder, UPDATE_STATE_FILE), JSON.stringify({ schemaVersion: 1, channel: "latest", current: { version: "0.2.1", packageRoot: folder } }));
    // The channel left behind is not reported in the meantime, nor by a daemon restarted before its first look.
    expect(checks.newest()).toBeNull();
    const restarted = start(folder, npm, time.now);
    expect(restarted.newest()).toBeNull();
    restarted.stop();

    await time.advance(HOUR);
    expect(npm.asked).toEqual(["next", "latest"]);
    expect(checks.newest()?.latest).toBe("0.3.0");
  });

  it("stops asking and reporting once a signed customer profile is activated while it runs", async () => {
    const folder = home(), time = clock(), npm = registry("0.3.0", "0.4.0");
    const checks = start(folder, npm, time.now);
    await time.advance(1_000);
    expect(checks.newest()?.latest).toBe("0.3.0");

    writeFileSync(join(folder, ACTIVE_PROFILE_FILE), "acme\n");
    expect(checks.newest()).toBeNull();
    await time.advance(2 * DAY);
    expect(npm.asked).toHaveLength(1);
  });

  it("keeps its record private to the account, and starts over from one it cannot read", async () => {
    const folder = home(), time = clock(), npm = registry("0.3.0");
    writeFileSync(join(folder, UPDATE_CHECK_FILE), "not json");
    start(folder, npm, time.now);
    await time.advance(1_000);
    expect(npm.asked).toHaveLength(1);
    expect(JSON.parse(readFileSync(join(folder, UPDATE_CHECK_FILE), "utf8"))).toMatchObject({ channel: "latest", latest: "0.3.0" });
    if (process.platform !== "win32") expect(statSync(join(folder, UPDATE_CHECK_FILE)).mode & 0o777).toBe(0o600);
  });

  it("drops an answer that comes after the daemon stopped", async () => {
    const folder = home(), time = clock();
    let answer: (version: string) => void = () => undefined;
    const npm: PackageRegistry = {
      resolveVersion: async () => await new Promise<string>(resolve => { answer = resolve; }),
      installVersion: async () => undefined,
    };
    const checks = start(folder, npm, time.now);
    await time.advance(1_000);
    checks.stop();
    answer("0.3.0");
    await time.advance(0);
    expect(checks.newest()).toBeNull();
  });
});

describe("whether the daemon checks at all", () => {
  it("checks unless RERAIL_UPDATE_CHECK is 0 or false, or a signed customer profile sets the channel", () => {
    const folder = home();
    expect(updateCheckApplies(folder, {})).toBe(true);
    expect(updateCheckApplies(folder, { RERAIL_UPDATE_CHECK: "1" })).toBe(true);
    expect(updateCheckApplies(folder, { RERAIL_UPDATE_CHECK: "0" })).toBe(false);
    expect(updateCheckApplies(folder, { RERAIL_UPDATE_CHECK: "false" })).toBe(false);
    writeFileSync(join(folder, ACTIVE_PROFILE_FILE), "acme\n");
    expect(updateCheckApplies(folder, {})).toBe(false);
  });
});
