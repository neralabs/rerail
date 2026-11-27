/**
 * What a walk of the session folder works out, on the daemon's thread, to name each file it finds as the auditor names
 * it (relative to the session folder, `/`-separated): each `path.relative` naming a file or a folder of a checkout of
 * 200 folders, over the walk at a turn's end, and the paths the auditor is asked about. Then each path the walk hands
 * on, compared with what `path.relative` from the walk's root makes of the same path, for each way of naming the
 * session folder.
 */
import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import type * as Path from "node:path";
import { join, relative, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

/** While `on`: the path each `path.relative` was asked to reach. */
const arithmetic = vi.hoisted(() => ({ on: false, reached: [] as string[] }));
vi.mock("node:path", async importOriginal => {
  const actual = await importOriginal<typeof Path>();
  return {
    ...actual,
    relative: (from: string, to: string): string => {
      if (arithmetic.on) arithmetic.reached.push(to);
      return actual.relative(from, to);
    },
  };
});

import { CodeRuleBridge } from "../../../../src/engine/code-rule/code-rule-bridge.js";
import type { CodeRuleAuditor, CodeRuleEditBatch } from "../../../../src/engine/code-rule/code-rule.js";
import { linkFolder } from "../../../support/platform/links.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  arithmetic.on = false;
  vi.restoreAllMocks();
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

/** The tree's folders, d0 to d9 each holding e0 to e18: 200 folders, each holding one Python file. */
const FOLDERS = Array.from({ length: 10 }, (_, top) => [`d${top}`, ...Array.from({ length: 19 }, (_, leaf) => `d${top}/e${leaf}`)]).flat();
const FILES = FOLDERS.map(folder => `${folder}/x.py`);

function response(batch: CodeRuleEditBatch) {
  return { revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] };
}

async function temporaryRoot(): Promise<string> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-walk-session-paths-")));
  cleanups.push(() => removeFolder(root));
  return root;
}

describe("a walk of a checkout of many folders", () => {
  it("names each file it finds without working out its path from the session folder's again", async () => {
    const root = await temporaryRoot();
    for (const folder of FOLDERS) {
      await mkdir(join(root, folder), { recursive: true });
      await writeFile(join(root, folder, "x.py"), "x = 1\n");
    }
    git(root, "init", "--quiet", "-b", "main");
    git(root, "add", "-A");
    git(root, "commit", "--quiet", "-m", "seed");
    // Each path the walk asks Git's ignore question of: every file it finds, as the auditor names it.
    const asked: string[] = [];
    const auditor: CodeRuleAuditor = {
      openWorkspace: async () => undefined,
      acceptsPath: path => path.endsWith(".py"),
      ignoresPath: (path, directory) => {
        if (!directory) asked.push(path);
        return false;
      },
      auditGeneratedEdits: async (_sessionId, batch) => response(batch),
      close: async () => undefined,
    };
    const bridge = new CodeRuleBridge(root, auditor, { discoveryWaitMs: Infinity });
    cleanups.push(() => bridge.close());
    await bridge.openWorkspace("paths");
    await bridge.settleWalk();
    asked.length = 0;
    arithmetic.reached = [];

    arithmetic.on = true;
    try {
      await bridge.checkpoint("turn");
    } finally {
      arithmetic.on = false;
    }

    const files = new Set(FILES.map(path => join(root, path)));
    const folders = new Set(FOLDERS.map(path => join(root, path)));
    // `path.relative` from the session folder to each file it found: 200 a walk, each resolving both paths again.
    expect.soft(arithmetic.reached.filter(path => files.has(path)), "path.relative naming a file").toHaveLength(0);
    // And to each folder it lists, which its listing and Git's ignore question name.
    expect.soft(arithmetic.reached.filter(path => folders.has(path)), "path.relative naming a folder").toHaveLength(0);
    expect(asked.toSorted()).toEqual(FILES.toSorted());
  }, 60_000);
});

/** What the walk hands on, by where: the view's questions and listings, the grouping by checkout, the audit. */
interface Handed {
  /** The walk's root, as the walk asks its view for it (`walkView`). */
  readonly roots: string[];
  readonly listed: string[];
  readonly folders: string[];
  readonly files: string[];
  readonly accepted: string[];
  readonly grouped: string[];
  readonly lookedUp: string[];
  readonly audited: string[];
}

/** A file Git would ignore and a folder Git would ignore, as the auditor's view answers. */
const IGNORED = new Set(["skip.py", "built"]);

/** An auditor whose walk view records each path it is handed into `handed`, the checkout of every path being `checkout`. */
function recordingAuditor(handed: Handed, checkout: string): CodeRuleAuditor {
  return {
    openWorkspace: async () => undefined,
    acceptsPath: path => path.endsWith(".py"),
    ignoresPath: path => IGNORED.has(path),
    sourceRoot: path => {
      handed.lookedUp.push(path);
      return checkout;
    },
    walkView: root => {
      if (root !== undefined) handed.roots.push(root);
      return {
        acceptsPath: path => {
          handed.accepted.push(path);
          return path.endsWith(".py");
        },
        ignoresPath: (path, directory) => {
          (directory ? handed.folders : handed.files).push(path);
          return IGNORED.has(path);
        },
        sourceRoot: path => {
          handed.grouped.push(path);
          return checkout;
        },
        folderListed: path => {
          handed.listed.push(path);
        },
      };
    },
    auditGeneratedEdits: async (_sessionId, batch) => {
      handed.audited.push(...batch.edits.map(edit => edit.path));
      return response(batch);
    },
    close: async () => undefined,
  };
}

function nothingHanded(): Handed {
  return { roots: [], listed: [], folders: [], files: [], accepted: [], grouped: [], lookedUp: [], audited: [] };
}

/**
 * A committed checkout at `root` holding a file directly in it (`top.py`), one two folders down (`src/a/b.py`), a file
 * and a folder the view ignores (`skip.py`, `built/c.py`), and, made after the commit, a link to `src` the walk does not
 * follow (`linked`: a junction on Windows).
 */
async function comparedTree(root: string): Promise<void> {
  await mkdir(join(root, "src", "a"), { recursive: true });
  await mkdir(join(root, "built"));
  for (const path of [["top.py"], ["skip.py"], ["src", "a", "b.py"], ["built", "c.py"]]) await writeFile(join(root, ...path), "x = 1\n");
  git(root, "init", "--quiet", "-b", "main");
  git(root, "add", "-A");
  git(root, "commit", "--quiet", "-m", "seed");
  linkFolder(join(root, "src"), join(root, "linked"));
}

/**
 * The ways a session folder can be given: as it is, with a trailing separator, through a link to it (a junction on
 * Windows), and on Windows with its drive letter in lower and in upper case.
 */
function spellings(root: string, link: string): string[] {
  const given = [root, `${root}${sep}`, link];
  if (process.platform === "win32") given.push(`${root.charAt(0).toLowerCase()}${root.slice(1)}`, `${root.charAt(0).toUpperCase()}${root.slice(1)}`);
  return given;
}

/** What `main` named `absolute` by in a walk from `root`: `path.relative` from the root, `/`-separated. */
function relativeWay(root: string, absolute: string): string {
  return relative(root, absolute).split(sep).join("/");
}

/** What the walk at a turn's end hands on, for a session opened at `given`, after `top.py` changed and `src/a/new.py` came. */
async function handedAtTurn(root: string, given: string): Promise<Handed> {
  const handed = nothingHanded();
  const bridge = new CodeRuleBridge(given, recordingAuditor(handed, root), { discoveryWaitMs: Infinity });
  try {
    await bridge.openWorkspace("paths");
    await bridge.settleWalk();
    await writeFile(join(root, "top.py"), "x = 2\n");
    await writeFile(join(root, "src", "a", "new.py"), "y = 1\n");
    for (const kind of Object.values(handed)) kind.length = 0;
    await bridge.checkpoint("turn");
    return handed;
  } finally {
    await bridge.close();
    await writeFile(join(root, "top.py"), "x = 1\n");
    await rm(join(root, "src", "a", "new.py"), { force: true });
  }
}

describe("the path a walk hands on for each entry it finds", () => {
  it("is the one path.relative makes of it from the walk's root, however the session folder is given", async () => {
    const root = await temporaryRoot();
    await comparedTree(root);
    const elsewhere = await temporaryRoot();
    linkFolder(root, join(elsewhere, "session"));

    for (const given of spellings(root, join(elsewhere, "session"))) {
      const handed = await handedAtTurn(root, given);
      expect(handed.roots, given).toHaveLength(1);
      const start = handed.roots[0]!;
      const named = (...names: string[]): string => relativeWay(start, join(start, ...names));
      // Every path the walk could name, as `path.relative` makes it: the root, each entry and each entry below.
      const every = new Set(
        [[], [".git"], ["top.py"], ["skip.py"], ["src"], ["src", "a"], ["src", "a", "b.py"], ["src", "a", "new.py"], ["built"], ["linked"]].map(names => named(...names)),
      );
      const found = [named("top.py"), named("skip.py"), named("src", "a", "b.py"), named("src", "a", "new.py")];

      expect(handed.listed.toSorted(), given).toEqual([named(), named("src"), named("src", "a")].toSorted());
      expect(handed.folders.toSorted(), given).toEqual([named("src"), named("src", "a"), named("built")].toSorted());
      expect(handed.files.toSorted(), given).toEqual(found.toSorted());
      expect(handed.grouped, given).toEqual(expect.arrayContaining([named("top.py"), named("src", "a", "new.py")]));
      for (const path of [...handed.accepted, ...handed.grouped, ...handed.lookedUp, ...handed.audited]) expect(every, `${given}: ${path}`).toContain(path);
    }
  }, 120_000);
});
