// Writes eslint.expected.json: ESLint's directive matrix. A test rule (`test/every-name`) reports every Identifier of
// the fixtures in eslint/, and ESLint's Linter applies the fixtures' directive comments; the file keeps where the
// reports that survive them start, as sorted byte offsets keyed by fixture name. `other/rule` is defined and reports
// nothing, so directives may name it. Unused directives are not reported. Run offline, not in CI:
//   node generate_eslint.mjs <node_modules with eslint@10.11.0 and @typescript-eslint/parser@8.70.1>
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";

const here = import.meta.dirname;
const require = createRequire(join(resolve(process.argv[2]), "noop.js"));
const { Linter } = require("eslint");
const parser = require("@typescript-eslint/parser");
const versions = {
  eslint: require("eslint/package.json").version,
  parser: require("@typescript-eslint/parser/package.json").version,
};

const plugins = {
  test: { rules: { "every-name": { create: context => ({ Identifier: node => context.report({ node, message: "name" }) }) } } },
  other: { rules: { rule: { create: () => ({}) } } },
};

// A report's line and column (UTF-16 code units, 1-based) as a byte offset, with ESLint's line breaks.
function offset(code, line, column) {
  const breaks = /\r\n|[\r\n\u2028\u2029]/g;
  let start = 0;
  for (let current = 1; current < line; current += 1) {
    const found = breaks.exec(code);
    start = found.index + found[0].length;
  }
  return Buffer.byteLength(code.slice(0, start)) + Buffer.byteLength(code.slice(start, start + column - 1));
}

const result = { versions };
for (const name of readdirSync(join(here, "eslint")).sort()) {
  if (!/\.(ts|tsx)$/.test(name)) continue;
  const code = readFileSync(join(here, "eslint", name), "utf8");
  const messages = new Linter({ configType: "flat" }).verify(
    code,
    [
      {
        files: ["**/*.ts", "**/*.tsx"],
        languageOptions: { parser, sourceType: "module" },
        linterOptions: { reportUnusedDisableDirectives: "off" },
        plugins,
        rules: { "test/every-name": "error", "other/rule": "error" },
      },
    ],
    name,
  );
  const fatal = messages.find(message => message.fatal);
  if (fatal !== undefined) throw new Error(`${name}: ${fatal.message}`);
  const reports = messages.filter(message => message.ruleId === "test/every-name");
  result[name] = [...new Set(reports.map(message => offset(code, message.line, message.column)))].sort((a, b) => a - b);
}
writeFileSync(join(here, "eslint.expected.json"), `${JSON.stringify(result, null, 1)}\n`);
