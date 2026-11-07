#!/usr/bin/env node
import { readdir } from "node:fs/promises";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
// The checkout root is the Rerail package; playground/ sits beside its dist/.
const rerailDist = join(dirname(root), "dist");
const entries = (await readdir(join(root, "src")))
  .filter(name => name.endsWith(".ts"))
  .sort();

function insideRerailDist(path) {
  const within = relative(rerailDist, path);
  return within !== "" && !within.startsWith("..") && !within.startsWith(sep);
}

for (const entry of entries) {
  await build({
    absWorkingDir: root,
    entryPoints: [join("src", entry)],
    outfile: join(root, "dist", entry.replace(/\.ts$/u, ".js")),
    bundle: true,
    packages: "external",
    platform: "node",
    target: "node24",
    format: "esm",
    sourcemap: true,
    plugins: [{
      name: "external-rerail-runtime",
      setup(context) {
        // Rerail's compiled runtime stays external: it finds its package root (model, native engine)
        // from its own location, and playground/src/ and playground/dist/ sit at the same depth so the
        // specifier is unchanged.
        context.onResolve({ filter: /^\.{1,2}\// }, args => {
          if (!insideRerailDist(resolve(args.resolveDir, args.path))) return;
          return { path: args.path, external: true };
        });
      },
    }],
  });
}
