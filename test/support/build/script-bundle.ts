import { readFile } from "node:fs/promises";
import { join } from "node:path";
import type { BuildOptions } from "esbuild";
import { z } from "zod";

const manifestSchema = z.object({ scripts: z.record(z.string(), z.string()) });

/** The esbuild option each flag of the package's build scripts sets; an output path lands under `root`. */
const FLAGS: Readonly<Record<string, (value: string, root: string) => BuildOptions>> = {
  "--bundle": () => ({ bundle: true }),
  "--platform": value => ({ platform: z.enum(["node", "browser", "neutral"]).parse(value) }),
  "--format": value => ({ format: z.enum(["esm", "cjs", "iife"]).parse(value) }),
  "--target": value => ({ target: value }),
  "--outfile": (value, root) => ({ outfile: join(root, value) }),
  "--outdir": (value, root) => ({ outdir: join(root, value) }),
};

/** A flag and its value: `--target=node22` and `--external:better-sqlite3` alike, `""` for a bare flag. */
function flagOf(word: string): [flag: string, value: string] {
  const separator = word.search(/[=:]/u);
  return separator < 0 ? [word, ""] : [word.slice(0, separator), word.slice(separator + 1)];
}

/**
 * The esbuild options `npm run <script>` bundles `entry` with, read from the package's own script, so a test builds the
 * file the package ships: `entry` as the one entry point, each flag of its esbuild command, and its `--outfile` or
 * `--outdir` under `root`, which stands for the package root. Throws when the script bundles no such entry, or passes a
 * flag this does not read, rather than build something the package does not.
 */
export async function scriptBundleOptions(script: string, entry: string, root: string): Promise<BuildOptions> {
  const { scripts } = manifestSchema.parse(JSON.parse(await readFile("package.json", "utf8")));
  const words = (scripts[script] ?? "").split("&&").map(part => part.trim().split(/\s+/u))
    .find(command => command[0] === "esbuild" && command.includes(entry));
  if (words === undefined) throw new Error(`npm run ${script} bundles no ${entry}`);
  const options: BuildOptions = { entryPoints: [entry], external: [] };
  for (const [flag, value] of words.filter(word => word.startsWith("--")).map(flagOf)) {
    const set = FLAGS[flag];
    if (flag === "--external") options.external = [...options.external ?? [], value];
    else if (set !== undefined) Object.assign(options, set(value, root));
    else throw new Error(`npm run ${script} passes esbuild ${flag}, which scriptBundleOptions does not read`);
  }
  return options;
}
