// Writes native/src/code_rule_dsl/scope/data/es_globals.generated.json: ESLint's globals for each ecmaVersion and for
// sourceType "commonjs", as `languageOptions` resolves them (conf/globals.js, conf/ecma-version.js). Run offline, not
// in CI:
//   npm install --prefix <dir> eslint@10.11.0
//   node scripts/generate/scope/es-globals.mjs <dir>/node_modules/eslint
// A different ESLint version is refused; bump PIN and regenerate to move the table.
import { writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";

const PIN = "10.11.0";
const eslint = process.argv[2];
if (eslint === undefined) throw new Error("usage: node scripts/generate/scope/es-globals.mjs <path to node_modules/eslint>");
const require = createRequire(join(eslint, "noop.js"));
const { version } = require(join(eslint, "package.json"));
if (version !== PIN) throw new Error(`eslint ${version} found; the table is pinned to eslint ${PIN}`);
const globals = require(join(eslint, "conf/globals.js"));
const { LATEST_ECMA_VERSION } = require(join(eslint, "conf/ecma-version.js"));

// ESLint's normalizeConfigGlobal: `false` is readonly, `true` writable.
const settings = table =>
  Object.fromEntries(Object.keys(table).sort().map(name => [name, table[name] ? "writable" : "readonly"]));
const versions = {};
for (const key of Object.keys(globals).filter(key => /^es\d+$/.test(key))) {
  versions[key.slice(2)] = settings(globals[key]);
}
const table = {
  source: `eslint@${version} conf/globals.js`,
  latest: LATEST_ECMA_VERSION,
  commonjs: settings(globals.commonjs),
  versions,
};
writeFileSync(
  join(import.meta.dirname, "../../../native/src/code_rule_dsl/scope/data/es_globals.generated.json"),
  `${JSON.stringify(table, null, 1)}\n`,
);
