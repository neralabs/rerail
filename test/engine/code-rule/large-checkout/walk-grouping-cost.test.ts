import { execFileSync } from "node:child_process";
import { appendFile, lstat, mkdir, mkdtemp, realpath, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as StableJson from "../../../../src/engine/stable-json.js";

/** Every comparison of two paths in deterministic order Rerail makes, for the case that pins what a walk sorts. */
const comparisons = vi.hoisted(() => ({ count: 0 }));
vi.mock("../../../../src/engine/stable-json.js", async importOriginal => {
  const actual = await importOriginal<typeof StableJson>();
  return {
    ...actual,
    compareDeterministicStrings: (left: string, right: string): number => {
      comparisons.count += 1;
      return actual.compareDeterministicStrings(left, right);
    },
  };
});

import { CheckoutOwners } from "../../../../src/engine/catalog/checkout-owner.js";
import { CheckoutCodeRuleRouter } from "../../../../src/engine/code-rule/checkout-code-rule-router.js";
import { CodeRuleBridge } from "../../../../src/engine/code-rule/code-rule-bridge.js";
import { CheckoutCatalogs, startingCheckout } from "../../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { codeRuleSchema } from "../../../../src/engine/code-rule/code-rule-schema.js";
import type { CodeRuleAuditor, CodeRuleEditBatch } from "../../../../src/engine/code-rule/code-rule.js";
import { setUpTestRepository } from "../../../support/repository-rules.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  vi.restoreAllMocks();
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

async function temporaryRoot(): Promise<string> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-walk-grouping-")));
  cleanups.push(() => removeFolder(root));
  return root;
}

/** The relative path of the `at`th of a checkout's files, spread over `folders` folders as the issue's script spreads them. */
function sourcePath(at: number, folders: number): string {
  return `src/d${at % folders}/mod_${at}.py`;
}

/**
 * A committed repository at `root` of `count` small Python files in `folders` folders, and a `.gitignore`. Returns once
 * the file system's clock has passed their stamps, so the session that opens next sees none of them as written after it
 * began.
 */
async function committedCheckout(root: string, count: number, folders: number): Promise<void> {
  for (let folder = 0; folder < folders; folder += 1) await mkdir(join(root, "src", `d${folder}`), { recursive: true });
  for (let at = 0; at < count; at += 1) await writeFile(join(root, sourcePath(at, folders)), `def f_${at}(value):\n    return value * ${at}\n`);
  await writeFile(join(root, ".gitignore"), "node_modules/\n");
  git(root, "init", "--quiet");
  git(root, "add", "-A");
  git(root, "commit", "--quiet", "-m", "seed");
  const last = await lstat(join(root, sourcePath(count - 1, folders)));
  const tick = join(root, "node_modules", "tick");
  await mkdir(join(root, "node_modules"));
  for (;;) {
    await writeFile(tick, "");
    if ((await lstat(tick)).mtimeMs > Math.max(last.mtimeMs, last.ctimeMs)) return;
  }
}

/** The session's shell command, which `write` stands for: its result's walk finds and audits what it wrote. */
async function shellCommand(bridge: CodeRuleBridge, write: () => Promise<void>): Promise<void> {
  await bridge.begin("shell-1", "shell", { command: "./write.sh" });
  await write();
  await bridge.finish("shell-1");
}

/** A write to each of the checkout's files, which the next walk finds as a change of each. */
async function changeEvery(root: string, count: number, folders: number): Promise<void> {
  for (let at = 0; at < count; at += 1) await appendFile(join(root, sourcePath(at, folders)), "# changed\n");
}

/**
 * How many elements every iteration of an array yields while `run` runs. A spread (`[...group, item]`) copies each
 * element of its array through one, so a grouping that copies a group per item reads about n * n / 2 for n items in one
 * group, where one that appends in place reads none.
 */
async function arrayElementsRead(run: () => Promise<unknown>): Promise<number> {
  const original = Object.getOwnPropertyDescriptor(Array.prototype, Symbol.iterator);
  if (original === undefined) throw new Error("arrays have no iterator to count");
  const iterate: (this: unknown[]) => Iterator<unknown> = original.value;
  let read = 0;
  function counted(this: unknown[]): IterableIterator<unknown> {
    const inner = iterate.call(this);
    return {
      next() {
        const step = inner.next();
        if (step.done !== true) read += 1;
        return step;
      },
      [Symbol.iterator]() { return this; },
    };
  }
  Object.defineProperty(Array.prototype, Symbol.iterator, { ...original, value: counted });
  try {
    await run();
  } finally {
    Object.defineProperty(Array.prototype, Symbol.iterator, original);
  }
  return read;
}

const NEEDLE_RULE = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:needle", revision: 1, mode: "log",
  scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{ id: "needle", violation: { code: "N", anchor: "witness.span", message: "No NEEDLE" }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
});

/**
 * A session at the root of a committed checkout of `count` files, through the router a daemon gives it, with a rule
 * that reads every Python file; `owners` is the router's own lookup of a path's checkout, which asks the disk.
 */
async function routedSession(count: number, folders: number) {
  const base = await temporaryRoot();
  const root = join(base, "repo");
  await mkdir(root);
  await committedCheckout(root, count, folders);
  const databasePath = join(base, "rerail.sqlite");
  setUpTestRepository(root, { library: [{ kind: "code", rule: NEEDLE_RULE }] }, databasePath);
  const catalogs = new CheckoutCatalogs(databasePath);
  const owners = new CheckoutOwners(() => catalogs.recordedCheckoutsVersion());
  const router = new CheckoutCodeRuleRouter({
    sessionRoot: root,
    startingCheckout: startingCheckout(owners, root).checkoutRoot,
    owners,
    catalogs,
    followGitignore: true,
    steerDelivery: { requestSteer: async () => undefined },
    audits: { start: () => "audit", finish: () => undefined, progress: () => undefined, interrupt: () => undefined },
  });
  const bridge = new CodeRuleBridge(root, router, { discoveryWaitMs: Infinity });
  cleanups.push(async () => {
    await bridge.close();
    catalogs.close();
  });
  return { root, owners, bridge };
}

function response(batch: CodeRuleEditBatch) {
  return { revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] };
}

describe("a walk of a large checkout costs in proportion to its files", () => {
  it("groups the files a first walk sees, and the changes a later one finds, without copying a group per file", async () => {
    const count = 4_000;
    const { root, bridge } = await routedSession(count, 4);

    // At the first walk every file is seen first, and grouped by its checkout before Git lists the changed ones.
    const firstWalk = await arrayElementsRead(async () => {
      await bridge.openWorkspace("copies");
      await bridge.settleWalk();
    });
    // A shell command changes every file: its result's walk groups the 4,000 changes by checkout, and the router groups
    // them by lane to audit them.
    const laterWalk = await arrayElementsRead(async () => await shellCommand(bridge, () => changeEvery(root, count, 4)));

    // A group copied per file reads count * (count - 1) / 2 elements at each grouping, about 8,000,000; every other read
    // of the walk is a few hundred a file.
    expect.soft(firstWalk, "array elements read at the first walk").toBeLessThan((count * count) / 4);
    expect.soft(laterWalk, "array elements read at the command's walk").toBeLessThan((count * count) / 4);
  }, 120_000);

  it("sorts only the changes a walk finds, not every file it sees", async () => {
    const count = 2_000;
    const root = await temporaryRoot();
    await committedCheckout(root, count, 4);
    const auditor: CodeRuleAuditor = {
      openWorkspace: async () => undefined,
      acceptsPath: path => path.endsWith(".py"),
      auditGeneratedEdits: async (_sessionId, batch) => response(batch),
      close: async () => undefined,
    };
    const bridge = new CodeRuleBridge(root, auditor, { discoveryWaitMs: Infinity });
    cleanups.push(() => bridge.close());
    await bridge.openWorkspace("sorts");
    await bridge.settleWalk();

    // A turn's end walks the whole folder; one file changed since the walk before. Sorting the 2,000 files the walk
    // sees takes about 2,000 * 11 comparisons.
    await appendFile(join(root, sourcePath(7, 4)), "# changed\n");
    comparisons.count = 0;
    await bridge.checkpoint("turn");
    expect(comparisons.count).toBeLessThan(count / 10);
  }, 60_000);

  it("asks the disk for the checkout of each folder a walk found once, not for each file it groups", async () => {
    const count = 400;
    const folders = 4;
    const { root, owners, bridge } = await routedSession(count, folders);
    const lookups = vi.spyOn(owners, "ownerOf");

    await bridge.openWorkspace("owners");
    await bridge.settleWalk();
    const firstWalk = lookups.mock.calls.length;
    lookups.mockClear();
    await shellCommand(bridge, () => changeEvery(root, count, folders));
    const laterWalk = lookups.mock.calls.length;

    // The first walk groups the 400 files it sees first: a lookup each would be 400. The command's walk groups the 400
    // changes, which would be 400 more; beside the grouping, the router looks up each change's checkout as it routes it
    // to its lane, and the changes with no preimage captured are grouped by checkout for Git's blobs, a lookup each, as
    // for any edit (2 * 400).
    expect.soft(firstWalk, "checkout lookups at the first walk").toBeLessThan(count / 4);
    expect.soft(laterWalk, "checkout lookups at the command's walk").toBeLessThan(2 * count + count / 4);
  }, 60_000);

  it("groups a walk's changes as before: unowned ones first, then each checkout's in the order of its first, each in path order", async () => {
    const root = await temporaryRoot();
    const nested = join(root, "m");
    await mkdir(nested);
    await writeFile(join(nested, "seed.txt"), "seed\n");
    git(nested, "init", "--quiet");
    git(nested, "add", "-A");
    git(nested, "commit", "--quiet", "-m", "seed");
    await writeFile(join(root, "seed.txt"), "seed\n");
    git(root, "init", "--quiet");
    git(root, "add", "-A");
    git(root, "commit", "--quiet", "-m", "seed");
    // Files under b/ and y/ belong to no checkout the auditor runs; under m/, to the nested one; any other, to the root.
    const checkoutOf = (path: string): string | undefined => {
      if (path.startsWith("b/") || path.startsWith("y/")) return undefined;
      return path.startsWith("m/") ? nested : root;
    };
    const audited: string[][] = [];
    const auditor: CodeRuleAuditor = {
      openWorkspace: async () => undefined,
      acceptsPath: path => path.endsWith(".py"),
      sourceRoot: checkoutOf,
      walkView: () => ({ acceptsPath: path => path.endsWith(".py"), sourceRoot: checkoutOf }),
      auditGeneratedEdits: async (_sessionId, batch) => {
        audited.push(batch.edits.map(edit => edit.path));
        return response(batch);
      },
      close: async () => undefined,
    };
    const bridge = new CodeRuleBridge(root, auditor, { discoveryWaitMs: Infinity });
    cleanups.push(() => bridge.close());
    await bridge.openWorkspace("groups");
    await bridge.settleWalk();

    const written = ["z/6.py", "m/sub/5.py", "a/1.py", "y/7.py", "n/4.py", "m/2.py", "b/3.py"];
    await shellCommand(bridge, async () => {
      for (const path of written) {
        await mkdir(join(root, path, ".."), { recursive: true });
        await writeFile(join(root, path), `value = "${path}"\n`);
      }
    });

    expect(audited.flat()).toEqual(["b/3.py", "y/7.py", "a/1.py", "n/4.py", "z/6.py", "m/2.py", "m/sub/5.py"]);
  }, 60_000);

  it("groups a walk's changes under the session folder, as a path it is named, for an auditor that judges one folder whatever its view says", async () => {
    const root = await temporaryRoot();
    await writeFile(join(root, "seed.txt"), "seed\n");
    git(root, "init", "--quiet");
    git(root, "add", "-A");
    git(root, "commit", "--quiet", "-m", "seed");
    // The auditor has no `sourceRoot`, so a path it is named belongs to the session folder; its walk view says files
    // under b/ and y/ belong to no checkout, which would group them apart, first.
    const audited: string[][] = [];
    const auditor: CodeRuleAuditor = {
      openWorkspace: async () => undefined,
      acceptsPath: path => path.endsWith(".py"),
      walkView: () => ({
        acceptsPath: path => path.endsWith(".py"),
        sourceRoot: path => (path.startsWith("b/") || path.startsWith("y/") ? undefined : root),
      }),
      auditGeneratedEdits: async (_sessionId, batch) => {
        audited.push(batch.edits.map(edit => edit.path));
        return response(batch);
      },
      close: async () => undefined,
    };
    const bridge = new CodeRuleBridge(root, auditor, { discoveryWaitMs: Infinity });
    cleanups.push(() => bridge.close());
    await bridge.openWorkspace("one-folder");
    await bridge.settleWalk();

    await shellCommand(bridge, async () => {
      for (const path of ["z/6.py", "a/1.py", "y/7.py", "n/4.py", "b/3.py"]) {
        await mkdir(join(root, path, ".."), { recursive: true });
        await writeFile(join(root, path), `value = "${path}"\n`);
      }
    });

    expect(audited.flat()).toEqual(["a/1.py", "b/3.py", "n/4.py", "y/7.py", "z/6.py"]);
  }, 60_000);
});
