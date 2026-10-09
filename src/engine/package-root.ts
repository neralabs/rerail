import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const RERAIL_PACKAGE_NAME = "@neralabs/rerail";

function nearestPackageRoot(from: string): string {
  for (let directory = from; ; directory = dirname(directory)) {
    if (existsSync(join(directory, "package.json"))) return directory;
    if (dirname(directory) === directory) throw new Error(`no package.json above ${from}`);
  }
}

/**
 * This package's root: the nearest directory above this module holding `package.json`. Modules sit at
 * different depths below it (`src/<layer>/…`, the compiled `dist/<layer>/…`, and the esbuild bundles that
 * inline this module), so nothing counts `..` from its own file.
 */
export const PACKAGE_ROOT = nearestPackageRoot(dirname(fileURLToPath(import.meta.url)));

/**
 * The built entry points the package starts, relative to its root: they mirror `src/<layer>/…`, and `package.json`
 * (`bin`, `exports`, the `omp`/`pi` extensions), `hooks/*.json` and every harness registration name the same files.
 */
export const ENTRY_POINTS = {
  index: "dist/index.js",
  cli: "dist/cli/cli.js",
  daemon: "dist/daemon/daemon.js",
  eventStream: "dist/harness/event-stream.js",
  hookLauncher: "dist/harness/hook-launcher.js",
  repositoryStart: "dist/runtime/repository-start.js",
  claudeHook: "dist/hosts/claude/claude-hook.js",
  claudeStatus: "dist/hosts/claude/claude-status.js",
  codexHook: "dist/hosts/codex/codex-hook.js",
  cursorHook: "dist/hosts/cursor/cursor-hook.js",
} as const;

/** `<package>/model`: the bundled embedding model. */
export function bundledModelPath(): string {
  return resolve(PACKAGE_ROOT, "model");
}
