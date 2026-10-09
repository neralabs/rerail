// Writes ts_lib_globals.json: the global variables typescript-eslint's scope manager defines for
// each TypeScript `lib` (`parserOptions.lib`), with the namespaces each one is valid in, and the
// libs each lib includes. Read from @typescript-eslint/scope-manager's generated lib definitions
// (dist/lib), which typescript-eslint derives from TypeScript's lib files. Run offline, not in CI:
//   npm install --prefix <dir> @typescript-eslint/scope-manager@8.70.1
//   node generate_ts_lib_globals.mjs <dir>/node_modules/@typescript-eslint/scope-manager
// A different version is refused; bump PIN and regenerate to move the table.
import { writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";

const PIN = "8.70.1";
const manager = process.argv[2];
if (manager === undefined) {
  throw new Error("usage: node generate_ts_lib_globals.mjs <path to node_modules/@typescript-eslint/scope-manager>");
}
const require = createRequire(join(manager, "noop.js"));
const { version } = require(join(manager, "package.json"));
if (version !== PIN) throw new Error(`@typescript-eslint/scope-manager ${version} found; the table is pinned to ${PIN}`);
// The package exports only its entry point; the lib map is its generated `dist/lib/index.js`.
const { lib } = require(join(manager, "dist/lib/index.js"));

const names = new Map([...lib].map(([name, definition]) => [definition, name]));
const space = ({ isTypeVariable, isValueVariable }) =>
  isTypeVariable && isValueVariable ? "typeValue" : isValueVariable ? "value" : "type";
const libs = {};
for (const name of [...lib.keys()].sort()) {
  const definition = lib.get(name);
  const includes = definition.libs.map(dependency => {
    const included = names.get(dependency);
    if (included === undefined) throw new Error(`${name} includes a lib the map does not name`);
    return included;
  });
  const variables = Object.fromEntries(
    [...definition.variables].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)).map(([variable, options]) => [variable, space(options)]),
  );
  libs[name] = { libs: includes, variables };
}
const table = { source: `@typescript-eslint/scope-manager@${version} dist/lib`, libs };
writeFileSync(join(import.meta.dirname, "ts_lib_globals.json"), `${JSON.stringify(table, null, 1)}\n`);
