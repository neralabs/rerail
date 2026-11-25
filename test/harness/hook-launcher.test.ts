import { spawnSync } from "node:child_process";
import { chmod, copyFile, mkdir, mkdtemp, readFile, realpath, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { removeFolder } from "../support/platform/remove-folder.js";

const LAUNCHER = resolve(import.meta.dirname, "../../src/harness/hook-launcher.ts");
// The launcher runs only on Windows, so its tests run there too, where a `#!/bin/sh` wrapper cannot stand in for a
// node: the recorded node is a copy of this one, told apart by its file name. Elsewhere it is a wrapper that sets
// RECORDED, since a copy of node need not start outside its install there (Homebrew's loads `@rpath/libnode`).
const WINDOWS = process.platform === "win32";
const RECORDED_NODE = WINDOWS ? "recorded-node.exe" : "recorded-node";
const RAN_RECORDED = WINDOWS ? RECORDED_NODE : "recorded";
const RAN_PATH_NODE = basename(process.execPath);

// Reports which node ran it, its argv[1], and the event on stdin, then exits non-zero like a failing hook.
const REPORTING_HOOK = [
  'import { readFileSync } from "node:fs";',
  'import { basename } from "node:path";',
  "process.stdout.write(JSON.stringify({ node: process.env.RECORDED ?? basename(process.execPath), argv1: process.argv[1], stdin: readFileSync(0, \"utf8\") }));",
  "process.exitCode = 3;",
].join("\n");

// Fails while loading, before it could answer: a missing module after an update, or a binding built for another node.
const CRASHING_HOOK = 'throw new Error("Cannot find module rerail-binding");';

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

type Recorded = "runs" | "missing" | "cannot-run" | "itself";

async function launch(recorded: Recorded, hookSource = REPORTING_HOOK): Promise<{ status: number | null; stdout: string; stderr: string }> {
  const base = await mkdtemp(join(tmpdir(), "rerail-hook-launcher-"));
  temporaryDirectories.push(base);
  const hook = join(base, "claude-hook.mjs");
  await writeFile(hook, hookSource);
  // A distinct "recorded node", so the test can tell it from the node that starts the launcher; or that node itself,
  // which runs the hook in the launcher's own process.
  const node = recorded === "itself" ? process.execPath : join(base, RECORDED_NODE);
  if (recorded === "runs") await recordedNode(node);
  // A file that is no program: without the execute bit, or on Windows (which has none) not an executable at all.
  if (recorded === "cannot-run") await (WINDOWS ? writeFile(node, "not a program\n") : writeFile(node, "#!/bin/sh\n", { mode: 0o644 }));
  const result = spawnSync(process.execPath, ["--import", "tsx", LAUNCHER, node, hook], { input: '{"hook_event_name":"Stop"}', encoding: "utf8" });
  return { status: result.status, stdout: result.stdout, stderr: result.stderr };
}

async function recordedNode(path: string): Promise<void> {
  if (WINDOWS) {
    await copyFile(process.execPath, path);
    return;
  }
  // Its node is not this file, so a launcher that started itself under it without end would fill the machine: a wrapper
  // started under another one ends at once.
  await writeFile(path, `#!/bin/sh\n[ -z "$RECORDED" ] || exit 97\nRECORDED=recorded exec "${process.execPath}" "$@"\n`);
  await chmod(path, 0o755);
}

describe("hook launcher started by the recorded node", () => {
  it("runs the hook in its own process, so a hook event starts one node", async () => {
    const base = await mkdtemp(join(tmpdir(), "rerail-hook-launcher-"));
    temporaryDirectories.push(base);
    const hook = join(base, "claude-hook.mjs");
    await writeFile(hook, "process.stdout.write(JSON.stringify({ pid: process.pid, argv1: process.argv[1] }));");
    // Set-up writes the recorded node as the command where `node` on PATH is another one.
    const result = spawnSync(process.execPath, ["--import", "tsx", LAUNCHER, process.execPath, hook], { input: "{}", encoding: "utf8" });
    expect(JSON.parse(result.stdout), result.stderr).toEqual({ pid: result.pid, argv1: hook });
    expect(result.status).toBe(0);
  }, 30_000);

  // A launcher started under another node starts itself under the recorded node with `--here` in that node's place. The
  // word itself, not the lack of a file of that name, makes the second launcher run the hook, so it never starts a
  // third, even from a project folder that holds a file named `--here`.
  it("runs the hook in its own process when another launcher started it, even beside a file named --here", async () => {
    const base = await mkdtemp(join(tmpdir(), "rerail-hook-launcher-"));
    temporaryDirectories.push(base);
    const hook = join(base, "claude-hook.mjs");
    await writeFile(hook, "process.stdout.write(JSON.stringify({ pid: process.pid, argv1: process.argv[1] }));");
    await writeFile(join(base, "--here"), "not a program\n");
    // From that folder, as Claude Code runs a hook from the project's, so tsx comes by its path.
    const tsx = pathToFileURL(createRequire(import.meta.url).resolve("tsx")).href;
    const result = spawnSync(process.execPath, ["--import", tsx, LAUNCHER, "--here", hook], { cwd: base, input: "{}", encoding: "utf8" });
    expect(JSON.parse(result.stdout), result.stderr).toEqual({ pid: result.pid, argv1: hook });
    expect(result.status).toBe(0);
  }, 30_000);

  const ANSWER = '{"continue":true}';
  it.each([
    ["ends the process before it answers", "process.exit(3);", "{}\n"],
    ["throws once it has loaded, before it answers", 'setTimeout(() => { throw new Error("late failure"); }, 0);', "{}\n"],
    ["leaves a failing exit code and answers nothing", "process.exitCode = 3;", "{}\n"],
    ["answers, then ends the process", `process.stdout.write(${JSON.stringify(ANSWER)}); process.exit(3);`, ANSWER],
    ["answers, then fails while loading", `process.stdout.write(${JSON.stringify(ANSWER)}); throw new Error("late failure");`, ANSWER],
  ])("answers for a hook that %s as for one it runs as a child: exit 0, and {} only when it wrote nothing", async (_case, source, answer) => {
    const child = await launch("runs", source);
    expect(child, child.stderr).toMatchObject({ status: 0, stdout: answer });
    const ownProcess = await launch("itself", source);
    expect(ownProcess, ownProcess.stderr).toMatchObject({ status: 0, stdout: answer });
    expect(ownProcess.stderr).toContain("rerail hook:");
  }, 30_000);
});

describe("hook launcher", () => {
  it("runs the hook under the recorded node while that binary exists, passing its answer through", async () => {
    const { status, stdout, stderr } = await launch("runs");
    expect(JSON.parse(stdout), stderr).toMatchObject({ node: RAN_RECORDED, stdin: '{"hook_event_name":"Stop"}' });
    // A failed hook would show as an error under every tool call, so the launcher never fails.
    expect(status).toBe(0);
    expect(stderr).toContain("rerail hook: exited 3");
  });

  it("falls back to the node that started it once the recorded node is gone", async () => {
    const { status, stdout, stderr } = await launch("missing");
    // The hook serves only when it is argv[1], as if `node <hook>` had started it.
    expect(JSON.parse(stdout), stderr).toEqual({ node: RAN_PATH_NODE, argv1: expect.stringMatching(/claude-hook\.mjs$/u), stdin: '{"hook_event_name":"Stop"}' });
    expect(status).toBe(0);
  });

  it.each<Recorded>(["runs", "missing"])("answers {} and exits 0 for a hook that fails while loading (recorded node %s)", async recorded => {
    const { status, stdout, stderr } = await launch(recorded, CRASHING_HOOK);
    expect(stdout).toBe("{}\n");
    expect(status).toBe(0);
    expect(stderr).toContain("rerail hook:");
  });

  // Under another node the hook runs in a second launcher, which a native crash or an out-of-memory abort ends before it
  // can answer: the first answers for it.
  it("answers {} and exits 0 for a hook that ends the second launcher before its exit handlers run", async () => {
    const { status, stdout, stderr } = await launch("runs", 'process.kill(process.pid, "SIGKILL");');
    expect({ status, stdout }, stderr).toEqual({ status: 0, stdout: "{}\n" });
  });

  it("answers {} and exits 0 when the recorded node cannot be started", async () => {
    const { status, stdout, stderr } = await launch("cannot-run");
    expect(stdout).toBe("{}\n");
    expect(status).toBe(0);
    // The failure names the program it could not start. Its code is libuv's: EACCES without the execute bit; on Windows,
    // a file it cannot load as a program, whose code Node reports as UNKNOWN.
    expect(stderr).toMatch(WINDOWS ? /rerail hook: spawnSync .*recorded-node\.exe \w+/u : /rerail hook: .*EACCES/u);
  });
});

// Reports the folder of Node's compile cache in the process that runs it, or null where the cache is off, and the
// NODE_COMPILE_CACHE its environment holds.
const CACHE_REPORTING_HOOK = [
  'import { getCompileCacheDir } from "node:module";',
  "process.stdout.write(JSON.stringify({ cache: getCompileCacheDir() ?? null, named: process.env.NODE_COMPILE_CACHE ?? null }));",
].join("\n");

interface CachePlace {
  base: string;
  node: string;
  hook: string;
}

/** A folder for one test: the cache reporting hook, the recorded node and a temporary folder (`tmp`) of its own. */
async function cachePlace(recorded: "runs" | "itself"): Promise<CachePlace> {
  const base = await mkdtemp(join(tmpdir(), "rerail-hook-launcher-"));
  temporaryDirectories.push(base);
  const hook = join(base, "claude-hook.mjs");
  await writeFile(hook, CACHE_REPORTING_HOOK);
  const node = recorded === "itself" ? process.execPath : join(base, RECORDED_NODE);
  if (recorded === "runs") await recordedNode(node);
  await mkdir(join(base, "tmp"));
  return { base, node, hook };
}

/** The launcher's exit status, the answer it passed on (`{}` where it wrote none), that answer's fields and its stderr. */
interface Launched {
  status: number | null;
  answer: { cache?: string | null; named?: string | null };
  cache: string | null;
  named: string | null;
  stderr: string;
}

/**
 * The launcher started as Claude Code starts a hook, with the place's temporary folder (where Node keeps its compile
 * cache by default) and none of the caller's compile cache settings; `env` adds to that environment, and `preload` is a
 * module the node that starts the launcher loads first.
 */
function launchAt(place: CachePlace, env: NodeJS.ProcessEnv = {}, preload?: string): Launched {
  const temp = join(place.base, "tmp");
  const { NODE_COMPILE_CACHE: _cache, NODE_DISABLE_COMPILE_CACHE: _off, ...caller } = process.env;
  const imports = preload === undefined ? [] : ["--import", pathToFileURL(preload).href];
  const result = spawnSync(process.execPath, ["--import", "tsx", ...imports, LAUNCHER, place.node, place.hook], {
    input: "{}",
    encoding: "utf8",
    env: { ...caller, TMPDIR: temp, TMP: temp, TEMP: temp, ...env },
  });
  const answer = JSON.parse(result.stdout || "{}") as Launched["answer"];
  return { status: result.status, answer, cache: answer.cache ?? null, named: answer.named ?? null, stderr: result.stderr };
}

async function launchForCache(recorded: "runs" | "itself", env: NodeJS.ProcessEnv = {}): Promise<Launched & { base: string }> {
  const place = await cachePlace(recorded);
  return { ...launchAt(place, env), base: place.base };
}

describe("hook launcher and Node's compile cache", () => {
  // Every hook event starts a Node that loads the hook's bundle; with the cache on, V8 reads the code it compiled at an
  // earlier event instead of parsing and compiling the bundle again.
  // Node keeps each version's code in a folder of its own inside the cache folder, so the hook under either node finds
  // the cache folder itself, never a folder inside another node's.
  it.each<"runs" | "itself">(["itself", "runs"])("runs the hook with Node's compile cache on, in its default folder (recorded node %s)", async recorded => {
    const { cache, base, stderr } = await launchForCache(recorded);
    expect(cache, stderr).not.toBeNull();
    expect(dirname(await realpath(cache ?? ""))).toBe(await realpath(join(base, "tmp", "node-compile-cache")));
  }, 30_000);

  it.each<"runs" | "itself">(["itself", "runs"])("keeps the cache in the folder the person named in NODE_COMPILE_CACHE (recorded node %s)", async recorded => {
    const place = await cachePlace(recorded);
    const named = join(place.base, "named-cache");
    const { cache, stderr } = launchAt(place, { NODE_COMPILE_CACHE: named });
    expect(cache, stderr).not.toBeNull();
    expect(dirname(await realpath(cache ?? ""))).toBe(await realpath(named));
  }, 30_000);

  it.each<"runs" | "itself">(["itself", "runs"])("leaves the cache off where the person turned it off (recorded node %s)", async recorded => {
    const { status, answer, stderr } = await launchForCache(recorded, { NODE_DISABLE_COMPILE_CACHE: "1" });
    // The hook's own answer: one the launcher wrote for a hook that never ran holds no `cache`.
    expect(status, stderr).toBe(0);
    expect(answer).toHaveProperty("cache", null);
  }, 30_000);

  // A daemon the hook starts, and through it the person's own hook commands, take the hook's environment.
  it.each<"runs" | "itself">(["itself", "runs"])("gives the hook no NODE_COMPILE_CACHE the person did not set (recorded node %s)", async recorded => {
    const { cache, named, stderr } = await launchForCache(recorded);
    expect(cache, stderr).not.toBeNull();
    expect(named).toBeNull();
  }, 30_000);
});

/**
 * Stands in for a node on PATH without `module.enableCompileCache` (before Node 22.8), while the recorded node keeps
 * it. It writes a line to `LAUNCHER_STARTS` for each node that starts a launcher, and ends the third at once, so a
 * launcher that started itself without end shows in the count instead of filling the machine.
 */
const NODE_WITHOUT_CACHE = [
  'import { appendFileSync, readFileSync } from "node:fs";',
  'import nodeModule, { syncBuiltinESMExports } from "node:module";',
  'import { basename } from "node:path";',
  "const starts = process.env.LAUNCHER_STARTS;",
  'appendFileSync(starts, "started\\n");',
  'if (readFileSync(starts, "utf8").split("\\n").length > 3) process.exit(0);',
  `if ((process.env.RECORDED ?? basename(process.execPath)) !== ${JSON.stringify(RAN_RECORDED)}) {`,
  "  delete nodeModule.enableCompileCache;",
  "  syncBuiltinESMExports();",
  "}",
].join("\n");

describe("hook launcher started by a node without Node's compile cache", () => {
  async function olderNodePlace(): Promise<{ place: CachePlace; preload: string }> {
    const place = await cachePlace("runs");
    const preload = join(place.base, "node-without-cache.mjs");
    await writeFile(preload, NODE_WITHOUT_CACHE);
    return { place, preload };
  }

  it("runs the hook with the cache on under the recorded node, which reads the code it kept at the next call", async () => {
    const { place, preload } = await olderNodePlace();
    // Node's own debug lines say whether V8 took the code it read from the cache.
    const env = { NODE_DEBUG_NATIVE: "COMPILE_CACHE" };
    launchAt(place, { ...env, LAUNCHER_STARTS: join(place.base, "first-starts") }, preload);
    const second = launchAt(place, { ...env, LAUNCHER_STARTS: join(place.base, "second-starts") }, preload);
    expect(second.cache, second.stderr).not.toBeNull();
    expect(second.stderr).toMatch(/\[compile cache\] V8 code cache for \S+ \S*claude-hook\.mjs was accepted/u);
  }, 60_000);

  // A node without it that runs the hook itself: the recorded node, or the node the launcher falls back to once that is
  // gone.
  it("runs the hook without the cache where the node that runs it has none", async () => {
    const place = await cachePlace("itself");
    const preload = join(place.base, "no-compile-cache.mjs");
    await writeFile(preload, 'import nodeModule, { syncBuiltinESMExports } from "node:module";\ndelete nodeModule.enableCompileCache;\nsyncBuiltinESMExports();\n');
    const { status, answer, stderr } = launchAt(place, {}, preload);
    expect(status, stderr).toBe(0);
    expect(answer).toHaveProperty("cache", null);
  }, 30_000);

  it("starts a launcher under the recorded node once, and that one runs the hook itself", async () => {
    const { place, preload } = await olderNodePlace();
    const starts = join(place.base, "starts");
    const { cache, stderr } = launchAt(place, { LAUNCHER_STARTS: starts }, preload);
    expect(cache, stderr).not.toBeNull();
    expect(await readFile(starts, "utf8")).toBe("started\nstarted\n");
  }, 30_000);
});
