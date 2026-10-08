// The Vue pack's pinned reference tools, loaded from the maintainer's install (never CI).
//
// Install once, by hand, into the gitignored `<checkout>/.nera/tools/vue/` (a package.json with these exact
// versions, then `npm install`). Every generator under rerail-fixtures/vue/ loads the tools from there, never from the
// repository's own node_modules, and refuses any other version.

import { existsSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

export const TOOLS = {
  "@vue/compiler-sfc": "3.5.43",
  "@vue/compiler-dom": "3.5.43",
  "@vue/shared": "3.5.43",
  "vue-eslint-parser": "10.4.1",
  "eslint": "10.11.0",
  "eslint-plugin-vue": "10.11.1",
  "@typescript-eslint/parser": "8.70.1",
  "typescript": "6.0.3",
};

/** `<checkout>/.nera/tools/vue`: the nearest ancestor of `from` holding it, or `--tools <dir>`. */
export function toolsDirectory(from, argv = process.argv) {
  const flag = argv.indexOf("--tools");
  if (flag >= 0) return resolve(argv[flag + 1]);
  for (let dir = resolve(from); ; dir = dirname(dir)) {
    const candidate = join(dir, ".nera/tools/vue");
    if (existsSync(join(candidate, "package.json"))) return candidate;
    if (dirname(dir) === dir) throw new Error("no .nera/tools/vue above this checkout; pass --tools <dir>");
  }
}

/** `<checkout>/.nera/corpora/vue`, found the same way, or `--corpora <dir>`. */
export function corporaDirectory(from, argv = process.argv) {
  const flag = argv.indexOf("--corpora");
  if (flag >= 0) return resolve(argv[flag + 1]);
  for (let dir = resolve(from); ; dir = dirname(dir)) {
    const candidate = join(dir, ".nera/corpora/vue");
    if (existsSync(candidate)) return candidate;
    if (dirname(dir) === dir) throw new Error("no .nera/corpora/vue above this checkout; pass --corpora <dir>");
  }
}

/** Imports a pinned tool from `directory`, after checking its installed version. */
export async function load(directory, name) {
  const require = createRequire(join(directory, "package.json"));
  const manifest = JSON.parse(readFileSync(join(directory, "node_modules", name, "package.json"), "utf8"));
  if (manifest.version !== TOOLS[name]) {
    throw new Error(`${name} ${manifest.version} is installed in ${directory}; this generator pins ${TOOLS[name]}`);
  }
  return import(pathToFileURL(require.resolve(name)).href);
}

export function toolList(...names) {
  return names.map(name => ({ name, version: TOOLS[name] }));
}
