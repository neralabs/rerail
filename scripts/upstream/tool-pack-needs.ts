/**
 * What a tool-pack's derivation needs on the machine beyond Node, read from the pack's own data rather than its name, so a
 * new pack that needs it is covered: `node --import tsx scripts/upstream/tool-pack-needs.ts --pack <pack> --tool <tool>`
 * prints `php=true` or `php=false`, for a step's `$GITHUB_OUTPUT` (upstream-bump.yml).
 *
 * A tool-pack needs PHP when the runtime its gates.yaml names (`upstream.runtime`) pins one (`.php-version`): Composer
 * installs no interpreter, and a bump's other stale tool-packs run in the runtime it changed, so they need the same.
 */
import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

import { readGatesManifest } from "../../conformance/languages/manifest.js";
import { RUNTIMES_DIRECTORY } from "../pack-data/runtimes.js";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

/** Whether `pack`/`tool`'s upstream runs under a PHP its runtime pins. */
export function needsPhp(root: string, pack: string, tool: string): boolean {
  const runtime = readGatesManifest(root, pack, tool).upstream?.runtime;
  return runtime !== undefined && existsSync(join(root, RUNTIMES_DIRECTORY, runtime, ".php-version"));
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { values } = parseArgs({ options: { pack: { type: "string" }, tool: { type: "string" } } });
  if (values.pack === undefined || values.tool === undefined) throw new Error("usage: tool-pack-needs.ts --pack <pack> --tool <tool>");
  console.log(`php=${needsPhp(ROOT, values.pack, values.tool)}`);
}
