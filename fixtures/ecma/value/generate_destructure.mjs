// Writes destructure.expected.json: for every TypeScript declaration in destructure.cases.json, the value each name
// it binds holds once it runs (TypeScript's transpileModule to JavaScript, then Node.js's vm). A name destructured
// from a literal carries the element it takes (`resolves.binding.value`); the test checks that element's value is
// the name's. Run offline, not in CI:
//   node generate_destructure.mjs <node_modules with typescript>
import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";
import vm from "node:vm";

const here = import.meta.dirname;
const require = createRequire(join(resolve(process.argv[2]), "noop.js"));
const ts = require("typescript");
const versions = { typescript: ts.version, node: process.versions.node };

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

// The names a declaration binds, read from TypeScript's own AST.
function names(source) {
  const file = ts.createSourceFile("case.ts", source, ts.ScriptTarget.Latest, true);
  const found = [];
  const visit = node => {
    if (ts.isBindingElement(node) || ts.isVariableDeclaration(node)) {
      if (ts.isIdentifier(node.name)) found.push({ name: node.name.text, start: Buffer.byteLength(source.slice(0, node.name.getStart(file))) });
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return found;
}

const cases = JSON.parse(readFileSync(join(here, "destructure.cases.json"), "utf8"));
const lines = cases.map(source => {
  const bound = names(source);
  const javascript = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ESNext } }).outputText;
  const values = vm.runInNewContext(`${javascript}\n({ ${bound.map(({ name }) => name).join(", ")} })`);
  return JSON.stringify({ src: source, names: bound.map(entry => ({ ...entry, value: encode(values[entry.name]) })) });
});
writeFileSync(join(here, "destructure.expected.json"), `{"versions": ${JSON.stringify(versions)}, "cases": [\n${lines.join(",\n")}\n]}\n`);
