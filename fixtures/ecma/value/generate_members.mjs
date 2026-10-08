// Writes members.expected.json: for every TypeScript module in keys.cases.json and members.cases.json, what
// typescript-eslint's getStaticMemberAccessValue (packages/eslint-plugin/src/util/misc.ts at v8.70.1, copied below; it
// reads the name of a non-computed identifier or private name and folds any other key with eslint-utils'
// getStaticValue in the node's scope) returns for each property, method, field, signature and member access, keyed by
// the byte range of its key. A symbol is recorded as such, `undefined` as null. Run offline, not in CI:
//   node generate_members.mjs <node_modules with eslint@10.11.0 and @typescript-eslint/parser@8.70.1>
import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";

const here = import.meta.dirname;
const modules = resolve(process.argv[2]);
const require = createRequire(join(modules, "noop.js"));
const { Linter } = require("eslint");
const parser = require("@typescript-eslint/parser");
const { getStaticValue } = require("@eslint-community/eslint-utils");
const versions = {
  eslint: require("eslint/package.json").version,
  parser: require("@typescript-eslint/parser/package.json").version,
  eslintUtils: require("@eslint-community/eslint-utils/package.json").version,
  typescriptEslint: "8.70.1 (misc.ts getStaticMemberAccessValue)",
};

// typescript-eslint v8.70.1, packages/eslint-plugin/src/util/misc.ts (MIT), types removed.
function getStaticMemberAccessValue(node, { sourceCode }) {
  const key = node.type === "MemberExpression" ? node.property : node.key;
  const { type } = key;
  if (!node.computed && (type === "Identifier" || type === "PrivateIdentifier")) {
    return key.name;
  }
  const result = getStaticValue(key, sourceCode.getScope(node));
  if (!result) {
    return undefined;
  }
  const { value } = result;
  return typeof value === "symbol" ? value : String(value);
}

const OWNERS = ["Property", "MethodDefinition", "PropertyDefinition", "TSPropertySignature", "TSMethodSignature", "MemberExpression"];

function members(code) {
  const found = [];
  const bytes = index => Buffer.byteLength(code.slice(0, index));
  const probe = {
    create(context) {
      const visit = node => {
        const key = node.type === "MemberExpression" ? node.property : node.key;
        const name = getStaticMemberAccessValue(node, context);
        found.push({
          owner: node.type,
          start: bytes(key.range[0]),
          end: bytes(key.range[1]),
          name:
            name === undefined
              ? null
              : typeof name === "symbol"
                ? "symbol"
                : Array.from({ length: name.length }, (_, index) => name.charCodeAt(index)),
        });
      };
      return Object.fromEntries(OWNERS.map(type => [type, visit]));
    },
  };
  const messages = new Linter({ configType: "flat" }).verify(
    code,
    [{ files: ["**/*.ts"], languageOptions: { parser, sourceType: "module" }, plugins: { oracle: { rules: { probe } } }, rules: { "oracle/probe": "error" } }],
    "case.ts",
  );
  const fatal = messages.find(message => message.fatal);
  if (fatal !== undefined) throw new Error(`${code}: ${fatal.message}`);
  return found.sort((a, b) => a.start - b.start || a.end - b.end);
}

const cases = [
  ...JSON.parse(readFileSync(join(here, "keys.cases.json"), "utf8")),
  ...JSON.parse(readFileSync(join(here, "members.cases.json"), "utf8")),
];
const lines = cases.map(source => JSON.stringify({ src: source, keys: members(source) }));
writeFileSync(join(here, "members.expected.json"), `{"versions": ${JSON.stringify(versions)}, "cases": [\n${lines.join(",\n")}\n]}\n`);
