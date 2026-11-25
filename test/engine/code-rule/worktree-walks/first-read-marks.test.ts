/**
 * A file written into another worktree before the walk of the other worktrees first reads it. That read is the
 * worktree's baseline, so no rule checks the file and nothing records it. The session's record names the worktree among
 * the places its walks could not check when that read finds a file changed since the session opened that Git did not
 * write and no call of the session that ran there wrote. A later read, and Git's own files, mark nothing.
 */
import { execFileSync } from "node:child_process";
import { lstat, mkdir, mkdtemp, realpath, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative, resolve, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CodeRuleBridge, type DiscoveryGap } from "../../../../src/engine/code-rule/code-rule-bridge.js";
import type { CodeRuleAuditor, CodeRuleEditBatch } from "../../../../src/engine/code-rule/code-rule.js";
import { WorktreeWalks } from "../../../../src/engine/code-rule/worktree-walks.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

/** Run before an `lstat` of the path, the first one first: a case holds a walk at a worktree's folder with it. */
const lstats = vi.hoisted(() => new Map<string, Array<() => Promise<void>>>());
vi.mock("node:fs/promises", async importOriginal => {
  const actual = await importOriginal<typeof import("node:fs/promises")>();
  const held = (async (...args: Parameters<typeof actual.lstat>) => {
    await lstats.get(String(args[0]))?.shift()?.();
    return await actual.lstat(...args);
  }) as typeof actual.lstat;
  return { ...actual, lstat: held };
});

/** The reason the record gives, as a person reads it beside the worktree's path. */
const NOT_CHECKED = "Files changed here before Rerail first read this worktree were not checked; `rerail scan --repo <this worktree> --scope git-diff` checks them";

const bridges: CodeRuleBridge[] = [];
const folders: string[] = [];
afterEach(async () => {
  lstats.clear();
  await Promise.all(bridges.splice(0).map(async bridge => await bridge.close()));
  await Promise.all(folders.splice(0).map(async path => await removeFolder(path)));
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

async function folder(): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), "rerail-first-read-"));
  folders.push(path);
  await mkdir(join(path, "src"));
  return await realpath(path);
}

/** Waits until the file system's clock has passed every stamp under `paths`: a later write is stamped after them. */
async function clockAfter(...paths: string[]): Promise<void> {
  const since = Math.max(...await Promise.all(paths.map(async path => { const info = await lstat(path); return Math.max(info.mtimeMs, info.ctimeMs); })));
  const probe = await folder();
  for (;;) {
    await writeFile(join(probe, "tick"), "");
    const info = await lstat(join(probe, "tick"));
    if (Math.max(info.mtimeMs, info.ctimeMs) > since) return;
  }
}

/** A repository on `main` with a branch `other`, which adds `src/new.py`, a file a rule would steer on. */
async function repository(): Promise<string> {
  const root = await folder();
  await writeFile(join(root, "src", "a.py"), "old\n");
  git(root, "init", "--quiet", "--initial-branch=main");
  git(root, "config", "core.autocrlf", "false");
  git(root, "add", ".");
  git(root, "commit", "--quiet", "-m", "seed");
  git(root, "switch", "--quiet", "-c", "other");
  await writeFile(join(root, "src", "new.py"), "violation\n");
  git(root, "add", ".");
  git(root, "commit", "--quiet", "-m", "other");
  git(root, "switch", "--quiet", "main");
  await clockAfter(join(root, "src", "a.py"));
  return root;
}

/** What the auditor got, and the places the session's record was told its walks could not check. */
interface Seen {
  readonly audited: Array<{ session: string; path: string; owner: string }>;
  readonly told: Map<string, DiscoveryGap[]>;
}

/**
 * Sessions on `root` that watch the worktrees `roots` lists, through one shared walk, each with what it was told. A
 * worktree is listed once `roots` names it: `add` adds one at `place` and lists it, `remove` removes it and unlists it.
 */
async function sessions(root: string) {
  const place = join(await folder(), "w");
  const roots: string[] = [];
  const seen: Seen = { audited: [], told: new Map() };
  const walks = new WorktreeWalks({ gapMs: 0 });
  const extra: Partial<CodeRuleAuditor> = {
    sourceRoot: path => roots.find(known => resolve(root, path).startsWith(`${known}${sep}`)) ?? root,
    worktreeRoots: () => roots,
  };
  const open = async (sessionId: string, resume?: Awaited<ReturnType<CodeRuleBridge["walkResume"]>>) => {
    const auditGeneratedEdits = async (session: string, batch: CodeRuleEditBatch) => {
      for (const edit of batch.edits) seen.audited.push({ session, path: edit.path, owner: batch.owner ?? "session" });
      return { revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] };
    };
    const auditor: CodeRuleAuditor = { openWorkspace: vi.fn(async () => undefined), acceptsPath: () => true, auditGeneratedEdits, close: vi.fn(async () => undefined), ...extra };
    const bridge = new CodeRuleBridge(root, auditor, {
      worktreeWalkGapMs: 0, worktreeWalks: walks,
      onDiscoveryGaps: gaps => { seen.told.set(sessionId, [...seen.told.get(sessionId) ?? [], ...gaps]); },
      ...(resume === undefined ? {} : { resume }),
    });
    bridges.push(bridge);
    await bridge.openWorkspace(sessionId);
    return bridge;
  };
  const add = async (): Promise<string> => {
    git(root, "worktree", "add", "--quiet", place, "other");
    const worktree = await realpath(place);
    roots.push(worktree);
    return worktree;
  };
  const remove = (): void => {
    git(root, "worktree", "remove", "--force", place);
    roots.length = 0;
  };
  return { open, add, remove, seen, named: relative(root, place).split(sep).join("/") };
}

/** Each session's turn end, then the shared walk settled, twice: a worktree's first read, then the read after it. */
async function settle(...bridges: CodeRuleBridge[]): Promise<void> {
  for (let pass = 0; pass < 2; pass += 1) {
    for (const bridge of bridges) {
      await bridge.checkpoint("turn");
      await bridge.settleWorktreeWalk();
    }
  }
}

/**
 * Settles `bridges` with `act` run as the walk's first read of `worktree` reaches its `src` folder, before it lists it:
 * what `act` writes there is in that read, and in no read before it.
 */
async function atFirstRead(worktree: string, act: () => Promise<void>, ...bridges: CodeRuleBridge[]): Promise<void> {
  let acted = false;
  lstats.set(join(worktree, "src"), [async () => { await act(); acted = true; }]);
  await settle(...bridges);
  expect(acted).toBe(true);
}

/** A shell call of `bridge` that ran in `cwd` and did `run`. */
async function command(bridge: CodeRuleBridge, run: () => Promise<void>, cwd: string, id = "shell-1"): Promise<void> {
  await bridge.begin(id, "shell", { command: "./run.sh" }, undefined, cwd);
  await run();
  await bridge.finish(id);
}

const places = (seen: Seen, session: string) => (seen.told.get(session) ?? []).map(({ path, reason }) => ({ path, reason }));

describe("a worktree's first read", () => {
  it.each([{ order: "begun before the command", before: true }, { order: "begun after it", before: false }])(
    "marks the session's record with the worktree once when the session's command, run in its own folder, wrote there before that read, in a walk $order",
    async ({ before }) => {
      const root = await repository();
      const { open, add, seen, named } = await sessions(root);
      const worktree = await add();
      const bridge = await open("session-1");
      const write = async () => {
        await bridge.begin("shell-1", "shell", { command: "./run.sh" }, undefined, root);
        await writeFile(join(worktree, "src", "steer_bad.py"), "violation\n");
      };
      if (before) await atFirstRead(worktree, write, bridge);
      else await write();
      await bridge.finish("shell-1");
      await settle(bridge);
      const { failures } = await bridge.checkpoint("turn");
      expect(seen.audited).toEqual([]);
      expect(places(seen, "session-1")).toEqual([{ path: named, reason: NOT_CHECKED }]);
      expect(failures.map(failure => failure.message)).toEqual([`${named}: ${NOT_CHECKED}`]);
    },
  );

  // The read cannot tell an editor's file from one a session's command wrote, so every session it is the first read of
  // is marked, the one with no call running too.
  it("marks the record of each session whose first read finds a file another writer put there", async () => {
    const root = await repository();
    const { open, add, seen, named } = await sessions(root);
    const worktree = await add();
    const [first, second] = [await open("session-1"), await open("session-2")];
    await atFirstRead(worktree, async () => { await writeFile(join(worktree, "src", "editor.py"), "violation\n"); }, first, second);
    await settle(first, second);
    const marked = [{ path: named, reason: NOT_CHECKED }];
    expect({ audited: seen.audited, first: places(seen, "session-1"), second: places(seen, "session-2") })
      .toEqual({ audited: [], first: marked, second: marked });
  });

  it("marks nothing and records nothing of the files Git writes as the session's command adds the worktree", async () => {
    const root = await repository();
    const { open, add, seen } = await sessions(root);
    const bridge = await open("session-1");
    await settle(bridge);
    await command(bridge, async () => { await add(); }, root);
    await settle(bridge);
    expect({ audited: seen.audited, told: places(seen, "session-1") }).toEqual({ audited: [], told: [] });
  });

  it("takes as the session's own, unmarked, a file its command wrote there from a folder in that worktree", async () => {
    const root = await repository();
    const { open, add, seen } = await sessions(root);
    const worktree = await add();
    const bridge = await open("session-1");
    const file = join(worktree, "src", "steer_bad.py");
    await command(bridge, async () => { await writeFile(file, "violation\n"); }, worktree);
    await settle(bridge);
    expect({ audited: seen.audited, told: places(seen, "session-1") })
      .toEqual({ audited: [{ session: "session-1", path: relative(root, file).split(sep).join("/"), owner: "session" }], told: [] });
  });
});

describe("a later read of a worktree", () => {
  it("records once for the repository, unmarked, what the session's command wrote there from its own folder after the first read", async () => {
    const root = await repository();
    const { open, add, seen } = await sessions(root);
    const worktree = await add();
    const bridge = await open("session-1");
    await settle(bridge);
    const file = join(worktree, "src", "steer_bad.py");
    await command(bridge, async () => { await writeFile(file, "violation\n"); }, root);
    await settle(bridge);
    expect({ audited: seen.audited, told: places(seen, "session-1") })
      .toEqual({ audited: [{ session: "session-1", path: relative(root, file).split(sep).join("/"), owner: "repository" }], told: [] });
  });

  it("marks neither of two sessions that read the worktree before a write made there while the second's call ran in its own folder", async () => {
    const root = await repository();
    const { open, add, seen } = await sessions(root);
    const worktree = await add();
    const [first, second] = [await open("session-1"), await open("session-2")];
    await settle(first, second);
    await command(second, async () => { await writeFile(join(worktree, "src", "editor.py"), "violation\n"); }, root, "long-2");
    await settle(first, second);
    expect({ first: places(seen, "session-1"), second: places(seen, "session-2"), owners: seen.audited.map(edit => edit.owner) })
      .toEqual({ first: [], second: [], owners: ["repository"] });
  });

  it("after a suspension marks nothing of what the session's command wrote there before it", async () => {
    const root = await repository();
    const { open, add, seen } = await sessions(root);
    const worktree = await add();
    const bridge = await open("session-1");
    await settle(bridge);
    await command(bridge, async () => { await writeFile(join(worktree, "src", "steer_bad.py"), "violation\n"); }, root);
    await settle(bridge);
    const resume = await bridge.walkResume();
    await bridge.close();
    const reopened = await open("session-1", resume);
    await command(reopened, async () => undefined, root, "shell-2");
    await settle(reopened);
    // Recorded for the repository before the suspension; the reopened session's first read of the worktree finds it again.
    expect({ recorded: seen.audited[0]?.owner, told: places(seen, "session-1") }).toEqual({ recorded: "repository", told: [] });
  });
});

describe("a marked worktree Git stops listing", () => {
  const editor = (worktree: string) => async () => { await writeFile(join(worktree, "src", "editor.py"), "violation\n"); };

  it("leaves every later checkpoint, and the session's record keeps the place it holds", async () => {
    const root = await repository();
    const { open, add, remove, seen, named } = await sessions(root);
    const worktree = await add();
    const bridge = await open("session-1");
    await atFirstRead(worktree, editor(worktree), bridge);
    const marked = (await bridge.checkpoint("turn")).failures.map(failure => failure.message);
    remove();
    await settle(bridge);
    const { failures } = await bridge.checkpoint("turn");
    expect({ marked, later: failures.map(failure => failure.message), record: places(seen, "session-1") })
      .toEqual({ marked: [`${named}: ${NOT_CHECKED}`], later: [], record: [{ path: named, reason: NOT_CHECKED }] });
  });

  it("is marked again when Git adds it again at its path and its new first read finds a file no call wrote, with the record's one place", async () => {
    const root = await repository();
    const { open, add, remove, seen, named } = await sessions(root);
    const worktree = await add();
    const bridge = await open("session-1");
    await atFirstRead(worktree, editor(worktree), bridge);
    remove();
    await settle(bridge);
    await add();
    await atFirstRead(worktree, editor(worktree), bridge);
    const { failures } = await bridge.checkpoint("turn");
    expect({ said: failures.map(failure => failure.message), record: places(seen, "session-1") })
      .toEqual({ said: [`${named}: ${NOT_CHECKED}`], record: [{ path: named, reason: NOT_CHECKED }] });
  });
});
