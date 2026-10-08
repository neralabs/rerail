// Writes eslint.expected.json: where ESLint's own rules report on the fixtures in eslint/, as sorted start byte offsets
// keyed by fixture name. A fixture `<rule>.ts` or `<rule>@<topic>.ts` is linted by that rule with the fixture's options
// and source type below (a module by default), through @typescript-eslint/parser. The rules read lines and comments
// (M12a's layout ports, and a report inside a directive comment), or count nodes past comments (the
// max-params and max-statements fixtures, whose messages hold the counts), or report each irregular white space
// run outside strings (no-irregular-whitespace, `each` over a regex group), or fix a regex group's text
// (capitalized-comments, whose fixed text is recorded). Run offline, not in CI:
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

// Options by fixture name.
const OPTIONS = {
  "capitalized-comments@fix.ts": ["never"],
  "capitalized-comments@inline.ts": ["always", { ignoreInlineComments: true }],
  "max-params@comments.ts": [1],
  "max-statements@comments.ts": [1],
  "no-redeclare@globals.ts": [{ builtinGlobals: true }],
  "no-redeclare@items.ts": [{ builtinGlobals: true }],
  "sort-keys@groups.ts": ["asc", { allowLineSeparatedGroups: true }],
};
// Source types by fixture name: a `/* global */` comment declares a variable of a script's global scope.
const SOURCE_TYPES = {
  "no-redeclare@globals.ts": "script",
  "no-redeclare@items.ts": "script",
};
// Configured globals by fixture name (`languageOptions.globals`).
const GLOBALS = {
  "no-redeclare@items.ts": { conf1: "readonly", conf2: "writable" },
};
// Fixtures whose messages are recorded too (under `messages`, beside each offset): the counts a
// transform writes into them.
const MESSAGES = new Set(["max-params@comments.ts", "max-statements@comments.ts"]);
// Fixtures whose fixed text is recorded too (under `fixes`): a fix a transform of a regex group writes.
const FIXES = new Set(["capitalized-comments@fix.ts"]);

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

const result = { versions, messages: {}, fixes: {} };
for (const name of readdirSync(join(here, "eslint")).sort()) {
  const match = /^([^@]+)(?:@[^.]+)?\.(ts|tsx)$/.exec(name);
  if (match === null) continue;
  const [, rule] = match;
  const code = readFileSync(join(here, "eslint", name), "utf8");
  const config = [{ files: ["**/*.ts", "**/*.tsx"], languageOptions: { parser, sourceType: SOURCE_TYPES[name] ?? "module", globals: GLOBALS[name] ?? {} }, rules: { [rule]: ["error", ...(OPTIONS[name] ?? [])] } }];
  const messages = new Linter({ configType: "flat" }).verify(code, config, name);
  const fatal = messages.find(message => message.fatal);
  if (fatal !== undefined) throw new Error(`${name}: ${fatal.message}`);
  result[name] = [...new Set(messages.map(message => offset(code, message.line, message.column)))].sort((a, b) => a - b);
  if (FIXES.has(name)) {
    result.fixes[name] = new Linter({ configType: "flat" }).verifyAndFix(code, config, name).output;
  }
  if (MESSAGES.has(name)) {
    result.messages[name] = messages.map(message => [offset(code, message.line, message.column), message.message]);
  }
}
writeFileSync(join(here, "eslint.expected.json"), `${JSON.stringify(result, null, 1)}\n`);
