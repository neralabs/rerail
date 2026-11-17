import { mkdir, mkdtemp, realpath, unlink, writeFile } from "node:fs/promises";
import type * as FileSystem from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CodeRuleBridge } from "../../../../src/engine/code-rule/code-rule-bridge.js";
import type { CodeRuleAuditor, CodeRuleEditBatch } from "../../../../src/engine/code-rule/code-rule.js";
import { SharedTreeWatch, WorktreeNotices } from "../../../../src/engine/code-rule/worktree-notices.js";
import { RepositoryWorktreeWalks, WorktreeWalks } from "../../../../src/engine/code-rule/worktree-walks.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";
import { StampTreeWatch } from "./support/stamp-tree-watch.js";

/** Every folder a walk listed, by its absolute path: a whole walk lists each folder of the session folder. */
const listed = vi.hoisted(() => ({ folders: [] as string[] }));
vi.mock("node:fs/promises", async importOriginal => {
  const actual = await importOriginal<typeof FileSystem>();
  return {
    ...actual,
    opendir: async (...args: Parameters<typeof actual.opendir>) => {
      listed.folders.push(String(args[0]));
      return await actual.opendir(...args);
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
});

/** A session folder of three packages of one Python file each. */
async function workspace(): Promise<string> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-session-notices-")));
  folders.push(root);
  for (const name of ["pkg_0", "pkg_1", "pkg_2"]) {
    await mkdir(join(root, "src", name), { recursive: true });
    await writeFile(join(root, "src", name, "m.py"), `def ${name}():\n    return 1\n`);
  }
  return root;
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
  const worktreeWalks = new WorktreeWalks({ gapMs: 0, notices: new WorktreeNotices({ treeWatch: shared.user() }) });
  const bridge = new CodeRuleBridge(root, auditor, { worktreeWalks });
  bridges.push(bridge);
  await bridge.openWorkspace("session-1");
  // The session's first walk of the other worktrees, after its first passes, hands the folder its watch; the next pass
  // reads the folder whole with the watch up, and on Linux one more, once the watch took up the folders it walked.
  for (let pass = 0; pass < 3; pass += 1) await bridge.checkpoint("turn");
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
    const watch = new StampTreeWatch();
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

  it("does not check again at the next result what an edit tool wrote and its own result checked", async () => {
    const root = await workspace();
    const { bridge, audits } = await opened(root, new StampTreeWatch());
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
    const watch = new StampTreeWatch();
    const { bridge, audits } = await opened(root, watch);
    // As a write through a hard link or a memory map: the watch tells nothing of it.
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

  it("walks the whole folder at a result after notices were lost or the watch ended, and checks the write there", async () => {
    const root = await workspace();
    const watch = new StampTreeWatch();
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
    const watch = new StampTreeWatch();
    watch.unwatchable(root);
    const { bridge, audits } = await opened(root, watch);
    await command(bridge, "shell-1", async () => await writeFile(join(root, "src", "pkg_0", "m.py"), "eval(a)\n"));
    expect(listed.folders.length).toBeGreaterThan(0);
    expect(checked(audits)).toEqual([["src/pkg_0/m.py"]]);
  });

  it("walks a folder a command made, and checks the files in it at the result", async () => {
    const root = await workspace();
    const { bridge, audits } = await opened(root, new StampTreeWatch());
    await command(bridge, "shell-1", async () => {
      await mkdir(join(root, "src", "pkg_3"));
      await writeFile(join(root, "src", "pkg_3", "m.py"), "eval(d)\n");
    });
    expect(listed.folders).toContain(join(root, "src", "pkg_3"));
    expect(checked(audits)).toEqual([["src/pkg_3/m.py"]]);
  });

  it("walks the whole folder at a result after a command changed Git's ignore rules", async () => {
    const root = await workspace();
    const { bridge, audits } = await opened(root, new StampTreeWatch());
    await command(bridge, "shell-1", async () => {
      await writeFile(join(root, ".gitignore"), "build/\n");
      await writeFile(join(root, "src", "pkg_0", "m.py"), "eval(e)\n");
    });
    expect(listed.folders.length).toBeGreaterThan(0);
    expect(checked(audits)).toEqual([[".gitignore", "src/pkg_0/m.py"]]);
  });
});
