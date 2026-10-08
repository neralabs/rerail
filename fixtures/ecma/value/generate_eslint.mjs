// Writes eslint.expected.json: where ESLint's own rules report on the fixtures in eslint/, as sorted start byte
// offsets keyed by fixture name. A fixture `<rule>.ts`, `<rule>.tsx` or `<rule>@<topic>.ts` is linted by that rule with the options below,
// through @typescript-eslint/parser (a .tsx fixture with JSX). Run offline, not in CI:
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

const OPTIONS = {
  "no-self-assign": [{ props: true }],
};

// Globals a fixture's rule reads as declared.
const GLOBALS = {
  "no-implied-eval": { setTimeout: "readonly", setInterval: "readonly", execScript: "readonly" },
};

// A report's line and column (UTF-16 code units, 1-based) as a byte offset.
function offset(code, line, column) {
  const lines = code.split("\n");
  const before = lines.slice(0, line - 1).reduce((sum, text) => sum + Buffer.byteLength(text) + 1, 0);
  return before + Buffer.byteLength(lines[line - 1].slice(0, column - 1));
}

const result = { versions };
for (const name of readdirSync(join(here, "eslint")).sort()) {
  const match = /^([^@]+)(?:@[^.]+)?\.(ts|tsx)$/.exec(name);
  if (match === null) continue;
  const [, rule] = match;
  const code = readFileSync(join(here, "eslint", name), "utf8");
  const messages = new Linter({ configType: "flat" }).verify(
    code,
    [
      {
        files: ["**/*.ts", "**/*.tsx"],
        languageOptions: { parser, sourceType: "module", ...(GLOBALS[rule] === undefined ? {} : { globals: GLOBALS[rule] }) },
        rules: { [rule]: ["error", ...(OPTIONS[rule] ?? [])] },
      },
    ],
    name,
  );
  const fatal = messages.find(message => message.fatal);
  if (fatal !== undefined) throw new Error(`${name}: ${fatal.message}`);
  result[name] = [...new Set(messages.map(message => offset(code, message.line, message.column)))].sort((a, b) => a - b);
}
writeFileSync(join(here, "eslint.expected.json"), `${JSON.stringify(result, null, 1)}\n`);
