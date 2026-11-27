/**
 * What a walk of the other worktrees asks the disk about files a session saw there: each existsSync, realpath and lstat
 * on the daemon's thread naming one of them or a path below it, each lstat, realpath and access through the thread pool,
 * each opening of one, and the most full lookups of their owners between two turns of the event loop, over one walk a
 * boundary of the session asks for. A session that lasts sees more files; a walk that asks nothing on the daemon's
 * thread of a file it does not read costs a hook no more for it. What the walk records of them stays as it was.
 */
import { execFileSync } from "node:child_process";
import type * as FileSystem from "node:fs";
import type * as FilePromises from "node:fs/promises";
import { mkdir, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { join, relative, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

/**
 * While `on`: each look, by kind and path (`looks`), and each file opened (`opens`); the turn of the event loop, counted
 * by `turn`, of each full owner lookup (`lookups`, by session-relative path); each start and end of an `lstat` of
 * `node:fs/promises`, in order (`lstats`). `asking` is called as each such `lstat` starts.
 */
const disk = vi.hoisted(() => ({
  on: false,
  turn: 0,
  looks: [] as Array<{ readonly kind: string; readonly path: string }>,
  opens: [] as string[],
  lookups: [] as Array<{ readonly path: string; readonly turn: number }>,
  lstats: [] as Array<{ readonly path: string; readonly started: boolean }>,
  asking: undefined as ((path: string) => void) | undefined,
}));
vi.mock("node:fs", async importOriginal => {
  const actual = await importOriginal<typeof FileSystem>();
  const look = (kind: string, path: unknown): void => {
    if (disk.on) disk.looks.push({ kind, path: String(path) });
  };
  const realpathSync = Object.assign(
    (...args: Parameters<typeof actual.realpathSync>) => {
      look("realpath", args[0]);
      return actual.realpathSync(...args);
    },
    {
      native: (...args: Parameters<typeof actual.realpathSync.native>) => {
        look("realpath", args[0]);
        return actual.realpathSync.native(...args);
      },
    },
  );
  return {
    ...actual,
    realpathSync,
    existsSync: (...args: Parameters<typeof actual.existsSync>) => {
      look("existsSync", args[0]);
      return actual.existsSync(...args);
    },
    lstatSync: (...args: Parameters<typeof actual.lstatSync>) => {
      look("lstat", args[0]);
      return actual.lstatSync(...args);
    },
  };
});
vi.mock("node:fs/promises", async importOriginal => {
  const actual = await importOriginal<typeof FilePromises>();
  // Each look the walk can ask through the thread pool, as kind `promised`.
  const look = (path: unknown): void => {
    if (disk.on) disk.looks.push({ kind: "promised", path: String(path) });
  };
  return {
    ...actual,
    open: async (...args: Parameters<typeof actual.open>) => {
      if (disk.on) disk.opens.push(String(args[0]));
      return await actual.open(...args);
    },
    lstat: async (...args: Parameters<typeof actual.lstat>) => {
      look(args[0]);
      const path = String(args[0]);
      if (disk.on) disk.lstats.push({ path, started: true });
      disk.asking?.(path);
      try {
        return await actual.lstat(...args);
      } finally {
        if (disk.on) disk.lstats.push({ path, started: false });
      }
    },
    realpath: async (...args: Parameters<typeof actual.realpath>) => {
      look(args[0]);
      return await actual.realpath(...args);
    },
    access: async (...args: Parameters<typeof actual.access>) => {
      look(args[0]);
      await actual.access(...args);
    },
  };
});

import { CheckoutCodeRuleRouter } from "../../../../src/engine/code-rule/checkout-code-rule-router.js";
import { CodeRuleBridge } from "../../../../src/engine/code-rule/code-rule-bridge.js";
import { codeRuleSchema } from "../../../../src/engine/code-rule/code-rule-schema.js";
import { ToolCallWindows } from "../../../../src/engine/code-rule/tool-call-windows.js";
import { RerailRuntime } from "../../../../src/runtime/index.js";
import { temporaryDirectories, testEmbedder } from "../../../support/hook-events.js";
import { setUpTestRepository } from "../../../support/repository-rules.js";

const temporaryDirectory = temporaryDirectories("rerail-seen-sources-");

afterEach(() => {
  disk.on = false;
  disk.asking = undefined;
  vi.restoreAllMocks();
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

const SESSION = "session-1";

const printRule = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:print", revision: 1, mode: "steer", scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{ id: "print", violation: { code: "P", anchor: "witness.span", message: "No print" }, forbid: { ast: { language: "python", rule: { pattern: "print($ARG)" } } } }],
});

interface Session {
  readonly runtime: RerailRuntime;
  readonly bridge: CodeRuleBridge;
  /** The session's folder, a started repository's main checkout; physical. */
  readonly main: string;
  /** The repository's other worktree; physical. */
  readonly w1: string;
}

/** A session open in a started repository's main checkout, after its first turn, with one other worktree, `w1`. */
async function sessionBesideWorktree(): Promise<Session> {
  const root = await realpath(await temporaryDirectory({ started: false }));
  const databasePath = join(root, "rerail.sqlite");
  const main = join(root, "main");
  await mkdir(join(main, "src"), { recursive: true });
  await writeFile(join(main, "src", "x.py"), "x = 1\n");
  git(main, "init", "--quiet");
  git(main, "add", ".");
  git(main, "commit", "--quiet", "-m", "seed");
  setUpTestRepository(main, { library: [{ kind: "code", rule: printRule }] }, databasePath);
  const w1 = join(root, "w1");
  git(main, "worktree", "add", "--quiet", "--detach", w1);
  const opens = vi.spyOn(CodeRuleBridge.prototype, "openWorkspace");
  const runtime = new RerailRuntime({
    storageRoot: join(root, "sessions"), databasePath, workspaceRoot: main, embedder: testEmbedder, autostart: false, startGate: true,
  });
  await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId: SESSION, cwd: main } });
  await runtime.codeRulesOpened(SESSION);
  const bridge = opens.mock.contexts.find((context): context is CodeRuleBridge => context instanceof CodeRuleBridge);
  if (bridge === undefined) throw new Error("the session opened no bridge");
  const session = { runtime, bridge, main, w1 };
  await turn(session);
  return session;
}

let events = 0;

/** A turn's end, and the walk of the other worktrees it asks for, both done. */
async function turn({ runtime, bridge }: Session): Promise<void> {
  await runtime.ingest({ kind: "turn-end", eventId: `turn-${++events}`, sessionId: SESSION, text: "" });
  await bridge.settleWorktreeWalk();
}

/** Writes `text` to `path` in a Write tool call of the session. */
async function write({ runtime }: Session, path: string, text: string): Promise<void> {
  const toolCallId = `write-${++events}`;
  await runtime.ingest({ kind: "call", eventId: `call-${toolCallId}`, sessionId: SESSION, toolCallId, toolName: "Write", input: { file_path: path, content: text } });
  await mkdir(join(path, ".."), { recursive: true });
  await writeFile(path, text);
  await runtime.ingest({ kind: "result", eventId: `result-${toolCallId}`, sessionId: SESSION, toolCallId, toolName: "Write", output: "ok" });
}

/** Starts a shell call of the session whose result has not come: its command runs through every walk until it does. */
async function commandStarts({ runtime }: Session, toolCallId: string): Promise<void> {
  await runtime.ingest({ kind: "call", eventId: `call-${toolCallId}`, sessionId: SESSION, toolCallId, toolName: "Bash", input: { command: "./build.sh" } });
}

async function commandEnds({ runtime }: Session, toolCallId: string): Promise<void> {
  await runtime.ingest({ kind: "result", eventId: `result-${toolCallId}`, sessionId: SESSION, toolCallId, toolName: "Bash", output: "" });
}

/** A shell call of the session from its call to its result, and the walk of the other worktrees its result asks for. */
async function shellCall(session: Session): Promise<void> {
  const toolCallId = `shell-${++events}`;
  await commandStarts(session, toolCallId);
  await commandEnds(session, toolCallId);
  await session.bridge.settleWorktreeWalk();
}

/** Whether `path` is one of `paths` or lies below one, letter case aside on Windows. */
function about(paths: readonly string[], path: string): boolean {
  const spelt = (text: string): string => (process.platform === "win32" ? text.toLowerCase().split("\\").join("/") : text);
  return paths.some(named => spelt(path) === spelt(named) || spelt(path).startsWith(`${spelt(named)}/`));
}

/** What a walk asked the disk about some files: each kind of look, the openings, and the most full owner lookups in one turn. */
interface Asked {
  readonly existsSync: number;
  readonly realpath: number;
  readonly lstat: number;
  /** Each `lstat`, `realpath` and `access` of `node:fs/promises`. */
  readonly promised: number;
  readonly opens: number;
  readonly run: number;
}

/** What `walk` asked the disk about `paths` (absolute) and the paths below them, in the session `session`. */
async function askedAbout(session: Session, paths: readonly string[], walk: () => Promise<void>): Promise<Asked> {
  disk.looks.length = 0;
  disk.opens.length = 0;
  disk.lookups.length = 0;
  disk.lstats.length = 0;
  const named = paths.map(path => relative(session.main, path).split(sep).join("/"));
  // A full lookup of a path's owner: a walk's own view (`walkView`) looks each folder up once and is not counted here.
  const accepts = CheckoutCodeRuleRouter.prototype.acceptsPath;
  const lookups = vi.spyOn(CheckoutCodeRuleRouter.prototype, "acceptsPath").mockImplementation(function (this: CheckoutCodeRuleRouter, path: string) {
    if (about(named, path)) disk.lookups.push({ path, turn: disk.turn });
    return accepts.call(this, path);
  });
  disk.on = true;
  const ticking = (async () => {
    for (; disk.on; disk.turn += 1) await new Promise(resolve => setImmediate(resolve));
  })();
  try {
    await walk();
  } finally {
    disk.on = false;
    await ticking;
    lookups.mockRestore();
  }
  const looks = (kind: string): number => disk.looks.filter(look => look.kind === kind && about(paths, look.path)).length;
  const perTurn = new Map<number, number>();
  for (const { turn: at } of disk.lookups) perTurn.set(at, (perTurn.get(at) ?? 0) + 1);
  return {
    existsSync: looks("existsSync"), realpath: looks("realpath"), lstat: looks("lstat"), promised: looks("promised"),
    opens: disk.opens.filter(path => about(paths, path)).length, run: Math.max(0, ...perTurn.values()),
  };
}

const NOTHING: Asked = { existsSync: 0, realpath: 0, lstat: 0, promised: 0, opens: 0, run: 0 };

/** Of the `lstat` calls of `node:fs/promises` the last `askedAbout` saw naming one of `paths`: how many, and the most out at once. */
function lstatsOf(paths: readonly string[]): { readonly asked: number; readonly atOnce: number } {
  let asked = 0;
  let out = 0;
  let atOnce = 0;
  for (const { path, started } of disk.lstats.filter(look => about(paths, look.path))) {
    asked += started ? 1 : 0;
    out += started ? 1 : -1;
    atOnce = Math.max(atOnce, out);
  }
  return { asked, atOnce };
}

/** `path` (absolute) as the session names it. */
function named(session: Session, path: string): string {
  return relative(session.main, path).split(sep).join("/");
}

/** Records from now each change an audit of the session is asked to record, as `deleted <path>` or `written <path>`. */
function recordedChanges(): () => string[] {
  const audits = vi.spyOn(CheckoutCodeRuleRouter.prototype, "auditGeneratedEdits");
  return () => audits.mock.calls.flatMap(([, batch]) => batch.edits.map(edit => `${edit.deleted === true ? "deleted" : "written"} ${edit.path}`));
}

describe("a walk of the other worktrees, about the files a session saw there", () => {
  it("asks one lstat through the thread pool of each source the session wrote in a folder no walk enters, there or gone, and nothing on the daemon's thread", { timeout: 120_000 }, async () => {
    const session = await sessionBesideWorktree();
    try {
      const built = [0, 1, 2, 3].map(index => join(session.w1, "build", `b${index}.py`));
      for (const [index, path] of built.entries()) await write(session, path, `b = ${index}\n`);
      await turn(session);

      const there = await askedAbout(session, built, async () => await turn(session));
      const held = built.map(path => session.bridge.observedDigest(named(session, path)));
      for (const path of built) await rm(path);
      const gone = await askedAbout(session, built, async () => await turn(session));
      const after = await askedAbout(session, built, async () => await turn(session));

      // `build` is a folder no walk enters, so no snapshot holds these files. Each walk looked up the owner of each one
      // twice and read it, for as long as the session lasted: { existsSync: 8, realpath: 8, lstat: 8, promised: 8,
      // opens: 4, run: 1 }. The walk that found them gone looked up each owner and opened each once more, then routed
      // each deletion it records to its checkout: { existsSync: 8, lstat: 8, opens: 4, run: 1 }. That routing, an
      // existsSync and an lstat naming each, is all that walk still asks on the daemon's thread.
      expect({ there, gone, after }).toEqual({
        there: { ...NOTHING, promised: 4 },
        gone: { ...NOTHING, existsSync: 4, lstat: 4, promised: 4 },
        after: NOTHING,
      });
      // While they were there, the session held what it saw of each.
      expect(held).toEqual(built.map(() => expect.any(String)));
    } finally {
      await session.runtime.dispose();
    }
  });

  /** Twenty sources the session wrote in `w1/build`, a folder no walk enters, and the walk after their writes done. */
  async function twentyBuilt(session: Session): Promise<string[]> {
    const built = Array.from({ length: 20 }, (_, index) => join(session.w1, "build", `b${index}.py`));
    for (const [index, path] of built.entries()) await write(session, path, `b = ${index}\n`);
    await turn(session);
    return built;
  }

  it("asks the lstats of the sources the session wrote in a folder no walk enters sixteen at a time", { timeout: 120_000 }, async () => {
    const session = await sessionBesideWorktree();
    try {
      const built = await twentyBuilt(session);

      await askedAbout(session, built, async () => await turn(session));

      // As the walk's reads ask theirs (`OBSERVATION_CONCURRENCY`): the walk waits for two round trips of the thread
      // pool, not one after another for each of the twenty.
      expect(lstatsOf(built)).toEqual({ asked: 20, atOnce: 16 });
    } finally {
      await session.runtime.dispose();
    }
  });

  it("asks no lstat of such a source after the wave out when the session closes during the walk", { timeout: 120_000 }, async () => {
    const session = await sessionBesideWorktree();
    try {
      const built = await twentyBuilt(session);
      const closing: Array<Promise<void>> = [];
      disk.asking = path => {
        if (closing.length === 0 && about(built, path)) closing.push(session.bridge.close());
      };

      await askedAbout(session, built, async () => await turn(session));
      await Promise.all(closing);

      // The close abandons the walk as the first lstat of its first wave starts: the rest of that wave, then none.
      expect(lstatsOf(built).asked).toBe(16);
    } finally {
      await session.runtime.dispose();
    }
  });

  it("records the deletion of a source the session wrote in a folder no walk enters, and answers for the session's calls there", { timeout: 120_000 }, async () => {
    const session = await sessionBesideWorktree();
    try {
      const built = join(session.w1, "build", "gen.py");
      await write(session, built, "g = 1\n");
      await turn(session);
      await rm(built);
      const recorded = recordedChanges();
      const answered = vi.spyOn(ToolCallWindows.prototype, "answeredIn");

      await turn(session);

      // The walk records the deletion, as of any source the session saw that is gone, and forgets what it saw of it.
      expect(recorded()).toEqual([`deleted ${named(session, built)}`]);
      expect(session.bridge.observedDigest(named(session, built))).toBeUndefined();
      // And it answers for the session's calls in `w1`, the Write among them.
      expect(answered.mock.calls.map(([place]) => place)).toContain(named(session, session.w1));
    } finally {
      await session.runtime.dispose();
    }
  });

  // A link to a file needs a right Windows grants only some accounts.
  it.skipIf(process.platform === "win32")("records as deleted a source the session wrote in a folder no walk enters once a link takes its place, as a read finds", { timeout: 120_000 }, async () => {
    const session = await sessionBesideWorktree();
    try {
      const built = join(session.w1, "build", "gen.py");
      await write(session, built, "g = 1\n");
      await turn(session);
      await writeFile(join(session.w1, "build", "other.py"), "o = 1\n");
      await rm(built);
      await symlink("other.py", built);
      const recorded = recordedChanges();

      await turn(session);

      // An `lstat` finds a link, which a read does not follow: the read decides, and finds no source there.
      expect(recorded()).toEqual([`deleted ${named(session, built)}`]);
    } finally {
      await session.runtime.dispose();
    }
  });

  /**
   * Four files no rule reads in `w1`, removed while a shell command of the session runs, after the walk at a call's
   * result found them gone and left them to the next walk, as the command may be writing them: the session keeps what
   * it saw of each meanwhile.
   */
  async function goneWhileCommandRuns(session: Session): Promise<string[]> {
    const notes = [0, 1, 2, 3].map(index => join(session.w1, "notes", `n${index}.txt`));
    await mkdir(join(session.w1, "notes"));
    for (const [index, path] of notes.entries()) await writeFile(path, `${index}\n`);
    await turn(session);
    await commandStarts(session, "long-command");
    for (const path of notes) await rm(path);
    await shellCall(session);
    return notes;
  }

  it("asks the disk nothing of the files gone there, nor of a source in a folder no walk enters, while a shell command of the session runs through its walks", { timeout: 120_000 }, async () => {
    const session = await sessionBesideWorktree();
    try {
      const built = join(session.w1, "build", "b.py");
      await write(session, built, "b = 1\n");
      const notes = await goneWhileCommandRuns(session);

      const during = await askedAbout(session, [...notes, built], async () => await shellCall(session));

      // Each walk the command ran through looked up the owner of each note (an existsSync and an lstat naming it, an
      // existsSync and a realpath of its folder), all in one turn of the event loop, { existsSync: 4, lstat: 4, run: 4 },
      // and read `b.py` as the first case says, though it leaves each to the next walk whatever the disk says.
      expect(during).toEqual(NOTHING);
    } finally {
      await session.runtime.dispose();
    }
  });

  it("forgets a file no rule reads once a walk no command runs through finds it gone, and asks after it no more", { timeout: 120_000 }, async () => {
    const session = await sessionBesideWorktree();
    try {
      const notes = await goneWhileCommandRuns(session);
      await commandEnds(session, "long-command");

      const ended = await askedAbout(session, notes, async () => await turn(session));
      const after = await askedAbout(session, notes, async () => await turn(session));

      expect(ended.run).toBeGreaterThan(0);
      // Gone, with no digest (no rule reads it), it was no deletion to record, and was never forgotten either: every
      // later walk asked after each one, for the session's life: { existsSync: 4, lstat: 4, run: 4 } at each.
      expect(after).toEqual(NOTHING);
    } finally {
      await session.runtime.dispose();
    }
  });

  it("asks the disk only the read's own lstat of each source the session saw that it reads unchanged, and one more of each folder it reads, its `.gitignore`", { timeout: 120_000 }, async () => {
    const session = await sessionBesideWorktree();
    try {
      const sources = [0, 1, 2, 3].map(index => join(session.w1, "src", `s${index}.py`));
      const folders = [0, 1, 2, 3].map(index => join(session.w1, "pkg", `d${index}`));
      for (const [index, path] of sources.entries()) await write(session, path, `s = ${index}\n`);
      for (const folder of folders) {
        await mkdir(folder, { recursive: true });
        await writeFile(join(folder, "f.py"), "f = 1\n");
      }
      await turn(session);

      const seen = await askedAbout(session, sources, async () => await turn(session));
      const walked = await askedAbout(session, folders, async () => await turn(session));

      expect({ seen, walked }).toEqual({
        // The read's own lstat of each entry it meets (`lookAt`), on the thread pool: none more for the session's sight.
        seen: { ...NOTHING, promised: 4 },
        // That lstat of each folder and of its file, and on the daemon's thread, of each folder's `.gitignore`, to find
        // Git's ignore rules there (`GitIgnoreAnswers`). None of its `.git` entry: its listing named none, so its
        // checkout is its parent's (`WalkFolderOwners.listed`).
        walked: { ...NOTHING, lstat: 4, promised: 8 },
      });
    } finally {
      await session.runtime.dispose();
    }
  });
});
