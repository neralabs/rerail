import { execFileSync } from "node:child_process";
import { mkdirSync, realpathSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

/** Paths looked at (`lstat`) and processes started while `counting` is on. */
const reads = vi.hoisted(() => ({ counting: false, starts: 0, looked: [] as string[] }));
vi.mock("node:fs", async importOriginal => {
  const original = await importOriginal<typeof import("node:fs")>();
  return {
    ...original,
    lstatSync: ((...parameters: Parameters<typeof original.lstatSync>) => {
      if (reads.counting) reads.looked.push(String(parameters[0]));
      return original.lstatSync(...parameters);
    }) as typeof original.lstatSync,
  };
});

/**
 * How many of the next Gits asked where a folder's repository is (`rev-parse --show-toplevel ...`) end as a Git that
 * could not start ends, with no answer (on Linux the loader's exit 127).
 */
const placements = vi.hoisted(() => ({ unanswered: 0 }));
vi.mock("../../src/engine/child-process.js", async () => {
  const original = await vi.importActual<typeof import("../../src/engine/child-process.js")>("../../src/engine/child-process.js");
  const spawn = ((...parameters: Parameters<typeof original.spawn>) => {
    if (reads.counting) reads.starts += 1;
    return original.spawn(...parameters);
  }) as typeof original.spawn;
  const spawnSync = ((...parameters: Parameters<typeof original.spawnSync>) => {
    if (reads.counting) reads.starts += 1;
    const args = parameters[1];
    if (placements.unanswered === 0 || !Array.isArray(args) || !args.includes("--show-toplevel")) return original.spawnSync(...parameters);
    placements.unanswered -= 1;
    return original.spawnSync(process.execPath, ["-e", "process.exit(127)"], { encoding: "utf8" });
  }) as typeof original.spawnSync;
  return { ...original, spawn, spawnSync };
});

import { callerFromHeader } from "../../src/api/mcp.js";
import { openRerailDatabase, recordCheckout } from "../../src/engine/index.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  placements.unanswered = 0;
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@t", ...args], { cwd, stdio: "ignore" });
}

/** A Git repository at `path` with one commit and `origin` at `remote`. */
function repositoryAt(path: string, remote: string): string {
  mkdirSync(path, { recursive: true });
  git(path, "init", "-q", "-b", "main");
  git(path, "remote", "add", "origin", remote);
  git(path, "commit", "-q", "--allow-empty", "-m", "first");
  return path;
}

/** A recorded checkout in a new folder, as a started repository's main checkout is, with its clone id in its `.git`. */
async function recordedCheckout(): Promise<string> {
  const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-mcp-caller-kept-")));
  cleanups.push(() => removeFolder(root));
  const app = repositoryAt(join(root, "app"), "git@github.com:acme/app.git");
  const database = openRerailDatabase(join(root, "rerail.sqlite"));
  try {
    recordCheckout(database, app);
  } finally {
    database.$client.close();
  }
  return app;
}

/** The checkout and folder a bridge started in `folder` is resolved to, or the reason its header is refused. */
function resolved(folder: string): { checkout: string | undefined; folder: string } | string {
  const caller = callerFromHeader(encodeURIComponent(folder));
  if (typeof caller === "string") return caller;
  return caller.kind === "agent" ? { checkout: caller.checkout, folder: caller.folder } : "a person";
}

/** {@link resolved}, with the processes it started and the `.git` entries it looked at. */
function counted(folder: string) {
  Object.assign(reads, { starts: 0, looked: [], counting: true });
  try {
    const answer = resolved(folder);
    return { answer, starts: reads.starts, gitEntries: reads.looked.filter(path => basename(path) === ".git").length };
  } finally {
    reads.counting = false;
  }
}

const LEVELS = Array.from({ length: 10 }, (_, index) => `level${index + 1}`);

describe("the checkout kept for an MCP bridge's folder", () => {
  it("serves an unchanged folder at one `.git` lstat per folder below its top level, and starts no process", async () => {
    const app = await recordedCheckout();
    for (const depth of [1, 3, 10]) {
      const folder = join(app, ...LEVELS.slice(0, depth));
      mkdirSync(folder, { recursive: true });
      const answer = { checkout: app, folder: LEVELS.slice(0, depth).join("/") };
      expect(resolved(folder), folder).toEqual(answer);
      expect(counted(folder), folder).toEqual({ answer, starts: 0, gitEntries: depth });
    }
  });

  it("reads a folder made a repository of its own once as a folder never asked about is read, then keeps it", async () => {
    const app = await recordedCheckout();
    for (const depth of [1, 3, 10]) {
      const below = [`asked-${depth}`, ...LEVELS.slice(0, depth - 1)];
      const folder = join(app, ...below);
      const neverAsked = join(app, `fresh-${depth}`, ...LEVELS.slice(0, depth - 1));
      mkdirSync(folder, { recursive: true });
      expect(resolved(folder), folder).toEqual({ checkout: app, folder: below.join("/") });

      repositoryAt(folder, "https://github.com/acme/tools.git");
      repositoryAt(neverAsked, "https://github.com/acme/tools.git");

      const fresh = counted(neverAsked);
      expect(fresh.answer, neverAsked).toEqual({ checkout: neverAsked, folder: "" });
      // One lstat more: the check that finds the folder's new `.git`.
      expect(counted(folder), folder).toEqual({ answer: { checkout: folder, folder: "" }, starts: fresh.starts, gitEntries: fresh.gitEntries + 1 });
      expect(counted(folder).starts, folder).toBe(0);
    }
  });

  it("asks Git again at the next request after Git gave no answer to where the folder's repository is", async () => {
    const app = await recordedCheckout();
    const folder = join(app, "web");
    mkdirSync(folder);
    placements.unanswered = 1;
    // Read as outside Git for that ask alone: its own top level.
    expect(resolved(folder)).toEqual({ checkout: folder, folder: "" });
    expect(placements.unanswered).toBe(0);

    expect(resolved(folder)).toEqual({ checkout: app, folder: "web" });
  });
});
