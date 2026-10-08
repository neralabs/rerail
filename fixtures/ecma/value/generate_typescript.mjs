// Writes typescript.expected.json: for every expression in typescript.cases.json, what @eslint-community/eslint-utils'
// getStaticValue makes of `(<expression>);` in a TypeScript script, given the global scope ESLint builds for it (so
// `undefined`, `NaN` and the other builtins resolve). A case marked `jsx` is a TSX element instead, and the value is
// that of its first attribute's string (typescript-estree's `Literal.value`). Run offline, not in CI:
//   node generate_typescript.mjs <node_modules with eslint@10.11.0 and @typescript-eslint/parser@8.70.1>
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

// typescript-eslint refuses legacy octal literals and escapes even in a script, as TypeScript does; espree, ESLint's
// own parser, reads them as a sloppy-mode script, so plain JavaScript that TypeScript refuses is read with it.
function staticValue(source, jsx) {
  if (jsx) return staticValueWith(source, parser, true);
  const typescript = staticValueWith(source, parser);
  if (typescript.error === undefined) return typescript;
  const javascript = staticValueWith(source, undefined);
  return javascript.error === undefined ? { ...javascript, parser: "espree" } : typescript;
}

function staticValueWith(source, customParser, jsx = false) {
  let result;
  const read = (context, node) => {
    const found = getStaticValue(node, context.sourceCode.getScope(node));
    result = found === null ? { unknown: null } : encode(found.value);
  };
  const probe = {
    create(context) {
      if (jsx) {
        return {
          "JSXAttribute > Literal"(node) {
            if (result === undefined) read(context, node);
          },
        };
      }
      return {
        "Program > ExpressionStatement"(node) {
          read(context, node.expression);
        },
      };
    },
  };
  const messages = new Linter({ configType: "flat" }).verify(
    `(${source});\n`,
    [
      {
        files: ["**/*.ts", "**/*.tsx"],
        languageOptions: { ...(customParser === undefined ? {} : { parser: customParser }), sourceType: "script", ecmaVersion: "latest" },
        plugins: { oracle: { rules: { probe } } },
        rules: { "oracle/probe": "error" },
      },
    ],
    jsx ? "case.tsx" : "case.ts",
  );
  const fatal = messages.find(message => message.fatal);
  return fatal === undefined ? { value: result } : { error: fatal.message };
}

const cases = JSON.parse(readFileSync(join(here, "typescript.cases.json"), "utf8"));
const lines = cases.map(entry => {
  const source = typeof entry === "string" ? entry : entry.src;
  const outcome = staticValue(source, typeof entry !== "string" && entry.jsx === true);
  const result = { src: source, value: outcome.error === undefined ? outcome.value : { error: outcome.error } };
  if (outcome.parser !== undefined) result.parser = outcome.parser;
  if (typeof entry !== "string") Object.assign(result, entry);
  return JSON.stringify({ ...result, src: source });
});
writeFileSync(join(here, "typescript.expected.json"), `{"versions": ${JSON.stringify(versions)}, "cases": [\n${lines.join(",\n")}\n]}\n`);
