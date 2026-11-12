import { readFile } from "node:fs/promises";
import { z } from "zod";

/**
 * The Node version a test plays when it runs a built entry point as an older Node would start it: the recorded-first
 * fallback starts `node` from PATH once the recorded one is gone, and the CLI, the hooks and the status line each say
 * then that Rerail needs Node 24.
 */
export const OLDER_NODE_VERSION = "22.20.0";

/**
 * Node flags under which this Node runs a script as Node 22 would: `using` declarations are off, the syntax Node 24
 * parses and Node 22 does not, and `process.versions.node` reads {@link OLDER_NODE_VERSION}.
 */
export function olderNodeArguments(): string[] {
  const version = `Object.defineProperty(process.versions, "node", { value: ${JSON.stringify(OLDER_NODE_VERSION)} });`;
  return ["--no-js-explicit-resource-management", "--import", `data:text/javascript,${encodeURIComponent(version)}`];
}

const manifestSchema = z.object({ scripts: z.record(z.string(), z.string()) });

/**
 * The esbuild `target` option of the command in `npm run <script>` that bundles `entry`: `{}` when that command names
 * none, which is esbuild's default, the newest syntax. Throws when the script bundles no such entry.
 */
export async function bundleTarget(script: string, entry: string): Promise<{ target?: string }> {
  const { scripts } = manifestSchema.parse(JSON.parse(await readFile("package.json", "utf8")));
  const command = (scripts[script] ?? "").split("&&").map(part => part.trim().split(/\s+/u))
    .find(words => words[0] === "esbuild" && words.includes(entry));
  if (command === undefined) throw new Error(`npm run ${script} bundles no ${entry}`);
  const target = command.find(word => word.startsWith("--target="))?.slice("--target=".length);
  return target === undefined ? {} : { target };
}
