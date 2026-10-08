// Writes follow.expected.json: for every program in follow.cases.json, what @eslint-community/eslint-utils'
// getStaticValue makes of the argument of its last `probe(...)` call, given the scope ESLint builds for it as the
// initial scope (`value.follow`: names read through their bindings). Programs are TypeScript modules. Run offline:
//   node generate_follow.mjs <node_modules with eslint@10.11.0 and @typescript-eslint/parser@8.70.1>
import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";

const here = import.meta.dirname;
const require = createRequire(join(resolve(process.argv[2]), "noop.js"));
const { Linter } = require("eslint");
const parser = require("@typescript-eslint/parser");
const { getStaticValue } = require("@eslint-community/eslint-utils");
const versions = {
  eslint: require("eslint/package.json").version,
  parser: require("@typescript-eslint/parser/package.json").version,
  eslintUtils: require("@eslint-community/eslint-utils/package.json").version,
};

function encode(value) {
  if (value === undefined) return { undefined: null };
  if (value === null) return { null: null };
  switch (typeof value) {
    case "boolean":
      return { bool: value };
    case "number":
      return { number: Object.is(value, -0) ? "-0" : String(value) };
    case "bigint":
      return { bigint: value.toString() };
    case "string":
      return { utf16: Array.from({ length: value.length }, (_, index) => value.charCodeAt(index)) };
    case "object":
      if (Array.isArray(value)) return { array: Array.from(value, encode) };
      if (Object.getPrototypeOf(value) === Object.prototype) return { object: null };
      return { other: value.constructor?.name ?? "object" };
    default:
      return { other: typeof value };
  }
}

function staticValue(source) {
  let result;
  const probe = {
    create(context) {
      return {
        "CallExpression[callee.name='probe']"(node) {
          const found = getStaticValue(node.arguments[0], context.sourceCode.getScope(node));
          result = found === null ? { unknown: null } : encode(found.value);
        },
      };
    },
  };
  const messages = new Linter({ configType: "flat" }).verify(
    source,
    [
      {
        files: ["**/*.ts"],
        languageOptions: { parser, sourceType: "module", ecmaVersion: "latest" },
        plugins: { oracle: { rules: { probe } } },
        rules: { "oracle/probe": "error" },
      },
    ],
    "case.ts",
  );
  const fatal = messages.find(message => message.fatal);
  return fatal === undefined ? { value: result } : { error: fatal.message };
}

const cases = JSON.parse(readFileSync(join(here, "follow.cases.json"), "utf8"));
const lines = cases.map(entry => {
  const source = typeof entry === "string" ? entry : entry.src;
  const outcome = staticValue(source);
  const result = { src: source, value: outcome.error === undefined ? outcome.value : { error: outcome.error } };
  if (typeof entry !== "string") Object.assign(result, entry);
  return JSON.stringify({ ...result, src: source });
});
writeFileSync(join(here, "follow.expected.json"), `{"versions": ${JSON.stringify(versions)}, "cases": [\n${lines.join(",\n")}\n]}\n`);
