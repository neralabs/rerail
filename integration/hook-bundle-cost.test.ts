import { execFile } from "node:child_process";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";
import { describe, expect, test } from "vitest";

const run = promisify(execFile);

/**
 * Imports the bundle `HOOK_BUNDLE` names in a fresh node, then prints the npm packages node resolved while loading it
 * and the process's peak working set (`maxRSS`, PeakWorkingSetSize on Windows). The bundle is not argv[1], so it loads
 * without serving a hook.
 */
const PROBE = String.raw`
import { registerHooks } from "node:module";
const packages = new Set();
registerHooks({ resolve(specifier, context, next) {
  const resolved = next(specifier, context);
  const name = /\/node_modules\/((?:@[^/]+\/)?[^/]+)\//u.exec(resolved.url)?.[1];
  if (name !== undefined) packages.add(name);
  return resolved;
} });
await import(process.env.HOOK_BUNDLE);
console.log(JSON.stringify({ packages: [...packages].sort(), peakMB: Math.round(process.resourceUsage().maxRSS / 1024) }));
`;

async function importCost(bundle: string): Promise<{ packages: string[]; peakMB: number }> {
  const { stdout } = await run(process.execPath, ["--input-type=module", "-e", PROBE], {
    env: { ...process.env, HOOK_BUNDLE: pathToFileURL(resolve(bundle)).href },
  });
  return JSON.parse(stdout) as { packages: string[]; peakMB: number };
}

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
    const cost = await importCost(bundle);
    expect(cost.packages.filter(name => name !== "better-sqlite3")).toEqual([]);
    expect(cost.peakMB).toBeLessThan(150);
  });
});
