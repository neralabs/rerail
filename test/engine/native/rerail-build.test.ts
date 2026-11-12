import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
  contentStorePath,
  contentStoreProblem,
  currentRerailBuild,
  isMainRerailBuild,
  mainContentStoreDirectory,
  resolveRerailBuild,
} from "../../../src/engine/native/rerail-build.js";

/** This binary's store file (`STORE_FILE_NAME`, raised with the store schema floor). */
const STORE_FILE = "content-v1.sqlite";
const MAIN_STORE = join(homedir(), ".cache", "rerail", STORE_FILE);

let root: string;
let storeOverride: string | undefined;

beforeEach(() => {
  root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-build-")));
  // The vitest setup points every file at its own store; these tests check the defaults behind that override.
  storeOverride = process.env.RERAIL_CONTENT_STORE_PATH;
  delete process.env.RERAIL_CONTENT_STORE_PATH;
});

afterEach(() => {
  if (storeOverride === undefined) delete process.env.RERAIL_CONTENT_STORE_PATH;
  else process.env.RERAIL_CONTENT_STORE_PATH = storeOverride;
  rmSync(root, { recursive: true, force: true });
});

function managedHome(version: string): { home: string; packageRoot: string } {
  const home = join(root, "rerail-home");
  const packageRoot = join(home, "versions", version, "node_modules", "@neralabs", "rerail");
  mkdirSync(packageRoot, { recursive: true });
  symlinkSync(packageRoot, join(home, "current"), "junction");
  return { home, packageRoot };
}

describe("resolveRerailBuild", () => {
  test("the managed installation loaded through `current` is main and owns the machine-wide store", () => {
    const { home, packageRoot } = managedHome("1.4.0");

    const build = resolveRerailBuild(join(home, "current"), { RERAIL_HOME: home });

    expect(build).toEqual({ kind: "main", home, version: "1.4.0", packageRoot });
    expect(contentStorePath(build)).toBe(MAIN_STORE);
    expect(contentStorePath(build)).toBe(join(mainContentStoreDirectory(), STORE_FILE));
  });

  test("a version under another Rerail home is not main", () => {
    const { packageRoot } = managedHome("1.4.0");

    expect(() => resolveRerailBuild(packageRoot, { RERAIL_HOME: join(root, "health-check-home") })).toThrow(
      "neither the managed installation",
    );
  });

  test("an unvalidated staging install is not main", () => {
    const home = join(root, "rerail-home");
    const staging = join(home, "versions", ".1.4.0.0b6f.staging", "node_modules", "@neralabs", "rerail");
    mkdirSync(staging, { recursive: true });

    expect(() => resolveRerailBuild(staging, { RERAIL_HOME: home })).toThrow("neither the managed installation");
  });

  test("a checkout build is dev with a store private to that checkout", () => {
    const checkout = join(root, "checkout");
    mkdirSync(join(checkout, ".git"), { recursive: true });
    mkdirSync(join(checkout, "rerail"));

    const build = resolveRerailBuild(join(checkout, "rerail"), { RERAIL_HOME: join(root, "rerail-home") });

    expect(build).toEqual({ kind: "dev", packageRoot: join(checkout, "rerail"), checkout });
    expect(contentStorePath(build)).toBe(join(checkout, ".rerail", STORE_FILE));
  });

  test("each worktree, marked by a `.git` file, gets its own store", () => {
    const stores = ["first", "second"].map((name) => {
      const worktree = join(root, name);
      mkdirSync(join(worktree, "rerail"), { recursive: true });
      writeFileSync(join(worktree, ".git"), `gitdir: ${join(root, "common", "worktrees", name)}\n`);
      const build = resolveRerailBuild(join(worktree, "rerail"), {});
      expect(build).toEqual({ kind: "dev", packageRoot: join(worktree, "rerail"), checkout: worktree });
      return contentStorePath(build);
    });

    expect(stores).toEqual([
      join(root, "first", ".rerail", STORE_FILE),
      join(root, "second", ".rerail", STORE_FILE),
    ]);
  });

  test("RERAIL_CONTENT_STORE_PATH overrides the store for main and dev alike", () => {
    const { home, packageRoot } = managedHome("1.4.0");
    const checkout = join(root, "checkout");
    mkdirSync(join(checkout, ".git"), { recursive: true });
    process.env.RERAIL_CONTENT_STORE_PATH = join(root, "override.sqlite");

    expect(contentStorePath(resolveRerailBuild(packageRoot, { RERAIL_HOME: home }))).toBe(join(root, "override.sqlite"));
    expect(contentStorePath(resolveRerailBuild(checkout, {}))).toBe(join(root, "override.sqlite"));
  });
});

describe("a running package with no content store", () => {
  /** This module as a process running from `packageRoot` loads it: its own `currentRerailBuild` cache. */
  async function runningFrom(packageRoot: string): Promise<typeof import("../../../src/engine/native/rerail-build.js")> {
    vi.resetModules();
    vi.doMock("../../../src/engine/package-root.js", async (original) => ({ ...(await original<object>()), PACKAGE_ROOT: packageRoot }));
    return await import("../../../src/engine/native/rerail-build.js");
  }

  afterEach(() => {
    vi.doUnmock("../../../src/engine/package-root.js");
    vi.resetModules();
  });

  test("fails the same way on every call: the failure is resolved once per process, like a build", async () => {
    const packageRoot = join(root, "install", "node_modules", "@neralabs", "rerail");
    mkdirSync(packageRoot, { recursive: true });
    const running = await runningFrom(packageRoot);

    expect(() => running.currentRerailBuild()).toThrow("so it has no content store");
    // Resolving again would now find a checkout; the process keeps the answer it started with.
    mkdirSync(join(root, "install", ".git"));
    expect(() => running.currentRerailBuild()).toThrow("so it has no content store");
  });

  test("says why, for the daemon to report, without throwing; the store override gives it one", async () => {
    const packageRoot = join(root, "install", "node_modules", "@neralabs", "rerail");
    mkdirSync(packageRoot, { recursive: true });
    const running = await runningFrom(packageRoot);

    expect(running.contentStoreProblem()).toBe(
      `Rerail at ${packageRoot} is neither the managed installation under ` +
        `${join(resolve(process.env.RERAIL_HOME!), "versions")} nor in a Git checkout, so it has no content store`,
    );
    process.env.RERAIL_CONTENT_STORE_PATH = join(root, "override.sqlite");
    expect(running.contentStoreProblem()).toBeUndefined();
  });

  test("a build that has a store has no problem to report", () => {
    expect(contentStoreProblem()).toBeUndefined();
  });
});

describe("currentRerailBuild", () => {
  test("this checkout's tests are a dev build that never opens the machine-wide store", () => {
    const packageRoot = realpathSync(join(dirname(fileURLToPath(import.meta.url)), "..", "..", ".."));
    const checkout = realpathSync(
      execFileSync("git", ["rev-parse", "--show-toplevel"], { cwd: packageRoot, encoding: "utf8" }).trim(),
    );

    expect(currentRerailBuild()).toEqual({ kind: "dev", packageRoot, checkout });
    expect(isMainRerailBuild()).toBe(false);
    expect(contentStorePath()).toBe(join(checkout, ".rerail", STORE_FILE));
    expect(contentStorePath()).not.toBe(MAIN_STORE);
  });
});
