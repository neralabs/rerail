// Writes eslint.expected.json: for every eslint/*.js here and each languageOptions case below, the references ESLint
// resolves to a declared global (a global-scope variable without definitions: ecmaVersion builtins, sourceType
// "commonjs", configured globals, /* global */ comments), as byte offsets. Run offline, not in CI:
//   node generate_eslint.mjs <node_modules with eslint@10.11.0>
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";

const CASES = [
  { ecmaVersion: 5, sourceType: "script" },
  { ecmaVersion: 3, sourceType: "script", globals: { Number: "off", window: "readonly" } },
  { ecmaVersion: 2015, sourceType: "module", globals: { String: "off", undeclared: "writable" } },
  { ecmaVersion: 2020, sourceType: "commonjs" },
  { ecmaVersion: "latest", sourceType: "module", globals: { Temporal: "off", jQuery: "off", Intl: "readonly" } },
  { ecmaVersion: 2025, sourceType: "script", globals: { custom: "readonly", Reflect: "writable" } },
];

const require = createRequire(join(process.argv[2], "noop.js"));
const { Linter } = require("eslint");
const { version } = require("eslint/package.json");
const root = join(import.meta.dirname, "eslint");
const files = [];
for (const name of readdirSync(root).filter(name => name.endsWith(".js")).sort()) {
  const source = readFileSync(join(root, name), "utf8");
  const byteOffset = index => Buffer.byteLength(source.slice(0, index));
  // `es2015-*.js` uses ES2015 syntax, which the ES3 and ES5 parsers refuse; `export` needs a module.
  const modern = name.startsWith("es2015-");
  const applicable = CASES.filter(({ ecmaVersion, sourceType }) => !modern || (ecmaVersion !== 3 && ecmaVersion !== 5 && sourceType === "module"));
  const cases = applicable.map(languageOptions => {
    const found = [];
    const oracle = {
      create(context) {
        return {
          "Program:exit"() {
            for (const variable of context.sourceCode.scopeManager.globalScope.variables) {
              if (variable.defs.length > 0) continue;
              for (const reference of variable.references) {
                found.push({ start: byteOffset(reference.identifier.range[0]), name: variable.name });
              }
            }
          },
        };
      },
    };
    const messages = new Linter({ configType: "flat" }).verify(source, [
      { files: ["**/*.js"], languageOptions, plugins: { oracle: { rules: { globals: oracle } } }, rules: { "oracle/globals": "error" } },
    ], "fixture.js");
    const fatal = messages.find(message => message.fatal);
    if (fatal !== undefined) throw new Error(`${name}: ${fatal.message}`);
    return { languageOptions, globals: found.sort((a, b) => a.start - b.start) };
  });
  files.push({ path: name, cases });
}
const expected = { oracle: `eslint ${version} (espree) global scope: references to variables without definitions`, files };
writeFileSync(join(import.meta.dirname, "eslint.expected.json"), `${JSON.stringify(expected, null, 1)}\n`);
