/**
 * What a walk of the session folder asks the disk, on the daemon's thread, to learn which checkout holds each folder it
 * walks: each lstatSync, existsSync and realpathSync naming a folder the walk entered or a path in it, over the walk at
 * a turn's end of a checkout of 200 folders; and the checkout the walk's view, and the grouping of the walk's changes by
 * checkout, give each file it found.
 */
import { execFileSync } from "node:child_process";
import type * as FileSystem from "node:fs";
import { appendFile, mkdir, mkdtemp, realpath, rename, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

/** While `on`: each look on the daemon's thread, by kind and path. */
const disk = vi.hoisted(() => ({ on: false, looks: [] as Array<{ readonly kind: string; readonly path: string }> }));
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

import { CheckoutOwners } from "../../../../src/engine/catalog/checkout-owner.js";
import { CheckoutCodeRuleRouter } from "../../../../src/engine/code-rule/checkout-code-rule-router.js";
import { CodeRuleBridge } from "../../../../src/engine/code-rule/code-rule-bridge.js";
import { CheckoutCatalogs, startingCheckout } from "../../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { codeRuleSchema } from "../../../../src/engine/code-rule/code-rule-schema.js";
import type { WalkView } from "../../../../src/engine/code-rule/code-rule.js";
import { setUpTestRepository } from "../../../support/repository-rules.js";
import { linkFolder } from "../../../support/platform/links.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  disk.on = false;
  vi.restoreAllMocks();
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "-c", "protocol.file.allow=always", ...args], { cwd, stdio: "ignore" });
}

const NEEDLE_RULE = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:needle", revision: 1, mode: "log",
  scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{ id: "needle", violation: { code: "N", anchor: "witness.span", message: "No NEEDLE" }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
});

/** The tree's top folders, each holding `LEAVES`: 200 folders in all, each holding one Python file. */
const TOPS = Array.from({ length: 10 }, (_, at) => `d${at}`);
const LEAVES = Array.from({ length: 19 }, (_, at) => `e${at}`);
const FOLDERS = TOPS.flatMap(top => [top, ...LEAVES.map(leaf => `${top}/${leaf}`)]);

/** What a walk of the tree asks of each folder whether Git ignores it: none of the tree's, and `out/`. */
const EVERY_FOLDER_ENTERED = Object.fromEntries([...FOLDERS.map(folder => [folder, false]), ["out", true]]);

/** Each file of the tree, relative to the checkout: one in each folder. */
const FILES = FOLDERS.map(folder => `${folder}/${folder.includes("/") ? "x" : "m"}.py`);

/** Plants one top folder's files at `top`: its own, and one in each of its `LEAVES`. */
async function plantTop(top: string): Promise<void> {
  await mkdir(top, { recursive: true });
  await writeFile(join(top, "m.py"), "m = 1\n");
  for (const leaf of LEAVES) {
    await mkdir(join(top, leaf));
    await writeFile(join(top, leaf, "x.py"), "x = 1\n");
  }
}

/** A committed repository at `root` holding `tops`, and a `.gitignore` of `ignored` with a file in an `out/` it ignores. */
async function committedCheckout(root: string, tops: readonly string[], ignored: readonly string[]): Promise<void> {
  for (const top of tops) await plantTop(join(root, top));
  await mkdir(join(root, "out"), { recursive: true });
  await writeFile(join(root, "out", "made.py"), "made = 1\n");
  await writeFile(join(root, ".gitignore"), ["out/", ...ignored].map(line => `${line}\n`).join(""));
  git(root, "init", "--quiet", "-b", "main");
  git(root, "add", "-A");
  git(root, "commit", "--quiet", "-m", "seed");
}

/** A committed repository at `root` holding one top folder's files, as `d0` holds them, to clone. */
async function seedRepository(root: string): Promise<void> {
  await plantTop(root);
  git(root, "init", "--quiet", "-b", "main");
  git(root, "add", "-A");
  git(root, "commit", "--quiet", "-m", "seed");
}

interface Walked {
  /** The session's folder, as the session names it. */
  readonly root: string;
  readonly bridge: CodeRuleBridge;
  /** The session's folder as the bridge walks it: its real path. */
  readonly real: string;
  /** The checkout a router given then, with nothing kept from before, gives the session's `path`. */
  readonly fresh: (path: string) => Promise<string | undefined>;
  /** Each folder a walk asked whether Git ignores, with the answer, since `entered.clear()`. */
  readonly entered: Map<string, boolean>;
  /** The views of the auditor each walk asked for, with the folder it started at. */
  readonly views: () => ReadonlyArray<{ readonly start: string | undefined; readonly view: WalkView }>;
  /**
   * Each path a view of the auditor was asked the checkout of (`sourceRoot`), with its answer, since `grouped.length = 0`:
   * within a walk only the grouping of what it found by checkout asks it.
   */
  readonly grouped: Array<[string, string | undefined]>;
  /** The router's lookup of a path's checkout, which asks the disk. */
  readonly owners: CheckoutOwners;
}

/**
 * A session at `root`, through the router a daemon gives it, following `.gitignore`; the checkout at `recorded` (`root`
 * by default) recorded with `databasePath`, and each of `plainRoots` taken as a recorded checkout outside Git, as the
 * runtime takes one.
 */
async function routedSession(root: string, databasePath: string, recorded = root, plainRoots: readonly string[] = []): Promise<Walked> {
  setUpTestRepository(recorded, { library: [{ kind: "code", rule: NEEDLE_RULE }] }, databasePath);
  const catalogs = new CheckoutCatalogs(databasePath);
  const routed = (owners = new CheckoutOwners(() => catalogs.recordedCheckoutsVersion())): CheckoutCodeRuleRouter => {
    for (const plain of plainRoots) owners.addPlainRoot(plain);
    return new CheckoutCodeRuleRouter({
      sessionRoot: root, startingCheckout: startingCheckout(owners, root).checkoutRoot, owners, catalogs, followGitignore: true,
      steerDelivery: { requestSteer: async () => undefined },
      audits: { start: () => "audit", finish: () => undefined, progress: () => undefined, interrupt: () => undefined },
    });
  };
  const owners = new CheckoutOwners(() => catalogs.recordedCheckoutsVersion());
  const router = routed(owners);
  const fresh = async (path: string): Promise<string | undefined> => {
    const other = routed();
    try {
      return other.sourceRoot(path);
    } finally {
      await other.close();
    }
  };
  // Each view a walk asks for, its answers whether Git ignores a folder kept in `entered`, its checkouts in `grouped`.
  const entered = new Map<string, boolean>();
  const grouped: Array<[string, string | undefined]> = [];
  const walkView = router.walkView.bind(router);
  const asked = vi.spyOn(router, "walkView").mockImplementation(start => {
    const view = walkView(start);
    return {
      ...view,
      ignoresPath: (path, directory) => {
        const ignored = view.ignoresPath(path, directory);
        if (directory) entered.set(path, ignored);
        return ignored;
      },
      sourceRoot: path => {
        const checkout = view.sourceRoot(path);
        grouped.push([path, checkout]);
        return checkout;
      },
    };
  });
  const bridge = new CodeRuleBridge(root, router, { discoveryWaitMs: Infinity });
  cleanups.push(async () => {
    await bridge.close();
    catalogs.close();
  });
  const views = (): Array<{ start: string | undefined; view: WalkView }> =>
    asked.mock.calls.map(([start], at) => ({ start, view: asked.mock.results[at]?.value as WalkView }));
  return { root, real: await realpath(root), bridge, fresh, entered, views, grouped, owners };
}

async function temporaryRoot(): Promise<string> {
  const base = await realpath(await mkdtemp(join(tmpdir(), "rerail-walk-folder-owners-")));
  cleanups.push(() => removeFolder(base));
  return base;
}

/**
 * The looks of the walk at a turn's end, after the session's first walk, that name a folder of `FOLDERS` or a path in
 * it, by that folder (the deepest that holds the path): a `.gitignore` apart, which Git's ignore answers look at once a
 * folder a walk; the looks at the session folder's `.git`, which the walk's top holds; and the view that walk asked of
 * the auditor.
 */
async function turnWalk(session: Walked): Promise<{ owners: Map<string, number>; ignoreFiles: number; atTop: number; view: WalkView }> {
  const { root, bridge } = session;
  await bridge.openWorkspace("session");
  await bridge.settleWalk();
  disk.looks = [];
  session.entered.clear();
  disk.on = true;
  let view: WalkView;
  try {
    view = await walkAtTurn(session);
  } finally {
    disk.on = false;
  }
  const owners = new Map<string, number>();
  let ignoreFiles = 0;
  const atTop = disk.looks.filter(({ path }) => path === join(root, ".git")).length;
  for (const { path } of disk.looks) {
    const folder = [...FOLDERS].reverse().find(candidate => path === join(root, candidate) || path.startsWith(join(root, candidate) + sep));
    if (folder === undefined) continue;
    if (basename(path) === ".gitignore") ignoreFiles += 1;
    else owners.set(folder, (owners.get(folder) ?? 0) + 1);
  }
  return { owners, ignoreFiles, atTop, view };
}

/** The view the walk at a turn's end asked of the auditor, once that walk is done. */
async function walkAtTurn({ root, real, bridge, views }: Walked): Promise<WalkView> {
  const before = views().length;
  await bridge.checkpoint("turn");
  const walks = views().slice(before).filter(({ start }) => start === root || start === real);
  expect(walks).toHaveLength(1);
  return walks[0]!.view;
}

/** How many looks `owners` counts at the folders `within` names. */
function looksAt(owners: ReadonlyMap<string, number>, within: (folder: string) => boolean): number {
  return [...owners].filter(([folder]) => within(folder)).reduce((sum, [, looks]) => sum + looks, 0);
}

describe("a walk of a checkout of many folders", () => {
  it("asks the disk nothing to learn the checkout of a folder whose listing holds no .git entry", async () => {
    const base = await temporaryRoot();
    const root = join(base, "repo");
    await committedCheckout(root, TOPS, []);
    const session = await routedSession(root, join(base, "rerail.sqlite"));

    const { owners, ignoreFiles, atTop, view } = await turnWalk(session);

    // An lstat of each folder's .git, asked as the folder's parent listed it, before the folder's own listing named its
    // entries: 200 a walk. The walk's top, whose listing names .git, is still looked up: the looks are counted.
    expect.soft(looksAt(owners, () => true), `looks at the folders walked: ${JSON.stringify([...owners])}`).toBe(0);
    expect.soft(atTop, "looks at the session folder's .git").toBeGreaterThan(0);
    // Git's ignore answers look at each folder's .gitignore once a walk; this change leaves them as they are.
    expect.soft(ignoreFiles, "looks at a .gitignore of the folders walked").toBe(FOLDERS.length);
    expect(FILES.filter(path => view.sourceRoot?.(path) !== root)).toEqual([]);
    expect(Object.fromEntries(session.entered)).toEqual(EVERY_FOLDER_ENTERED);
  }, 120_000);

  it("finds a checkout of its own at each folder whose listing holds a .git entry, at what a lookup of it costs", async () => {
    const base = await temporaryRoot();
    const root = join(base, "repo");
    const databasePath = join(base, "rerail.sqlite");
    // d0 is a clone the session's checkout ignores, started; d1 a submodule; d2 a worktree of d0; d3 a `git init` nobody
    // started, whose files the checkout around it judges.
    await committedCheckout(root, TOPS.slice(3), ["d0/"]);
    const seed = join(base, "seed");
    await seedRepository(seed);
    git(root, "clone", "--quiet", seed, "d0");
    git(root, "submodule", "add", "--quiet", seed, "d1");
    git(root, "commit", "--quiet", "-m", "submodule");
    git(join(root, "d0"), "worktree", "add", "--quiet", "-b", "linked", join(root, "d2"));
    git(join(root, "d3"), "init", "--quiet");
    setUpTestRepository(join(root, "d0"), { library: [{ kind: "code", rule: NEEDLE_RULE }] }, databasePath);
    const session = await routedSession(root, databasePath);

    const { owners, view } = await turnWalk(session);

    const ownTop = (folder: string): string | undefined => ["d0", "d1", "d2", "d3"].find(top => folder === top);
    expect.soft(looksAt(owners, folder => ownTop(folder) === undefined), `looks at the folders with no .git: ${JSON.stringify([...owners])}`).toBe(0);
    // A folder holding .git costs its lookup, and the lookup of the checkout of the .git entry its listing names, as at
    // the base: 4 or 5 looks there.
    const atEach = ["d0", "d1", "d2", "d3"].map(top => owners.get(top) ?? 0);
    expect.soft(Math.max(...atEach), `looks at the folders holding .git: ${JSON.stringify([...owners])}`).toBeLessThanOrEqual(5);
    expect.soft(Math.min(...atEach), `looks at the folders holding .git: ${JSON.stringify([...owners])}`).toBeGreaterThan(0);
    const expected = (path: string): string => {
      const top = path.slice(0, path.indexOf("/"));
      return ["d0", "d1", "d2"].includes(top) ? join(root, top) : root;
    };
    expect(FILES.filter(path => view.sourceRoot?.(path) !== expected(path))).toEqual([]);
    // The clone the checkout ignores is walked, as a checkout of its own.
    expect(Object.fromEntries(session.entered)).toEqual(EVERY_FOLDER_ENTERED);
  }, 120_000);

  it("groups the changes a walk finds in each folder by checkout with its own view, looking none of the folders up", async () => {
    const base = await temporaryRoot();
    const root = join(base, "repo");
    await committedCheckout(root, TOPS, []);
    const session = await routedSession(root, join(base, "rerail.sqlite"));
    await session.bridge.openWorkspace("session");
    await session.bridge.settleWalk();
    for (const path of FILES) await appendFile(join(root, path), "# changed\n");
    const lookups = [vi.spyOn(session.owners, "ownerOf"), vi.spyOn(session.owners, "ownerOfReal")];

    // A turn's end reads the whole folder, whatever the change notices name.
    await walkAtTurn(session);

    const asked = lookups.flatMap(lookup => lookup.mock.calls.map(([path]) => path));
    const kinds = `ownerOf ${lookups[0]!.mock.calls.length}, ownerOfReal ${lookups[1]!.mock.calls.length}`;
    // The walk looks up its top and the .git entry its listing names: the lookups are counted.
    expect.soft(asked.length, kinds).toBeGreaterThan(0);
    // A grouping with a view of its own looked up each folder holding a change: 200.
    expect(asked.filter(path => FOLDERS.some(folder => path === join(root, folder))).length, kinds).toBe(0);
  }, 120_000);
});

/** A committed repository at `root` holding `a/b/x.py`. */
async function smallCheckout(root: string): Promise<void> {
  await mkdir(join(root, "a", "b"), { recursive: true });
  await writeFile(join(root, "a", "b", "x.py"), "x = 1\n");
  git(root, "init", "--quiet", "-b", "main");
  git(root, "add", "-A");
  git(root, "commit", "--quiet", "-m", "seed");
}

/** Each of `paths`, with the checkout the walk's view gives it, and the checkout a router given now gives it. */
async function handedTo(session: Walked, view: WalkView, paths: readonly string[]): Promise<Array<[string, string | undefined, string | undefined]>> {
  const handed: Array<[string, string | undefined, string | undefined]> = [];
  for (const path of paths) handed.push([path, view.sourceRoot?.(path), await session.fresh(path)]);
  return handed;
}

describe("the checkout a walk gives each file it found", () => {
  it("is the full lookup's for a session reached through a link", async () => {
    const base = await temporaryRoot();
    const repo = join(base, "repo");
    await smallCheckout(repo);
    const link = join(base, "link");
    linkFolder(repo, link);
    const session = await routedSession(link, join(base, "rerail.sqlite"), repo);
    await session.bridge.openWorkspace("session");
    await session.bridge.settleWalk();

    const view = await walkAtTurn(session);

    expect(await handedTo(session, view, ["a/b/x.py", "a/y.py"])).toEqual([["a/b/x.py", repo, repo], ["a/y.py", repo, repo]]);
  }, 60_000);

  it.runIf(process.platform === "win32" || process.platform === "darwin")("is the full lookup's for a session named in another letter case", async () => {
    const base = await temporaryRoot();
    const repo = join(base, "repo");
    await smallCheckout(repo);
    const session = await routedSession(join(base, "REPO"), join(base, "rerail.sqlite"), repo);
    await session.bridge.openWorkspace("session");
    await session.bridge.settleWalk();

    const view = await walkAtTurn(session);

    const handed = await handedTo(session, view, ["a/b/x.py"]);
    expect(handed).toEqual([["a/b/x.py", handed[0]![2], handed[0]![2]]]);
    expect(handed[0]![2]?.toLowerCase()).toBe(repo.toLowerCase());
  }, 60_000);

  it("follows a folder that becomes a checkout between two walks, and one that stops being one", async () => {
    const base = await temporaryRoot();
    const repo = join(base, "repo");
    await smallCheckout(repo);
    const later = join(repo, "later");
    await mkdir(join(later, "a", "b"), { recursive: true });
    await writeFile(join(later, "a", "b", "x.py"), "x = 2\n");
    const session = await routedSession(repo, join(base, "rerail.sqlite"));
    await session.bridge.openWorkspace("session");
    await session.bridge.settleWalk();
    const plain = await handedTo(session, await walkAtTurn(session), ["later/a/b/x.py"]);

    await removeFolder(later);
    git(repo, "worktree", "add", "--quiet", "--detach", later);
    const checkout = await handedTo(session, await walkAtTurn(session), ["later/a/b/x.py"]);
    git(repo, "worktree", "remove", "--force", later);
    await mkdir(join(later, "a", "b"), { recursive: true });
    await writeFile(join(later, "a", "b", "x.py"), "x = 3\n");
    session.grouped.length = 0;
    const view = await walkAtTurn(session);
    // Each checkout the grouping of the walk's changes put the change to later/a/b/x.py under.
    const groupedAgain = new Set(session.grouped.filter(([path]) => path === "later/a/b/x.py").map(([, owner]) => owner));
    const plainAgain = await handedTo(session, view, ["later/a/b/x.py"]);

    expect(plain).toEqual([["later/a/b/x.py", repo, repo]]);
    expect(checkout).toEqual([["later/a/b/x.py", later, later]]);
    expect(plainAgain).toEqual([["later/a/b/x.py", repo, repo]]);
    expect([...groupedAgain]).toEqual([repo]);
  }, 60_000);

  it("is the full lookup's in a recorded checkout outside Git inside another", async () => {
    const base = await temporaryRoot();
    const outer = join(base, "outer");
    const inner = join(outer, "inner");
    for (const folder of [join(outer, "a"), join(inner, "a")]) {
      await mkdir(folder, { recursive: true });
      await writeFile(join(folder, "x.py"), "x = 1\n");
    }
    const databasePath = join(base, "rerail.sqlite");
    setUpTestRepository(inner, { library: [{ kind: "code", rule: NEEDLE_RULE }] }, databasePath);
    const session = await routedSession(outer, databasePath, outer, [outer, inner]);
    await session.bridge.openWorkspace("session");
    await session.bridge.settleWalk();

    const view = await walkAtTurn(session);

    expect(await handedTo(session, view, ["a/x.py", "inner/a/x.py"])).toEqual([["a/x.py", outer, outer], ["inner/a/x.py", inner, inner]]);
  }, 60_000);

  it("is the full lookup's below a .git Git does not take for a top level, and below one spelled .GIT", async () => {
    const base = await temporaryRoot();
    const repo = join(base, "repo");
    await smallCheckout(repo);
    // A .git file naming a Git folder that is gone.
    await mkdir(join(repo, "stray", "c"), { recursive: true });
    await writeFile(join(repo, "stray", ".git"), `gitdir: ${join(base, "gone")}\n`);
    await writeFile(join(repo, "stray", "c", "x.py"), "x = 1\n");
    // A worktree whose .git file is spelled .GIT: Windows and macOS find it as .git, and Git with them; Linux finds none.
    const upper = join(repo, "upper");
    git(repo, "worktree", "add", "--quiet", "--detach", upper);
    await rename(join(upper, ".git"), join(upper, ".GIT"));
    const session = await routedSession(repo, join(base, "rerail.sqlite"));
    await session.bridge.openWorkspace("session");
    await session.bridge.settleWalk();

    const view = await walkAtTurn(session);

    const handed = await handedTo(session, view, ["stray/c/x.py", "upper/a/b/x.py"]);
    expect(handed.map(([path, , full]) => [path, full, full])).toEqual(handed);
    expect(handed[1]![2]).toBe(process.platform === "linux" ? repo : upper);
  }, 60_000);
});
