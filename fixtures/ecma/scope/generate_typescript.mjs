// Portions of this file are adapted from @eslint-community/eslint-utils 4.10.1 (MIT); see
// ../licenses/eslint-utils/UPSTREAM.md.
//
// Writes <fixture>.expected.json for every typescript/*.ts and *.tsx beside this script: the references
// typescript-eslint's scope manager leaves unresolved (the global scope's `through`), as byte
// offsets; and for every reference it resolves, whether eslint-utils' getStaticValue would read
// the variable's initializer (`constant`: `canBeConsideredConst` with a plain-name declarator
// and an initializer) and whether a reference mutates a property of its value (`mutated`:
// `hasMutationInProperty`, with static property names); and for every variable with a definition,
// by its first definition's name, how many references it has (`variables`). Run offline:
//   node generate_typescript.mjs <node_modules with @typescript-eslint/parser> [fixture directory]
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";

const require = createRequire(join(process.argv[2], "noop.js"));
const parser = require("@typescript-eslint/parser");
const version = require("@typescript-eslint/parser/package.json").version;
const root = process.argv[3] ?? join(import.meta.dirname, "typescript");

// eslint-utils 4.10.1 `isEffectivelyConst` and `canBeConsideredConst`.
function isEffectivelyConst(variable) {
  const refs = variable.references;
  const inits = refs.filter(ref => ref.init).length;
  const reads = refs.filter(ref => ref.isReadOnly()).length;
  return inits === 1 && reads + inits === refs.length;
}

function canBeConsideredConst(variable) {
  if (variable.defs.length !== 1) return false;
  const def = variable.defs[0];
  return Boolean(def.parent && def.type === "Variable" && (def.parent.kind === "const" || isEffectivelyConst(variable)));
}

const MUTATORS = new Set(["copyWithin", "fill", "pop", "push", "reverse", "shift", "sort", "splice", "unshift"]);

// eslint-utils 4.10.1 `hasMutationInProperty`, reading a computed name only when it is a literal.
function hasMutationInProperty(variable) {
  for (const ref of variable.references) {
    let node = ref.identifier;
    while (node.parent?.type === "MemberExpression") node = node.parent;
    if (!node.parent) continue;
    const parent = node.parent;
    if ((parent.type === "AssignmentExpression" && parent.left === node) || (parent.type === "UpdateExpression" && parent.argument === node)) {
      return true;
    }
    if (parent.type === "CallExpression" && parent.callee === node && node.type === "MemberExpression") {
      const property = node.computed ? (node.property.type === "Literal" ? String(node.property.value) : null) : node.property.name;
      if (property !== null && MUTATORS.has(property)) return true;
    }
  }
  return false;
}

for (const name of readdirSync(root).filter(name => /\.tsx?$/.test(name)).sort()) {
  const source = readFileSync(join(root, name), "utf8");
  const { ast, scopeManager, visitorKeys } = parser.parseForESLint(source, {
    range: true,
    sourceType: "module",
    ecmaFeatures: { jsx: name.endsWith(".tsx") },
    lib: [],
    jsxPragma: null,
    jsxFragmentName: null,
  });
  // ESLint sets `parent` while it traverses; the parser alone does not.
  const setParents = (node, parent) => {
    node.parent = parent;
    for (const key of visitorKeys[node.type] ?? []) {
      for (const child of [node[key]].flat()) {
        if (child && typeof child.type === "string") setParents(child, node);
      }
    }
  };
  setParents(ast, null);
  const global = scopeManager.globalScope;
  const byteOffset = index => Buffer.byteLength(source.slice(0, index));
  const unresolved = global.through
    .map(reference => ({ start: byteOffset(reference.identifier.range[0]), name: reference.identifier.name }))
    .sort((a, b) => a.start - b.start);
  const resolved = new Map();
  for (const scope of scopeManager.scopes) {
    for (const reference of scope.references) {
      const variable = reference.resolved;
      if (!variable || variable.defs.length === 0) continue;
      const start = byteOffset(reference.identifier.range[0]);
      const def = variable.defs[0];
      const constant = canBeConsideredConst(variable) && def.node.id?.type === "Identifier" && def.node.init != null;
      resolved.set(start, { start, name: reference.identifier.name, constant, mutated: hasMutationInProperty(variable) });
    }
  }
  const constants = [...resolved.values()].sort((a, b) => a.start - b.start);
  // `declare global` bodies bind nothing locally (the table leaves them out).
  const augmentation = scope => scope.block.type === "TSModuleDeclaration" && scope.block.kind === "global";
  const variables = scopeManager.scopes
    .filter(scope => !augmentation(scope))
    .flatMap(scope => scope.variables)
    // A `this` parameter is a pseudo-parameter the table does not bind.
    .filter(variable => variable.defs.length > 0 && variable.name !== "this")
    .map(variable => ({ start: byteOffset(variable.defs[0].name.range[0]), name: variable.name, references: variable.references.length }))
    .sort((a, b) => a.start - b.start);
  const expected = { oracle: `@typescript-eslint/parser ${version} scope manager (lib: [])`, builtins: [], unresolved, constants, variables };
  writeFileSync(join(root, name.replace(/\.tsx?$/, ".expected.json")), `${JSON.stringify(expected, null, 1)}\n`);
}
