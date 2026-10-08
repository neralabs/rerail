// Writes names.expected.json: for every TypeScript module in names.cases.json, the name ESTree gives each
// identifier (`Identifier.name`, `PrivateIdentifier.name`, `JSXIdentifier.name`: escapes decoded, a private name
// without its `#`) and the value of each string literal, as typescript-eslint's parser reads them; ESLint rules such as
// func-name-matching and no-useless-rename compare these. Keyed by byte range. Run offline, not in CI:
//   node generate_names.mjs <node_modules with eslint@10.11.0 and @typescript-eslint/parser@8.70.1>
import { readFileSync, writeFileSync } from "node:fs";
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

function names(code) {
  const found = [];
  const bytes = index => Buffer.byteLength(code.slice(0, index));
  const units = text => Array.from({ length: text.length }, (_, index) => text.charCodeAt(index));
  const record = (node, name) => found.push({ type: node.type, start: bytes(node.range[0]), end: bytes(node.range[1]), name: units(name) });
  const probe = {
    create() {
      return {
        Identifier: node => record(node, node.name),
        PrivateIdentifier: node => record(node, node.name),
        Literal: node => {
          if (typeof node.value === "string") record(node, node.value);
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
  return found.sort((a, b) => a.start - b.start || a.end - b.end);
}

const cases = JSON.parse(readFileSync(join(here, "names.cases.json"), "utf8"));
const lines = cases.map(source => JSON.stringify({ src: source, names: names(source) }));
writeFileSync(join(here, "names.expected.json"), `{"versions": ${JSON.stringify(versions)}, "cases": [\n${lines.join(",\n")}\n]}\n`);
