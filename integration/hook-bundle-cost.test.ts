import { execFile } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";
import { afterEach, describe, expect, test } from "vitest";

const run = promisify(execFile);

/**
 * Records the npm packages node resolves from here on, and defines `report()`, which prints them with the process's
 * peak working set (`maxRSS`, PeakWorkingSetSize on Windows).
 */
const RESOLVED_PACKAGES = String.raw`
import { registerHooks } from "node:module";
const packages = new Set();
registerHooks({ resolve(specifier, context, next) {
  const resolved = next(specifier, context);
  const name = /\/node_modules\/((?:@[^/]+\/)?[^/]+)\//u.exec(resolved.url)?.[1];
  if (name !== undefined) packages.add(name);
  return resolved;
} });
const report = () => console.log(JSON.stringify({ packages: [...packages].sort(), peakMB: Math.round(process.resourceUsage().maxRSS / 1024) }));
`;

/** Imports the bundle `HOOK_BUNDLE` names. It is not argv[1], so it loads without serving a hook. */
const IMPORT_PROBE = `${RESOLVED_PACKAGES}
await import(process.env.HOOK_BUNDLE);
report();
`;

/** One run of the status line `HOOK_BUNDLE` names, as Claude Code runs it, with the person's settings at `STATUS_SETTINGS`. */
const STATUS_PROBE = `${RESOLVED_PACKAGES}
const { Readable, Writable } = await import("node:stream");
const { runClaudeStatus } = await import(process.env.HOOK_BUNDLE);
const output = new Writable({ write(_chunk, _encoding, done) { done(); } });
await runClaudeStatus(Readable.from([JSON.stringify({ session_id: "s", cwd: process.cwd() })]), output, process.env.STATUS_SETTINGS);
report();
`;

interface Cost {
  packages: string[];
  peakMB: number;
}

async function probe(script: string, bundle: string, options: { cwd?: string; env?: NodeJS.ProcessEnv } = {}): Promise<Cost> {
  const { stdout } = await run(process.execPath, ["--input-type=module", "-e", script], {
    ...(options.cwd === undefined ? {} : { cwd: options.cwd }),
    env: { ...process.env, ...options.env, HOOK_BUNDLE: pathToFileURL(resolve(bundle)).href },
  });
  return JSON.parse(stdout) as Cost;
}

const homes: string[] = [];
afterEach(async () => {
  await Promise.all(homes.splice(0).map(home => rm(home, { recursive: true, force: true })));
});

/**
 * A coding agent starts a hook process for every event, often several at once, and an out-of-process host starts the
 * event stream for every session. Node 24 parses a package's whole `exports` map again for each module it loads from
 * that package, and drizzle-orm's is 179 KB: resolved at run time it took a hook to 285 MB and half a second before the
 * hook did anything. Bundled, it costs neither; better-sqlite3 alone stays outside, for its native binding.
 */
describe("hook bundles", () => {
  test.each([
    "dist/hosts/claude/claude-hook.js",
    "dist/hosts/codex/codex-hook.js",
    "dist/hosts/cursor/cursor-hook.js",
    "dist/harness/event-stream.js",
    "dist/hosts/omp/default-extension.js",
  ])("%s resolves no npm package at run time but better-sqlite3, and stays under 150 MB", { timeout: 30_000 }, async bundle => {
    const cost = await probe(IMPORT_PROBE, bundle);
    expect(cost.packages.filter(name => name !== "better-sqlite3")).toEqual([]);
    expect(cost.peakMB).toBeLessThan(150);
  });
});

/**
 * Claude Code runs the status line every few seconds in each session. When no daemon answers within 2 s (it is busy, or
 * down) the status line reads the checkout's start state itself: through the unbundled repository-start.js that read
 * resolved drizzle-orm at run time and peaked at 285 MB, in every session, for as long as the daemon stayed busy.
 */
describe("the Claude Code status line with no daemon answering", () => {
  test("reads the start state without resolving drizzle-orm, and stays under 150 MB", { timeout: 30_000 }, async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-status-cost-"));
    homes.push(home);
    const cost = await probe(STATUS_PROBE, "dist/hosts/claude/claude-status.js", {
      cwd: home,
      env: { HOME: home, USERPROFILE: home, RERAIL_HOME: join(home, ".rerail"), STATUS_SETTINGS: join(home, "settings.json") },
    });
    expect(cost.packages).not.toContain("drizzle-orm");
    expect(cost.peakMB).toBeLessThan(150);
  });
});
