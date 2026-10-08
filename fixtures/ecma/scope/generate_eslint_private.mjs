// Writes eslint-private.expected.json: where ESLint 10.11.0's no-unused-private-class-members
// reports on each case of eslint-private/cases.json, as byte ranges. The cases are the rule's own
// tests (valid/N, invalid/N) and extra ones; `typescript` cases parse with typescript-eslint's
// parser. Run offline, not in CI:
//   node generate_eslint_private.mjs <node_modules with eslint and @typescript-eslint/parser>
import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";

const require = createRequire(join(process.argv[2], "noop.js"));
const { Linter } = require("eslint");
const parser = require("@typescript-eslint/parser");
const eslint = require("eslint/package.json").version;
const RULE = "no-unused-private-class-members";

function reports({ code, typescript }) {
  // The rule's RuleTester: `ecmaVersion: 2022`, flat config's module default.
  const config = {
    files: ["**/*.js", "**/*.ts"],
    languageOptions: { ecmaVersion: 2022, sourceType: "module", ...(typescript ? { parser } : {}) },
    rules: { [RULE]: "error" },
  };
  const messages = new Linter({ configType: "flat" }).verify(code, [config], typescript ? "a.ts" : "a.js");
  const fatal = messages.find(message => message.fatal);
  if (fatal !== undefined) throw new Error(`${code}: ${fatal.message}`);
  const lineStarts = [0];
  for (let index = 0; index < code.length; index += 1) if (code[index] === "\n") lineStarts.push(index + 1);
  const byte = (line, column) => Buffer.byteLength(code.slice(0, lineStarts[line - 1] + column - 1));
  return messages
    .filter(message => message.ruleId === RULE)
    .map(message => [byte(message.line, message.column), byte(message.endLine, message.endColumn)])
    .sort((a, b) => a[0] - b[0] || a[1] - b[1]);
}

const { cases } = JSON.parse(readFileSync(join(import.meta.dirname, "eslint-private", "cases.json"), "utf8"));
const expected = { eslint, rule: RULE, cases: cases.map(item => ({ ...item, reports: reports(item) })) };
writeFileSync(join(import.meta.dirname, "eslint-private.expected.json"), `${JSON.stringify(expected, null, 1)}\n`);
