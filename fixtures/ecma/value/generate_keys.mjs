// Writes keys.expected.json: for every TypeScript source in keys.cases.json, what ESLint's own getStaticPropertyName
// (lib/rules/utils/ast-utils.js, as no-dupe-keys and no-dupe-class-members read keys) returns for each property, method,
// field, signature and member access, keyed by the byte range of its key (a computed key without its brackets). Run
// offline, not in CI:
//   node generate_keys.mjs <node_modules with eslint@10.11.0 and @typescript-eslint/parser@8.70.1>
import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";

const here = import.meta.dirname;
const modules = resolve(process.argv[2]);
const require = createRequire(join(modules, "noop.js"));
const { Linter } = require("eslint");
const parser = require("@typescript-eslint/parser");
const { getStaticPropertyName } = require(join(modules, "eslint/lib/rules/utils/ast-utils.js"));
const versions = {
  eslint: require("eslint/package.json").version,
  parser: require("@typescript-eslint/parser/package.json").version,
};

const OWNERS = ["Property", "MethodDefinition", "PropertyDefinition", "TSPropertySignature", "TSMethodSignature", "MemberExpression"];

function keys(code) {
  const found = [];
  const bytes = index => Buffer.byteLength(code.slice(0, index));
  const probe = {
    create() {
      const visit = node => {
        const key = node.type === "MemberExpression" ? node.property : node.key;
        const name = getStaticPropertyName(node);
        found.push({
          owner: node.type,
          start: bytes(key.range[0]),
          end: bytes(key.range[1]),
          name: name === null ? null : Array.from({ length: name.length }, (_, index) => name.charCodeAt(index)),
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

const cases = JSON.parse(readFileSync(join(here, "keys.cases.json"), "utf8"));
const lines = cases.map(source => JSON.stringify({ src: source, keys: keys(source) }));
writeFileSync(join(here, "keys.expected.json"), `{"versions": ${JSON.stringify(versions)}, "cases": [\n${lines.join(",\n")}\n]}\n`);
