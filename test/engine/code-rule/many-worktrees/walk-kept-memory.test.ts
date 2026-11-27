/**
 * What the walks of a repository's other worktrees keep in memory for each file of a worktree they watch. The worktrees
 * of one repository hold mostly the same files under the same paths, so a worktree's observation of a file takes the
 * path and digest strings of the same file's observation in another worktree the walks keep, where they are the same
 * text; each worktree still keeps an observation of its own, since its stamp is its own. With 190 worktrees of 10,000
 * files each, the walks kept 700 MB, a third of it the same path and digest strings once a worktree. A walk names a
 * file by the part of its absolute path after the checkout, and keeps that part as a string of its own: kept as a view
 * of the absolute path, it kept the absolute path too, one a file a worktree.
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

/**
 * File `file`'s path in each checkout, as the walks name it, and its text. A quarter of the files sit at the checkout's
 * root, where a Windows path holds no separator to turn into a slash.
 */
const pathOf = (file: number): string =>
  file % 4 === 0 ? `module_at_the_root_${file}.py` : `src/handlers/package_${Math.floor(file / 50)}/module_${file % 50}.py`;
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
  readonly snapshot: {
    readonly meta: {
      readonly node_fields: string[];
      readonly node_types: [string[], ...unknown[]];
      readonly edge_fields: string[];
      readonly edge_types: [string[], ...unknown[]];
    };
  };
  readonly nodes: number[];
  readonly edges: number[];
  readonly strings: string[];
}

/** How many strings in the heap hold one of the files' paths, one of their digests, one of their absolute paths. */
interface Copies {
  readonly paths: number;
  readonly digests: number;
  readonly absolutes: number;
}

/** The files' paths and digests, made only once a snapshot is taken, so the test holds no copy of its own in the heap. */
const textsOf = {
  paths: (): Set<string> => new Set(Array.from({ length: FILES }, (_, file) => pathOf(file))),
  digests: (): Set<string> => new Set(Array.from({ length: FILES }, (_, file) => createHash("sha256").update(textOf(file)).digest("hex"))),
};

/**
 * The text each string of `heap` holds, whatever kind V8 lists it as. A `string` is listed with its text; a
 * `concatenated string` with its two parts, its text unless it is flat (its second part empty), when its first part is
 * the string holding the text; a `sliced string` with the string it was cut from and not where, so one cut from a file's
 * absolute path holds what `below` gives of it: the part after the checkout and a separator, as a walk cuts it.
 */
function textsIn(heap: HeapSnapshot, below: (absolute: string) => string | undefined): string[] {
  const { node_fields: fields, node_types: [kinds], edge_fields: edgeFields, edge_types: [edgeKinds] } = heap.snapshot.meta;
  const [type, name, edgeCount] = [fields.indexOf("type"), fields.indexOf("name"), fields.indexOf("edge_count")];
  const [edgeType, edgeName, to] = [edgeFields.indexOf("type"), edgeFields.indexOf("name_or_index"), edgeFields.indexOf("to_node")];
  const firstEdge = [0];
  for (let node = 0; node < heap.nodes.length; node += fields.length) firstEdge.push(firstEdge.at(-1)! + heap.nodes[node + edgeCount]!);
  // The text of the string `node` holds as its part `wanted`, followed at most 64 strings deep, as no path is built.
  const partText = (node: number, wanted: string, depth: number): string | undefined => {
    for (let at = firstEdge[node]! * edgeFields.length; at < firstEdge[node + 1]! * edgeFields.length; at += edgeFields.length) {
      const named = edgeKinds[heap.edges[at + edgeType]!] === "internal" && heap.strings[heap.edges[at + edgeName]!] === wanted;
      if (named) return depth < 64 ? textOfNode(heap.edges[at + to]! / fields.length, depth + 1) : undefined;
    }
    return undefined;
  };
  const joined = (node: number, depth: number): string | undefined => {
    const [first, second] = [partText(node, "first", depth), partText(node, "second", depth)];
    return first === undefined || !second ? undefined : first + second;
  };
  const cut = (node: number, depth: number): string | undefined => {
    const parent = partText(node, "parent", depth);
    return parent === undefined ? undefined : below(parent);
  };
  const textOfNode = (node: number, depth = 0): string | undefined => {
    const kind = kinds[heap.nodes[node * fields.length + type]!];
    if (kind === "string") return heap.strings[heap.nodes[node * fields.length + name]!];
    return kind === "concatenated string" ? joined(node, depth) : kind === "sliced string" ? cut(node, depth) : undefined;
  };
  return Array.from({ length: firstEdge.length - 1 }, (_, node) => textOfNode(node)).filter(text => text !== undefined);
}

/**
 * How many strings in the heap hold one of the files' paths, digests and absolute paths in `checkouts`, after the
 * collection a heap snapshot runs first: each string a copy kept somewhere, whatever kind V8 lists it as (`textsIn`).
 */
async function copies(checkouts: readonly string[]): Promise<Copies> {
  const chunks: Buffer[] = [];
  for await (const chunk of getHeapSnapshot() as AsyncIterable<Buffer>) chunks.push(chunk);
  const heap = JSON.parse(Buffer.concat(chunks).toString("utf8")) as HeapSnapshot;
  const [paths, digests] = [textsOf.paths(), textsOf.digests()];
  const absolutes = new Map(checkouts.flatMap(checkout => [...paths].map(path => [join(checkout, ...path.split("/")), checkout] as const)));
  const below = (absolute: string): string | undefined => {
    const checkout = absolutes.get(absolute);
    return checkout === undefined ? undefined : absolute.slice(checkout.length + 1);
  };
  const counts = { paths: 0, digests: 0, absolutes: 0 };
  for (const text of textsIn(heap, below)) {
    if (paths.has(text)) counts.paths += 1;
    if (digests.has(text)) counts.digests += 1;
    if (absolutes.has(text)) counts.absolutes += 1;
  }
  return counts;
}

/**
 * What one session opened in `own` keeps once its first walk has read the checkouts `watched` holds, its copies of the
 * files' strings in `checkouts` counted; the session is closed before the next count, and the walks drop what they kept
 * once no session watches, so each count holds one session's walks alone.
 */
async function held(own: string, watched: readonly string[], checkouts: readonly string[]): Promise<Copies> {
  const walks = new WorktreeWalks({ gapMs: 0 });
  const bridge = await sessionWatching(own, watched, walks);
  await walked(bridge, walks);
  expect(walks.observations()).toBe(watched.length * FILES);
  const counted = await copies(checkouts);
  await bridge.close();
  await walks.idle();
  expect(walks.observations()).toBe(0);
  return counted;
}

describe("the walks of the other worktrees", () => {
  it("share a file's path and digest with another worktree that holds the file the same", { timeout: 120_000 }, async () => {
    const { main, worktrees } = await repository(6);
    const [own, first, ...more] = worktrees;
    const two = await held(own!, [main, first!], [main, ...worktrees]);
    const six = await held(own!, [main, first!, ...more], [main, ...worktrees]);
    // The count sees the strings it counts, whatever kind holds them: with two worktrees each file's path and digest
    // are held once at least.
    expect(two.paths).toBeGreaterThanOrEqual(FILES);
    expect(two.digests).toBeGreaterThanOrEqual(FILES);
    // Four more worktrees hold every file the same: the files' paths and digests are held as many times as with two,
    // where each worktree kept copies of its own, four more of each a file.
    expect(six.paths - two.paths).toBe(0);
    expect(six.digests - two.digests).toBe(0);
  });

  it("keep no file's absolute path, whichever system cuts a file's path from it", { timeout: 120_000 }, async () => {
    const { main, worktrees } = await repository(2);
    const [own, first] = worktrees;
    // The session folder's own walk and the walks of the two others each name every file by a part of its absolute path.
    const counted = await held(own!, [main, first!], [main, ...worktrees]);
    expect(counted.absolutes).toBe(0);
  });
});
