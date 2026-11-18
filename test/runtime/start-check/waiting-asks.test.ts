import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { makeAnswersFolder } from "../../../src/runtime/start-check/stopped-answers.js";
import { HookAsk, ServedAsks } from "../../../src/runtime/start-check/waiting-asks.js";
import { removeFolderSync } from "../../support/platform/remove-folder.js";

/** The daemon `daemon.json` names in each test's Rerail home. */
const DAEMON = { pid: 4242, startedAt: 7 };

let home = "";

/** The name a folder's kept answer and its marks go by in the daemon's folder (`answerName`). */
function nameOf(folder: string): string {
  return createHash("sha256").update(folder).digest("hex").slice(0, 32);
}

const asksFolder = (): string => join(home, "stopped-answers", `${DAEMON.pid}-${DAEMON.startedAt}`, "asks");

/**
 * The marks of `folder`'s start checks in the daemon's folder: `asking` where a hook's pending mark (`<name>.<hook>.asking`)
 * is there, `waiting`, or neither.
 */
function marksOf(folder: string): string[] {
  const name = nameOf(folder);
  const entries = existsSync(asksFolder()) ? readdirSync(asksFolder()) : [];
  const asking = entries.some(entry => entry.startsWith(`${name}.`) && entry.endsWith(".asking"));
  return [...(asking ? ["asking"] : []), ...(entries.includes(`${name}.waiting`) ? ["waiting"] : [])];
}

/** A daemon started in the home, as it makes its folders, with an answer of stopped kept for `folder`. */
function daemonWithKeptAnswer(folder: string): ServedAsks {
  makeAnswersFolder(home);
  const asks = new ServedAsks();
  asks.open(home, DAEMON);
  writeFileSync(join(home, "stopped-answers", `${DAEMON.pid}-${DAEMON.startedAt}`, `${nameOf(folder)}.json`), "{}");
  return asks;
}

/** A hook's ask in `folder` that may repeat the kept answer, once it learned the live daemon and before it asked. */
function asked(folder: string): { ask: HookAsk; asks: boolean } {
  const ask = new HookAsk(() => true);
  return { ask, asks: ask.beforeAsking(DAEMON, folder) };
}

describe("the start checks hooks gave up on", () => {
  beforeEach(() => {
    home = realpathSync.native(mkdtempSync(join(tmpdir(), "rerail-waiting-asks-")));
    vi.stubEnv("RERAIL_HOME", home);
  });
  afterEach(() => {
    vi.unstubAllEnvs();
    removeFolderSync(home);
  });

  it("leaves one waiting mark where two hooks of one folder ask at once and both give up after they sent", () => {
    const folder = join(home, "app");
    daemonWithKeptAnswer(folder);
    const [first, second] = [asked(folder), asked(folder)];
    const pending = marksOf(folder);
    for (const { ask } of [first, second]) {
      ask.sent();
      ask.abandoned();
    }
    expect({ asks: [first.asks, second.asks], pending, after: marksOf(folder), files: readdirSync(asksFolder()).length })
      .toEqual({ asks: [true, true], pending: ["asking"], after: ["waiting"], files: 1 });
  });

  it("leaves no mark where the hook ends a request it never handed to the OS, which the daemon never gets", () => {
    const folder = join(home, "app");
    daemonWithKeptAnswer(folder);
    const { ask } = asked(folder);
    ask.abandoned();
    expect(marksOf(folder)).toEqual([]);
  });

  it("marks the next hook's check as waiting where a hook killed after its pending mark left it: that mark claims nothing and stops no other hook, and the daemon removes both as it serves a start check there", () => {
    const folder = join(home, "app");
    const daemon = daemonWithKeptAnswer(folder);
    // Killed before it sent: its pending mark stays.
    asked(folder);
    const next = asked(folder);
    next.ask.sent();
    next.ask.abandoned();
    const left = marksOf(folder);
    const after = asked(folder);
    daemon.served(folder);
    expect({ asks: next.asks, left, after: after.asks, served: marksOf(folder) }).toEqual({ asks: true, left: ["asking", "waiting"], after: false, served: [] });
  });

  it.each(["abandoned", "done"] as const)("marks nothing and asks nothing once its hook ended the ask (%s), as a start check that goes on after its hook answered", end => {
    const folder = join(home, "app");
    daemonWithKeptAnswer(folder);
    const ask = new HookAsk(() => true);
    ask[end]();
    expect({ asks: ask.beforeAsking(DAEMON, folder), marks: readdirSync(asksFolder()) }).toEqual({ asks: false, marks: [] });
  });

  it("removes every pending mark of the folder it serves a start check for, and none of another folder", () => {
    const [folder, other] = [join(home, "app"), join(home, "other")];
    const daemon = daemonWithKeptAnswer(folder);
    writeFileSync(join(home, "stopped-answers", `${DAEMON.pid}-${DAEMON.startedAt}`, `${nameOf(other)}.json`), "{}");
    for (const each of [folder, folder, other]) asked(each);
    const pending = readdirSync(asksFolder()).length;
    daemon.served(folder);
    expect({ pending, folder: marksOf(folder), other: marksOf(other) }).toEqual({ pending: 3, folder: [], other: ["asking"] });
  });

  it("serves a start check where its folder of marks is gone, as where it cannot be listed", () => {
    const folder = join(home, "app");
    const daemon = daemonWithKeptAnswer(folder);
    removeFolderSync(asksFolder());
    expect(() => daemon.served(folder)).not.toThrow();
  });

  it("makes no mark where the daemon served a start check of the folder after the hook asked: the mark would claim a check it already served", () => {
    const folder = join(home, "app");
    const daemon = daemonWithKeptAnswer(folder);
    const { ask } = asked(folder);
    ask.sent();
    daemon.served(folder);
    ask.abandoned();
    expect(marksOf(folder)).toEqual([]);
  });

  it("removes the waiting mark of a folder as it serves a start check there, and tries a removal that failed again at the next start check it serves, of any folder", () => {
    const [folder, other] = [join(home, "app"), join(home, "other")];
    const daemon = daemonWithKeptAnswer(folder);
    // A folder at the mark's name: its removal fails, as one the system refuses does.
    const refused = join(asksFolder(), `${nameOf(folder)}.waiting`);
    mkdirSync(refused);
    daemon.served(folder);
    const stays = existsSync(refused);
    removeFolderSync(refused);
    writeFileSync(refused, "");
    daemon.served(other);
    expect({ stays, after: marksOf(folder) }).toEqual({ stays: true, after: [] });
  });

  it("makes no folder of marks where no daemon made the folder of kept answers, and hooks then make none", () => {
    const folder = join(home, "app");
    const asks = new ServedAsks();
    asks.open(home, DAEMON);
    const { ask, asks: asking } = asked(folder);
    expect({ folders: readdirSync(home), asking, unasked: ask.unasked }).toEqual({ folders: [], asking: true, unasked: false });
  });
});
