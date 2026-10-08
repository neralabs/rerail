// Writes references.expected.json: for every pair of TypeScript expressions in references.cases.json, what ESLint's
// own isSameReference (lib/rules/utils/ast-utils.js, as no-self-compare's callers, operator-assignment and
// no-self-assign read it) returns for them, with static computed keys (the default) and without
// (`disableStaticComputedKey`, operator-assignment's). Each pair is the two arguments of `f(left, right);` in a class
// method, through @typescript-eslint/parser. Run offline, not in CI:
//   node generate_references.mjs <node_modules with eslint@10.11.0 and @typescript-eslint/parser@8.70.1>
import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";

const here = import.meta.dirname;
const modules = resolve(process.argv[2]);
const require = createRequire(join(modules, "noop.js"));
const { Linter } = require("eslint");
const parser = require("@typescript-eslint/parser");
const { isSameReference } = require(join(modules, "eslint/lib/rules/utils/ast-utils.js"));
const versions = {
  eslint: require("eslint/package.json").version,
  parser: require("@typescript-eslint/parser/package.json").version,
};

export const PREFIX = "class C extends D { #p; #q; m() { f(";

function same(left, right) {
  const code = `${PREFIX}${left}, ${right}); } }\n`;
  let result;
  const probe = {
    create() {
      return {
        "CallExpression[callee.name='f']"(node) {
          const [a, b] = node.arguments;
          result = { same: isSameReference(a, b), sameWithoutStaticKeys: isSameReference(a, b, true) };
        },
      };
    },
  };
  const messages = new Linter({ configType: "flat" }).verify(
    code,
    [{ files: ["**/*.ts"], languageOptions: { parser, sourceType: "module" }, plugins: { oracle: { rules: { probe } } }, rules: { "oracle/probe": "error" } }],
    "case.ts",
  );
  const fatal = messages.find(message => message.fatal);
  if (fatal !== undefined) throw new Error(`${code}: ${fatal.message}`);
  return result;
}

const cases = JSON.parse(readFileSync(join(here, "references.cases.json"), "utf8"));
const lines = cases.map(([left, right]) => JSON.stringify({ left, right, ...same(left, right) }));
writeFileSync(join(here, "references.expected.json"), `{"versions": ${JSON.stringify(versions)}, "prefix": ${JSON.stringify(PREFIX)}, "cases": [\n${lines.join(",\n")}\n]}\n`);
