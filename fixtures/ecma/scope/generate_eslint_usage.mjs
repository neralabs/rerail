// Writes eslint-usage.expected.json: where ESLint 10.11.0's no-unused-vars, no-undef and
// no-redeclare and typescript-eslint 8.70.1's no-unused-vars and no-redeclare report on each
// eslint-usage/ fixture, under each option set below, as byte ranges. Run offline, not in CI:
//   node generate_eslint_usage.mjs <node_modules with eslint, @typescript-eslint/parser and
//   @typescript-eslint/eslint-plugin>
// With a directory as a second argument it scores every .ts and .tsx file there instead, under
// typescript-eslint's no-unused-vars (default options) and ESLint's no-undef (typescript-eslint's
// parser, `lib: []`, ESLint's latest globals), and writes that directory's
// eslint-usage.expected.json (the repository-wide rerun).
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";

const require = createRequire(join(process.argv[2], "noop.js"));
const { Linter } = require("eslint");
const parser = require("@typescript-eslint/parser");
const plugin = require("@typescript-eslint/eslint-plugin");
const versions = {
  eslint: require("eslint/package.json").version,
  typescriptEslint: require("@typescript-eslint/eslint-plugin/package.json").version,
};

const unused = [
  {},
  { args: "all" },
  { args: "none" },
  { vars: "local" },
  { argsIgnorePattern: "^_", caughtErrorsIgnorePattern: "^_", varsIgnorePattern: "^_", reportUsedIgnorePattern: true },
  { ignoreRestSiblings: true },
  { caughtErrors: "none" },
  { destructuredArrayIgnorePattern: "^_", reportUsedIgnorePattern: true },
  { ignoreClassWithStaticInitBlock: true },
];
const script = { sourceType: "script" };
const CASES = [
  ...unused.map(options => ({ file: "reads.js", rule: "no-unused-vars", flavor: "eslint", options })),
  ...unused.map(options => ({ file: "reads.js", rule: "no-unused-vars", flavor: "typescript-eslint", options })),
  { file: "types.ts", rule: "no-unused-vars", flavor: "typescript-eslint", options: {} },
  { file: "types.ts", rule: "no-unused-vars", flavor: "typescript-eslint", options: { args: "all" } },
  { file: "ambient.d.ts", rule: "no-unused-vars", flavor: "typescript-eslint", options: {} },
  { file: "overridden.d.ts", rule: "no-unused-vars", flavor: "typescript-eslint", options: {} },
  { file: "jsx.tsx", rule: "no-unused-vars", flavor: "typescript-eslint", options: {} },
  { file: "jsx.tsx", rule: "no-unused-vars", flavor: "typescript-eslint", options: {}, languageOptions: { jsxPragma: "h", jsxFragmentName: "Fragment" } },
  { file: "jsx.tsx", rule: "no-unused-vars", flavor: "eslint", options: {} },
  { file: "dynamic.js", rule: "no-unused-vars", flavor: "eslint", options: {}, languageOptions: script },
  { file: "redeclare.js", rule: "no-unused-vars", flavor: "eslint", options: { vars: "local" }, languageOptions: script },
  { file: "redeclare.js", rule: "no-redeclare", flavor: "eslint", options: {}, languageOptions: script },
  { file: "redeclare.js", rule: "no-redeclare", flavor: "eslint", options: { builtinGlobals: false }, languageOptions: script },
  { file: "redeclare.js", rule: "no-redeclare", flavor: "eslint", options: {}, languageOptions: { sourceType: "script", globals: { top: "readonly" } } },
  { file: "redeclare.ts", rule: "no-redeclare", flavor: "typescript-eslint", options: {}, languageOptions: script },
  { file: "redeclare.ts", rule: "no-redeclare", flavor: "typescript-eslint", options: { ignoreDeclarationMerge: false }, languageOptions: script },
  { file: "lib.ts", rule: "no-redeclare", flavor: "typescript-eslint", options: {}, languageOptions: { sourceType: "script", lib: ["dom"] } },
  { file: "lib.ts", rule: "no-redeclare", flavor: "typescript-eslint", options: {}, languageOptions: { sourceType: "script" } },
  { file: "lib.ts", rule: "no-undef", flavor: "eslint", options: {}, languageOptions: { lib: ["dom"] } },
  { file: "lib.ts", rule: "no-undef", flavor: "eslint", options: {} },
  { file: "undef.js", rule: "no-undef", flavor: "eslint", options: {} },
  { file: "undef.js", rule: "no-undef", flavor: "eslint", options: { typeof: true } },
  { file: "undef.js", rule: "no-undef", flavor: "eslint", options: {}, languageOptions: { globals: { window: "readonly" } } },
];

/** Where each report of `rule` on `source` stands, as byte ranges. */
function reports(source, name, { rule, flavor, options, languageOptions = {}, parser: parserName }, inline = true) {
  const typescript = flavor === "typescript-eslint";
  const jsx = name.endsWith("x");
  const { jsxPragma, jsxFragmentName, lib, ...rest } = languageOptions;
  const parserOptions = { ecmaFeatures: { jsx } };
  if (typescript || name.endsWith(".ts") || name.endsWith(".tsx")) {
    parserOptions.lib = lib ?? [];
    if (jsxPragma !== undefined) parserOptions.jsxPragma = jsxPragma;
    if (jsxFragmentName !== undefined) parserOptions.jsxFragmentName = jsxFragmentName;
  }
  const config = {
    files: ["**/*.js", "**/*.jsx", "**/*.ts", "**/*.tsx"],
    languageOptions: { sourceType: "module", ...rest, parserOptions },
    rules: { [typescript ? `ts/${rule}` : rule]: ["error", options] },
    // The repository's own `eslint-disable` comments would hide reports the ports make.
    linterOptions: { noInlineConfig: !inline, reportUnusedDisableDirectives: "off" },
  };
  // ESLint's own cases parse JavaScript with espree: a .tsx fixture holding only JSX does too.
  const espree = !typescript && !name.endsWith(".ts") && parserName !== "typescript";
  if (!espree) config.languageOptions.parser = parser;
  else delete parserOptions.lib;
  if (typescript) config.plugins = { ts: plugin };
  const lineStarts = [0];
  for (let index = 0; index < source.length; index += 1) if (source[index] === "\n") lineStarts.push(index + 1);
  const byte = (line, column) => Buffer.byteLength(source.slice(0, lineStarts[line - 1] + column - 1));
  const linted = espree ? name.replace(/\.[jt]sx?$/, jsx ? ".jsx" : ".js") : name;
  const messages = new Linter({ configType: "flat" }).verify(source, [config], linted);
  const fatal = messages.find(message => message.fatal);
  if (fatal !== undefined) throw new Error(`${name}: ${fatal.message}`);
  const ruleId = typescript ? `ts/${rule}` : rule;
  const ranges = messages
    .filter(message => message.ruleId === ruleId)
    .map(message => [byte(message.line, message.column), byte(message.endLine, message.endColumn)]);
  return [...new Map(ranges.map(range => [range.join(), range])).values()].sort((a, b) => a[0] - b[0] || a[1] - b[1]);
}

const directory = process.argv[3];
if (directory === undefined) {
  const root = join(import.meta.dirname, "eslint-usage");
  const cases = CASES.map(item => ({ ...item, reports: reports(readFileSync(join(root, item.file), "utf8"), item.file, item) }));
  const expected = { eslint: versions.eslint, typescriptEslint: versions.typescriptEslint, cases };
  writeFileSync(join(import.meta.dirname, "eslint-usage.expected.json"), `${JSON.stringify(expected, null, 1)}\n`);
} else {
  const cases = [];
  for (const file of readdirSync(directory).filter(name => /\.tsx?$/.test(name)).sort()) {
    const source = readFileSync(join(directory, file), "utf8");
    for (const item of [
      { file, rule: "no-unused-vars", flavor: "typescript-eslint", options: {} },
      { file, rule: "no-undef", flavor: "eslint", options: {}, parser: "typescript" },
    ]) {
      cases.push({ ...item, reports: reports(source, file, item, false) });
    }
  }
  const expected = { eslint: versions.eslint, typescriptEslint: versions.typescriptEslint, cases };
  writeFileSync(join(directory, "eslint-usage.expected.json"), `${JSON.stringify(expected)}\n`);
}
