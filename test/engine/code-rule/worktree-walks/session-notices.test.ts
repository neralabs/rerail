import { type Dir, writeFileSync } from "node:fs";
import { link, mkdir, mkdtemp, realpath, rename, unlink, writeFile } from "node:fs/promises";
import type * as FileSystem from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { BOUNDARY_DISCOVERY_WAIT_MS, CodeRuleBridge } from "../../../../src/engine/code-rule/code-rule-bridge.js";
import type { CodeRuleAuditor, CodeRuleEditBatch } from "../../../../src/engine/code-rule/code-rule.js";
import { LinkedFiles, SessionNotices, type SessionReads, SharedTreeWatch, WorktreeNotices } from "../../../../src/engine/code-rule/worktree-notices.js";
import { RepositoryWorktreeWalks, WORKTREE_FULL_WALK_MS, WorktreeWalks } from "../../../../src/engine/code-rule/worktree-walks.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";
import { StampTreeWatch } from "./support/stamp-tree-watch.js";

/**
 * Every folder a walk listed, by its absolute path: a whole walk lists each folder of the session folder. `after` holds,
 * by a folder's absolute path, what runs once the next listing of it was read to its end.
 */
const listed = vi.hoisted(() => ({ folders: [] as string[], after: new Map<string, () => Promise<void>>() }));
/**
 * While `on`: each path looked at (lstat), by its absolute path, the most looks in flight at once, and each path whose
 * spelling was looked at (realpath).
 */
const looked = vi.hoisted(() => ({ on: false, paths: [] as string[], inFlight: 0, most: 0, spelled: [] as string[] }));
vi.mock("node:fs/promises", async importOriginal => {
  const actual = await importOriginal<typeof FileSystem>();
  return {
    ...actual,
    opendir: async (...args: Parameters<typeof actual.opendir>) => {
      const folder = String(args[0]);
      listed.folders.push(folder);
      const listing = await actual.opendir(...args);
      const after = listed.after.get(folder);
      if (after === undefined) return listing;
      listed.after.delete(folder);
      // Only the walk's `for await` reads the listing.
      return { async *[Symbol.asyncIterator]() { yield* listing; await after(); } } as unknown as Dir;
    },
    lstat: async (...args: Parameters<typeof actual.lstat>) => {
      if (!looked.on) return await actual.lstat(...args);
      looked.paths.push(String(args[0]));
      looked.most = Math.max(looked.most, (looked.inFlight += 1));
      try {
        return await actual.lstat(...args);
      } finally {
        looked.inFlight -= 1;
      }
    },
    realpath: async (...args: Parameters<typeof actual.realpath>) => {
      if (looked.on) looked.spelled.push(String(args[0]));
      return await actual.realpath(...args);
    },
  };
});

const bridges: CodeRuleBridge[] = [];
const watches: SharedTreeWatch[] = [];
const folders: string[] = [];
afterEach(async () => {
  await Promise.all(bridges.splice(0).map(async bridge => await bridge.close()));
  for (const watch of watches.splice(0)) watch.close();
  await Promise.all(folders.splice(0).map(async folder => await removeFolder(folder)));
  listed.folders.length = 0;
  listed.after.clear();
  Object.assign(looked, { on: false, paths: [], inFlight: 0, most: 0, spelled: [] });
});

/** A watch that tells what the platform's engine watch tells: on Linux the entries of each folder it is given, else the whole tree. */
function platformWatch(): StampTreeWatch {
  return new StampTreeWatch({ byFolder: process.platform === "linux" });
}

/** A session folder of three packages of one Python file each, `m.py`, and `more` files more each (`m0.py` and on). */
async function workspace(more = 0): Promise<string> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-session-notices-")));
  folders.push(root);
  for (const name of ["pkg_0", "pkg_1", "pkg_2"]) {
    await mkdir(join(root, "src", name), { recursive: true });
    await writeFile(join(root, "src", name, "m.py"), `def ${name}():\n    return 1\n`);
    for (let index = 0; index < more; index += 1) await writeFile(join(root, "src", name, `m${index}.py`), `def ${name}_${index}():\n    return 1\n`);
  }
  return root;
}

/** A folder outside the session folder, with a second name in it for each of `files` (absolute paths in `root`). */
async function linkedOutside(root: string, files: readonly string[]): Promise<string> {
  const outside = await realpath(await mkdtemp(join(tmpdir(), "rerail-session-notices-link-")));
  folders.push(outside);
  for (const [index, file] of files.entries()) await link(file, join(outside, `${index}.py`));
  return outside;
}

/**
 * A bridge over `root` whose walk of the other worktrees (none here) takes its notices from `watch`, as the engine
 * service's walks share the engine's, opened, with its first walks done.
 */
async function opened(root: string, watch: StampTreeWatch): Promise<{ bridge: CodeRuleBridge; audits: CodeRuleEditBatch[] }> {
  const audits: CodeRuleEditBatch[] = [];
  const auditor: CodeRuleAuditor = {
    openWorkspace: async () => undefined,
    acceptsPath: () => true,
    worktreeRoots: () => [],
    auditGeneratedEdits: async (_sessionId, batch) => {
      audits.push(batch);
      return { revision: batch.revision, disposition: "continue", violations: [], failures: [] };
    },
    close: async () => undefined,
  };
  const shared = new SharedTreeWatch(watch);
  watches.push(shared);
  const worktreeWalks = new WorktreeWalks({ gapMs: 0, notices: new WorktreeNotices({ treeWatch: shared.user() }), sessionWatch: () => shared.user() });
  const bridge = new CodeRuleBridge(root, auditor, { worktreeWalks });
  bridges.push(bridge);
  await bridge.openWorkspace("session-1");
  // The session's first walk of the other worktrees, after its first passes, hands the folder its watch; the next pass
  // reads the folder whole with the watch up, and on Linux one more, once the watch took up the folders it walked.
  for (let pass = 0; pass < 3; pass += 1) await bridge.checkpoint("turn");
  // The walk of the other worktrees takes the watch's notices too: none runs while a test's command writes.
  await bridge.settleWorktreeWalk();
  listed.folders.length = 0;
  return { bridge, audits };
}

/** Runs a shell command whose effect is `write`, and answers its result. */
async function command(bridge: CodeRuleBridge, id: string, write: () => Promise<void>): Promise<void> {
  await bridge.begin(id, "Bash", { command: "python3 tools/rewrite.py" });
  await write();
  await bridge.finish(id);
}

/** The paths each audit checked, in order. */
function checked(audits: readonly CodeRuleEditBatch[]): string[][] {
  return audits.map(batch => batch.edits.map(edit => edit.path));
}

describe("a shell command's result reads the session folder by its change notices", () => {
  it("watches the session folder through the engine watch the repository's walk shares, and not without one", async () => {
    const root = await workspace();
    const watch = new StampTreeWatch();
    const repositories = new RepositoryWorktreeWalks({ treeWatch: watch });
    try {
      const session = repositories.of("repository").sessionWatch();
      expect(session?.watch(root, [""])).toBe(true);
      expect(watch.folders.get(root)).toEqual([""]);
      session?.close();
      expect(watch.unwatched).toEqual([root]);
    } finally {
      repositories.close();
    }
    expect(new WorktreeWalks().sessionWatch()).toBeUndefined();
  });

  it("reads only the paths a notice named, and checks the write at the result", async () => {
    const root = await workspace();
    const watch = platformWatch();
    const { bridge, audits } = await opened(root, watch);
    // On Linux the watch follows each folder the whole walk walked into; elsewhere it holds the whole tree.
    expect(new Set(watch.folders.get(root))).toEqual(new Set(process.platform === "linux" ? ["", "src", "src/pkg_0", "src/pkg_1", "src/pkg_2"] : [""]));

    await command(bridge, "shell-1", async () => await writeFile(join(root, "src", "pkg_1", "new.py"), "eval(x)\n"));
    expect(listed.folders).toEqual([]);
    expect(audits).toEqual([expect.objectContaining({
      checkpoints: ["edit", "function"],
      edits: [expect.objectContaining({ path: "src/pkg_1/new.py", generatedText: "eval(x)\n" })],
    })]);

    // A removal is checked at the result too, and a command that writes nothing reads nothing.
    await command(bridge, "shell-2", async () => await unlink(join(root, "src", "pkg_0", "m.py")));
    await command(bridge, "shell-3", async () => undefined);
    expect(listed.folders).toEqual([]);
    expect(audits.at(-1)?.edits).toEqual([expect.objectContaining({ path: "src/pkg_0/m.py", deleted: true })]);
    expect(audits).toHaveLength(2);
  });

  it("checks at a command's own result a file it was still writing when another command's result read the folder", async () => {
    const root = await workspace();
    const { bridge, audits } = await opened(root, platformWatch());
    await bridge.begin("shell-long", "Bash", { command: "python3 tools/rewrite.py" });
    await writeFile(join(root, "src", "pkg_1", "m.py"), "eval(x)\n");
    // The long command may be part way through writing m.py: the short one's result leaves it to the walk after.
    await command(bridge, "shell-short", async () => undefined);
    expect(checked(audits).flat()).not.toContain("src/pkg_1/m.py");

    // No notice names m.py again, and its writer's result still checks it, as a whole walk there does.
    await bridge.finish("shell-long");
    expect(checked(audits).at(-1)).toEqual(["src/pkg_1/m.py"]);
  });

  it("reads at a command's own result the file another command's result left to it by name, not the whole folder", async () => {
    const root = await workspace();
    const { bridge, audits } = await opened(root, platformWatch());
    await bridge.begin("shell-long", "Bash", { command: "python3 tools/rewrite.py" });
    await writeFile(join(root, "src", "pkg_1", "m.py"), "eval(x)\n");
    await command(bridge, "shell-short", async () => undefined);
    listed.folders.length = 0;
    await bridge.finish("shell-long");
    expect(checked(audits).at(-1)).toEqual(["src/pkg_1/m.py"]);
    // The short command's result named the file it left to the walk after: no folder is listed to find it again.
    expect(listed.folders).toEqual([]);
  });

  it("reads the paths a notice named many at once, looking at each no more often than the whole walk does", async () => {
    // Large enough that 40 paths stay under half of what the whole walk looks at.
    const root = await workspace(30);
    const watch = platformWatch();
    const { bridge, audits } = await opened(root, watch);
    const files = (name: string): string[] => Array.from({ length: 40 }, (_, index) => join(root, "src", "pkg_1", `${name}${index}.py`));
    const looks = (paths: readonly string[]): number => looked.paths.filter(path => paths.includes(path)).length;
    const noticed = files("noticed");
    looked.on = true;
    await command(bridge, "shell-1", async () => {
      for (const path of noticed) await writeFile(path, "eval(x)\n");
    });
    expect(listed.folders).toEqual([]);
    expect(checked(audits).flat()).toHaveLength(noticed.length);
    // On a network or virtualised mount each look is a round trip: one at a time, a large write's result would wait
    // longer than the whole walk, which looks at files in waves.
    expect(looked.most).toBeGreaterThan(1);
    const noticedLooks = looks(noticed);

    // The same write as no notice tells of, which the turn's end reads by a whole walk.
    const walked = files("walked");
    await command(bridge, "shell-2", async () => {
      for (const path of walked) await writeFile(path, "eval(x)\n");
      watch.silence(root);
    });
    await bridge.checkpoint("turn");
    expect(listed.folders.length).toBeGreaterThan(0);
    expect(checked(audits).flat()).toHaveLength(noticed.length + walked.length);
    // At most: on Windows the whole walk now and then looks at a file once more than here.
    expect(noticedLooks).toBeLessThanOrEqual(looks(walked));
  });

  it("does not check again at the next result what an edit tool wrote and its own result checked", async () => {
    const root = await workspace();
    const { bridge, audits } = await opened(root, platformWatch());
    const path = join(root, "src", "pkg_2", "m.py");
    await bridge.begin("edit-1", "Edit", { file_path: path });
    await writeFile(path, "def pkg_2():\n    return 2\n");
    await bridge.finish("edit-1");
    await command(bridge, "shell-1", async () => undefined);
    expect(checked(audits)).toEqual([["src/pkg_2/m.py"]]);
    expect(listed.folders).toEqual([]);
  });

  it("checks at the turn's end, by a whole walk, a write no notice told of", async () => {
    const root = await workspace();
    const watch = platformWatch();
    const { bridge, audits } = await opened(root, watch);
    // As a memory-mapped write: the watch tells nothing of it.
    await command(bridge, "shell-1", async () => {
      await writeFile(join(root, "src", "pkg_0", "m.py"), "eval(y)\n");
      watch.silence(root);
    });
    expect(audits).toEqual([]);

    await bridge.checkpoint("turn");
    expect(listed.folders.length).toBeGreaterThan(0);
    // Still the session's own write, as its command ran when the file was stamped: checked, not only recorded.
    expect(audits).toEqual([expect.objectContaining({
      checkpoints: ["turn"],
      edits: [expect.objectContaining({ path: "src/pkg_0/m.py", generatedText: "eval(y)\n" })],
    })]);
    expect(audits[0]).not.toHaveProperty("mode");
  });

  it("goes on, when opened again, from the last whole walk, not from a pass of noticed paths", async () => {
    const root = await workspace();
    const watch = platformWatch();
    const { bridge } = await opened(root, watch);
    const since = (await bridge.walkResume())?.since;
    await command(bridge, "shell-1", async () => {
      await writeFile(join(root, "src", "pkg_0", "m.py"), "eval(y)\n");
      watch.silence(root);
    });
    // A session suspended now still judges, once opened again, the write no notice told of.
    expect(since).toBeDefined();
    expect((await bridge.walkResume())?.since).toBe(since);
  });

  it("walks the whole folder at a result after notices were lost or the watch ended, and checks the write there", async () => {
    const root = await workspace();
    const watch = platformWatch();
    const { bridge, audits } = await opened(root, watch);
    await command(bridge, "shell-1", async () => {
      await writeFile(join(root, "src", "pkg_0", "m.py"), "eval(a)\n");
      watch.lose(root);
    });
    expect(listed.folders.length).toBeGreaterThan(0);
    listed.folders.length = 0;
    await command(bridge, "shell-2", async () => {
      await writeFile(join(root, "src", "pkg_1", "m.py"), "eval(b)\n");
      watch.end(root);
    });
    expect(listed.folders.length).toBeGreaterThan(0);
    expect(checked(audits)).toEqual([["src/pkg_0/m.py"], ["src/pkg_1/m.py"]]);
    // The whole walk watched the folder again: a later write is still checked at its result.
    await command(bridge, "shell-3", async () => await writeFile(join(root, "src", "pkg_2", "m.py"), "eval(c)\n"));
    expect(checked(audits).at(-1)).toEqual(["src/pkg_2/m.py"]);
  });

  it("walks the whole folder at every result while the folder cannot be watched", async () => {
    const root = await workspace();
    const watch = platformWatch();
    watch.unwatchable(root);
    const { bridge, audits } = await opened(root, watch);
    await command(bridge, "shell-1", async () => await writeFile(join(root, "src", "pkg_0", "m.py"), "eval(a)\n"));
    expect(listed.folders.length).toBeGreaterThan(0);
    expect(checked(audits)).toEqual([["src/pkg_0/m.py"]]);
  });

  it("walks a folder a command made, and checks the files in it at the result", async () => {
    const root = await workspace();
    const { bridge, audits } = await opened(root, platformWatch());
    await command(bridge, "shell-1", async () => {
      await mkdir(join(root, "src", "pkg_3"));
      await writeFile(join(root, "src", "pkg_3", "m.py"), "eval(d)\n");
    });
    expect(listed.folders).toContain(join(root, "src", "pkg_3"));
    expect(checked(audits)).toEqual([["src/pkg_3/m.py"]]);
  });

  it("checks at their own results the writes into a folder a command deleted and made again", async () => {
    const root = await workspace();
    const { bridge, audits } = await opened(root, platformWatch());
    const made = join(root, "src", "pkg_0");
    // As a code generator that empties its output folder: on Linux the watch lets go of the folder that went.
    await command(bridge, "shell-1", async () => {
      await removeFolder(made);
      await mkdir(made);
      await writeFile(join(made, "m.py"), "eval(g)\n");
    });
    await command(bridge, "shell-2", async () => await writeFile(join(made, "after.py"), "eval(h)\n"));
    expect(checked(audits).at(-1)).toEqual(["src/pkg_0/after.py"]);
    await command(bridge, "shell-3", async () => await writeFile(join(made, "later.py"), "eval(i)\n"));
    expect(checked(audits).at(-1)).toEqual(["src/pkg_0/later.py"]);
  });

  it("walks the whole folder at the next result after a write into a new folder while a walk listed it", async () => {
    const root = await workspace();
    const { bridge, audits } = await opened(root, platformWatch());
    const made = join(root, "src", "pkg_3");
    // Written once the result's whole walk read the new folder, before Linux's watch takes the folder up.
    listed.after.set(made, async () => await writeFile(join(made, "b.py"), "eval(f)\n"));
    await command(bridge, "shell-1", async () => {
      await mkdir(made);
      await writeFile(join(made, "a.py"), "eval(d)\n");
    });
    expect(checked(audits)).toEqual([["src/pkg_3/a.py"]]);
    expect(bridge.observedDigest("src/pkg_3/b.py")).toBeUndefined();
    // Read at the next result, as a whole walk reads it: on Linux by a whole walk, since no notice tells of it.
    await command(bridge, "shell-2", async () => undefined);
    expect(bridge.observedDigest("src/pkg_3/b.py")).toBeDefined();
  });

  it("checks at the result a write through a name outside the session folder", async () => {
    const root = await workspace();
    const outside = await realpath(await mkdtemp(join(tmpdir(), "rerail-session-notices-link-")));
    folders.push(outside);
    await link(join(root, "src", "pkg_0", "m.py"), join(outside, "m.py"));
    const watch = platformWatch();
    const { bridge, audits } = await opened(root, watch);
    await command(bridge, "shell-1", async () => {
      await writeFile(join(outside, "m.py"), "eval(z)\n");
      // No notice of the session folder tells of a write through another folder's name.
      watch.silence(root);
    });
    expect(listed.folders).toEqual([]);
    expect(audits).toEqual([expect.objectContaining({
      checkpoints: ["edit", "function"],
      edits: [expect.objectContaining({ path: "src/pkg_0/m.py", generatedText: "eval(z)\n" })],
    })]);
  });

  it("reads a linked file no notice named by its stamp alone, and only it", async () => {
    const root = await workspace();
    const file = join(root, "src", "pkg_0", "m.py");
    await linkedOutside(root, [file]);
    const { bridge, audits } = await opened(root, platformWatch());
    looked.on = true;
    await command(bridge, "shell-1", async () => undefined);
    expect(listed.folders).toEqual([]);
    // Its spelling is the walk's own, which no look at the folder needs to confirm.
    expect(looked.paths.filter(path => path.startsWith(root))).toEqual([file]);
    expect(looked.spelled).not.toContain(file);
    expect(audits).toEqual([]);
  });

  it("checks at a later result a write through the other name of a file a command linked into the folder", async () => {
    const root = await workspace();
    const outside = await realpath(await mkdtemp(join(tmpdir(), "rerail-session-notices-link-")));
    folders.push(outside);
    await writeFile(join(outside, "linked.py"), "def linked():\n    return 1\n");
    const watch = platformWatch();
    const { bridge, audits } = await opened(root, watch);
    const told = (): string[] => watch.told.flatMap(notice => [...notice.changed, ...notice.renamed]);
    await command(bridge, "shell-1", async () => await link(join(outside, "linked.py"), join(root, "src", "pkg_0", "linked.py")));
    expect(told()).toContain("src/pkg_0/linked.py");
    watch.told.length = 0;
    // Written and silenced in one step, so no take of the watch can come between them and tell of the write.
    await command(bridge, "shell-2", async () => {
      writeFileSync(join(outside, "linked.py"), "eval(z)\n");
      watch.silence(root);
    });
    // No notice named the file: the pass read it as a linked file the last pass found.
    expect(told()).not.toContain("src/pkg_0/linked.py");
    expect(listed.folders).toEqual([]);
    expect(audits.at(-1)?.edits).toEqual([expect.objectContaining({ path: "src/pkg_0/linked.py", generatedText: "eval(z)\n" })]);
  });

  it("reads by notices again at the result after one whose pass of noticed paths fell back to a whole walk", async () => {
    // Twelve linked files of 38 looks: under the bound once, past it if the fallback's walk noted them a second time.
    const root = await workspace(10);
    const files = [...["m", ...Array.from({ length: 10 }, (_, index) => `m${index}`)].map(file => join(root, "src", "pkg_0", `${file}.py`)), join(root, "src", "pkg_1", "m.py")];
    await linkedOutside(root, files);
    const { bridge } = await opened(root, platformWatch());
    // On a volume that ignores case, the old spelling still finds the file: only a whole walk reads it.
    await command(bridge, "shell-1", async () => await rename(join(root, "src", "pkg_2", "m.py"), join(root, "src", "pkg_2", "M.py")));
    listed.folders.length = 0;
    await command(bridge, "shell-2", async () => undefined);
    expect(listed.folders).toEqual([]);
  });

  it("walks the whole folder at a result whose linked files reach half of what the whole walk looked at", async () => {
    // A checkout whose files a copy made by links shares (cp -al): reading each file again would cost more than the walk.
    const root = await workspace(10);
    const files = ["pkg_0", "pkg_1", "pkg_2"].flatMap(name => ["m", ...Array.from({ length: 10 }, (_, index) => `m${index}`)].map(file => join(root, "src", name, `${file}.py`)));
    const outside = await linkedOutside(root, files);
    const watch = platformWatch();
    const { bridge, audits } = await opened(root, watch);
    await command(bridge, "shell-1", async () => {
      await writeFile(join(outside, "0.py"), "eval(z)\n");
      watch.silence(root);
    });
    expect(listed.folders.length).toBeGreaterThan(0);
    // Still checked at its own result: the whole walk finds the write.
    expect(checked(audits)).toEqual([["src/pkg_0/m.py"]]);
  });

  it("records at the result each file of a folder a command moved out of the session folder as gone", async () => {
    const root = await workspace();
    const away = await realpath(await mkdtemp(join(tmpdir(), "rerail-session-notices-away-")));
    folders.push(away);
    const watch = platformWatch();
    const { bridge, audits } = await opened(root, watch);
    // The system tells of the folder alone: nothing watches its files once they are elsewhere.
    await command(bridge, "shell-1", async () => {
      await rename(join(root, "src", "pkg_0"), join(away, "pkg_0"));
      watch.movedAway(root);
    });
    expect(audits).toEqual([expect.objectContaining({ edits: [expect.objectContaining({ path: "src/pkg_0/m.py", deleted: true })] })]);
  });

  it("reports an unlisted file a command wrote past the size limit at a turn's end that answers before its walk", async () => {
    const root = await workspace();
    const { bridge } = await opened(root, platformWatch());
    await command(bridge, "shell-1", async () => await writeFile(join(root, "src", "pkg_0", "dump.raw"), "x".repeat(1_000_001)));
    expect(listed.folders).toEqual([]);
    // The turn's whole walk is held at a folder past the boundary's wait, so the turn answers with what the result found.
    const held = Promise.withResolvers<void>();
    const release = Promise.withResolvers<void>();
    listed.after.set(join(root, "src", "pkg_1"), async () => {
      held.resolve();
      await release.promise;
    });
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      const answer = bridge.checkpoint("turn");
      await held.promise;
      vi.advanceTimersByTime(BOUNDARY_DISCOVERY_WAIT_MS);
      expect((await answer).failures).toContainEqual(expect.objectContaining({
        code: "source-incomplete", message: expect.stringContaining("src/pkg_0/dump.raw: Source exceeds 1000000 byte policy limit"),
      }));
    } finally {
      vi.useRealTimers();
      release.resolve();
    }
    await bridge.checkpoint("turn");
  });

  it("records at the result a file renamed to another case as gone under its old name", async () => {
    const root = await workspace();
    const { bridge, audits } = await opened(root, platformWatch());
    // On a volume that ignores case (Windows, macOS), the old name still finds the file.
    await command(bridge, "shell-1", async () => await rename(join(root, "src", "pkg_0", "m.py"), join(root, "src", "pkg_0", "M.py")));
    expect(audits).toEqual([expect.objectContaining({
      edits: [
        expect.objectContaining({ path: "src/pkg_0/M.py" }),
        expect.objectContaining({ path: "src/pkg_0/m.py", deleted: true }),
      ],
    })]);
  });

  it("asks the engine to watch a folder it refused no more, until the watch it had ends", async () => {
    const root = await workspace();
    const watch = platformWatch();
    watch.unwatchable(root);
    const { bridge } = await opened(root, watch);
    const asked = watch.asked.get(root);
    await command(bridge, "shell-1", async () => await writeFile(join(root, "src", "pkg_0", "m.py"), "eval(a)\n"));
    await bridge.checkpoint("turn");
    expect(asked).toBe(1);
    expect(watch.asked.get(root)).toBe(asked);
  });

  it("notes the files with another name while they stay under half of what the last whole walk looked at", () => {
    const linked = new LinkedFiles(7);
    for (const path of ["a", "b", "c"]) linked.note(path);
    // Three of seven looks: a pass of noticed paths reads each.
    expect(linked.paths).toEqual(["a", "b", "c"]);
    expect(linked.keep(7)).toEqual(new Set(["a", "b", "c"]));
    // A fourth reaches the bound: no more are noted, and none is kept.
    linked.note("d");
    linked.note("e");
    expect(linked.paths).toEqual([]);
    expect(linked.keep(100)).toBeUndefined();
    // Before a whole walk set the bound, every one is noted, and kept only under half of what the walk looked at.
    const first = new LinkedFiles(Number.POSITIVE_INFINITY);
    for (const path of ["a", "b", "c"]) first.note(path);
    expect(first.paths).toEqual(["a", "b", "c"]);
    expect(first.keep(6)).toBeUndefined();
    expect(first.keep(7)).toEqual(new Set(["a", "b", "c"]));
  });

  it("hands each whole walk the bound the last one set, and reads by notices again once a walk finds fewer linked files", async () => {
    const root = await workspace();
    const notices = new SessionNotices(root, { platform: "win32", fullWalkMs: WORKTREE_FULL_WALK_MS });
    notices.use(new StampTreeWatch());
    const seen = { folders: new Set(["", "src", "src/pkg_0"]), leftOut: new Set<string>() };
    // The first whole walk: three linked files of six looks (three files, three folders).
    const first = notices.begin(false);
    for (const path of ["a", "b", "c"]) first.linked.note(path);
    expect(first.linked.paths).toEqual(["a", "b", "c"]);
    notices.ended(seen, { linked: first.linked, files: 3 });
    // At the bound: the next pass reads the whole folder, and its walk notes the linked files only under the bound.
    const second = notices.begin(false);
    expect(second.whole).toBe(true);
    for (const path of ["a", "b", "c"]) second.linked.note(path);
    expect(second.linked.paths).toEqual([]);
    notices.ended(seen, { linked: second.linked, files: 3 });
    // Only the fact was kept: the pass after reads the whole folder again; it finds one, which the next pass reads by its
    // stamp alone.
    const third = notices.begin(false);
    expect(third.whole).toBe(true);
    third.linked.note("a");
    notices.ended(seen, { linked: third.linked, files: 3 });
    expect(notices.begin(false)).toEqual({ whole: false, paths: new Map([["a", "stamp"]]), linked: expect.any(LinkedFiles) });
    notices.close();
  });

  it("walks the whole folder at the first pass ten minutes after the last whole walk began, and reads by notices again after it", async () => {
    const root = await workspace();
    let now = 0;
    const notices = new SessionNotices(root, { platform: "win32", fullWalkMs: WORKTREE_FULL_WALK_MS, now: () => now });
    notices.use(new StampTreeWatch());
    const seen = { folders: new Set(["", "src"]), leftOut: new Set<string>() };
    const pass = (at: number, whole = false): SessionReads => {
      now = at;
      return notices.begin(whole);
    };
    const end = (reads: SessionReads): void => notices.ended(reads.whole ? seen : undefined, { linked: reads.linked, files: 3 });
    end(pass(0));
    const at599 = pass(599_000);
    expect(at599.whole).toBe(false);
    end(at599);
    const at601 = pass(601_000);
    expect(at601.whole).toBe(true);
    // Timed from when the walk began, not from when it ended.
    now = 640_000;
    end(at601);
    const later = pass(641_000);
    expect(later.whole).toBe(false);
    end(later);
    const again = pass(1_201_500);
    expect(again.whole).toBe(true);
    end(again);
    // A turn's end walks whole, and the ten minutes run from it.
    end(pass(1_500_000, true));
    expect(pass(1_802_000).whole).toBe(false);
    notices.close();
  });

  it("checks a write no notice told of at the first result ten minutes after the last whole walk began, and reads by notices after it", async () => {
    const root = await workspace();
    const watch = platformWatch();
    const { bridge, audits } = await opened(root, watch);
    // The clock the period is read by; closed with the bridge, so no walk sleeps on a clock put back.
    bridges.splice(bridges.indexOf(bridge), 1);
    const start = Date.now();
    let at = start;
    const clock = vi.spyOn(Date, "now").mockImplementation(() => at);
    try {
      await bridge.checkpoint("turn");
      at = start + 1_000;
      // As a write through a hard link made to a name outside the folder after that walk: no notice tells of it.
      await command(bridge, "shell-1", async () => {
        writeFileSync(join(root, "src", "pkg_0", "m.py"), "eval(w)\n");
        watch.silence(root);
      });
      listed.folders.length = 0;
      at = start + 599_000;
      await command(bridge, "shell-2", async () => undefined);
      expect(listed.folders).toEqual([]);
      expect(audits).toEqual([]);
      at = start + 601_000;
      await command(bridge, "shell-3", async () => undefined);
      expect(listed.folders.length).toBeGreaterThan(0);
      expect(checked(audits)).toEqual([["src/pkg_0/m.py"]]);
      listed.folders.length = 0;
      at = start + 602_000;
      await command(bridge, "shell-4", async () => undefined);
      expect(listed.folders).toEqual([]);
    } finally {
      await bridge.close();
      clock.mockRestore();
    }
  });

  it("walks the whole folder at a result after a command changed Git's ignore rules", async () => {
    const root = await workspace();
    const { bridge, audits } = await opened(root, platformWatch());
    await command(bridge, "shell-1", async () => {
      await writeFile(join(root, ".gitignore"), "build/\n");
      await writeFile(join(root, "src", "pkg_0", "m.py"), "eval(e)\n");
    });
    expect(listed.folders.length).toBeGreaterThan(0);
    expect(checked(audits)).toEqual([[".gitignore", "src/pkg_0/m.py"]]);
  });
});
