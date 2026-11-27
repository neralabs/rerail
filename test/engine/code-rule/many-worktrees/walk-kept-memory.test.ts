/**
 * What the walks of a repository's other worktrees keep in memory for each file of a worktree they watch. The worktrees
 * of one repository hold mostly the same files under the same paths, so the walks keep a file's path and digest once
 * for every worktree that holds the file the same; each worktree still keeps an observation of its own, since its stamp
 * is its own. With 190 worktrees of 10,000 files each, the walks kept 668 MB, a third of it the same path and digest
 * strings once a worktree.
 */
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, realpath, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve, sep } from "node:path";
import { getHeapSnapshot } from "node:v8";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CodeRuleBridge } from "../../../../src/engine/code-rule/code-rule-bridge.js";
import type { CodeRuleAuditor, CodeRuleEditBatch } from "../../../../src/engine/code-rule/code-rule.js";
import { WorktreeWalks } from "../../../../src/engine/code-rule/worktree-walks.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

/** Files in each checkout, every one of them counted. */
const FILES = 200;

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

/** File `file`'s path in each checkout, as the walks name it, and its text. */
const pathOf = (file: number): string => `src/handlers/package_${Math.floor(file / 50)}/module_${file % 50}.py`;
const textOf = (file: number): string => `def value_${file}():\n    return ${file}\n`;

/** A repository at `main` of `FILES` Python files, each with a text of its own, and `count` worktrees beside it; physical paths. */
async function repository(count: number): Promise<{ readonly main: string; readonly worktrees: readonly string[] }> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-walk-memory-")));
  cleanups.push(async () => await removeFolder(root));
  const main = join(root, "main");
  for (let file = 0; file < FILES; file += 1) {
    const path = join(main, ...pathOf(file).split("/"));
    await mkdir(join(path, ".."), { recursive: true });
    await writeFile(path, textOf(file));
  }
  git(main, "init", "--quiet");
  git(main, "config", "core.autocrlf", "false");
  git(main, "add", ".");
  git(main, "commit", "--quiet", "-m", "seed");
  const worktrees = Array.from({ length: count }, (_, index) => join(root, `w${index + 1}`));
  for (const worktree of worktrees) git(main, "worktree", "add", "--quiet", "--detach", worktree);
  return { main, worktrees };
}

/** A session opened in `folder` whose walks watch the checkouts `watched` holds as each walk begins, as a router lists them. */
async function sessionWatching(folder: string, watched: readonly string[], walks: WorktreeWalks): Promise<CodeRuleBridge> {
  const holding = (absolute: string): string | undefined => [folder, ...watched]
    .filter(checkout => absolute === checkout || absolute.startsWith(`${checkout}${sep}`))
    .sort((a, b) => b.length - a.length)[0];
  const auditor: CodeRuleAuditor = {
    openWorkspace: async () => undefined,
    acceptsPath: () => true,
    refreshCodeRules: () => undefined,
    auditGeneratedEdits: vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => ({
      revision: batch.revision, disposition: "continue" as const, violations: [], failures: [],
    })),
    close: async () => undefined,
    sourceRoot: path => holding(resolve(folder, path)),
    worktreeRoots: () => [...watched],
  };
  const bridge = new CodeRuleBridge(folder, auditor, { worktreeWalks: walks });
  cleanups.push(async () => await bridge.close());
  await bridge.openWorkspace("session-1");
  return bridge;
}

/** A turn of the session, the walk of the other worktrees after it, and any walk still timed or reading. */
async function walked(bridge: CodeRuleBridge, walks: WorktreeWalks): Promise<void> {
  await bridge.checkpoint("turn");
  await bridge.settleWorktreeWalk();
  await walks.idle();
}

/** The parts of a V8 heap snapshot a count of strings reads. */
interface HeapSnapshot {
  readonly snapshot: { readonly meta: { readonly node_fields: string[]; readonly node_types: [string[], ...unknown[]] } };
  readonly nodes: number[];
  readonly strings: string[];
}

/** The files' paths and digests, made only once a snapshot is taken, so the test holds no copy of its own in the heap. */
const textsOf = {
  paths: (): Set<string> => new Set(Array.from({ length: FILES }, (_, file) => pathOf(file))),
  digests: (): Set<string> => new Set(Array.from({ length: FILES }, (_, file) => createHash("sha256").update(textOf(file)).digest("hex"))),
};

/**
 * How many strings in the heap hold one of the files' paths, and how many one of their digests, after the collection a
 * heap snapshot runs first: each string a copy kept somewhere, so two copies of one text are two strings of the
 * snapshot with that name.
 */
async function copies(): Promise<{ readonly paths: number; readonly digests: number }> {
  const chunks: Buffer[] = [];
  for await (const chunk of getHeapSnapshot() as AsyncIterable<Buffer>) chunks.push(chunk);
  const { snapshot, nodes, strings } = JSON.parse(Buffer.concat(chunks).toString("utf8")) as HeapSnapshot;
  const fields = snapshot.meta.node_fields;
  const string = snapshot.meta.node_types[0].indexOf("string");
  const [type, name] = [fields.indexOf("type"), fields.indexOf("name")];
  const [paths, digests] = [textsOf.paths(), textsOf.digests()];
  const counts = { paths: 0, digests: 0 };
  for (let node = 0; node < nodes.length; node += fields.length) {
    if (nodes[node + type] !== string) continue;
    const text = strings[nodes[node + name]!]!;
    if (paths.has(text)) counts.paths += 1;
    if (digests.has(text)) counts.digests += 1;
  }
  return counts;
}

describe("the walks of the other worktrees", () => {
  it("keep a file's path and digest once for all the worktrees that hold it the same", { timeout: 120_000 }, async () => {
    const { main, worktrees } = await repository(6);
    const [own, first, ...more] = worktrees;
    // Each count with one session whose first walk reads the checkouts it watches, closed before the next one opens: the
    // walks drop what they kept once no session watches, so each count holds one session's walks alone.
    const held = async (watched: readonly string[]): Promise<{ readonly paths: number; readonly digests: number }> => {
      const walks = new WorktreeWalks({ gapMs: 0 });
      const bridge = await sessionWatching(own!, watched, walks);
      await walked(bridge, walks);
      expect(walks.observations()).toBe(watched.length * FILES);
      const counted = await copies();
      await bridge.close();
      await walks.idle();
      expect(walks.observations()).toBe(0);
      return counted;
    };
    const two = await held([main, first!]);
    const six = await held([main, first!, ...more]);
    // Four more worktrees hold every file the same: the files' paths and digests are held as many times as with two,
    // where each worktree kept copies of its own, four more of each a file.
    expect(six.paths - two.paths).toBe(0);
    expect(six.digests - two.digests).toBe(0);
  });
});
