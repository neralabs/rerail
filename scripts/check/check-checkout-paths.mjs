#!/usr/bin/env node
// Every checkout path Rerail keys a record by is resolved through src/engine/catalog/checkout-control.ts.
//
// On Windows the JS realpath keeps the caller's spelling (`c:\work\app`, VS Code's), while Git prints a checkout's top
// level as stored, with an upper-case drive letter. Keys resolved both ways never met: the checkout of a session opened
// from VS Code read as unrecorded and no rule ran in it, the home-folder refusal let `c:\users\me` through, and audit
// settings and presentation preferences saved under one spelling were not found under the other. The modules that key
// checkouts read realpath only through checkout-control.ts (`checkoutRealpath`, `physicalPath`, `storedPath`), which
// resolves through the native realpath on Windows, and this check keeps them there.
//
// Rule, over the modules that key catalog rows, audit settings, presentation preferences and session checkouts by path:
//   bypass   No `realpath` or `realpathSync` is imported from node:fs, node:fs/promises, fs or fs/promises, and none is
//            read from a namespace, default or `promises` import of them.
//
//   node scripts/check/check-checkout-paths.mjs   (npm run check:checkout-paths; part of npm run check)
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

export const HELPER = "src/engine/catalog/checkout-control.ts";
/** The modules that key records by a checkout path: whole folders end in `/`. */
export const KEYED = [
  "src/engine/catalog/",
  "src/engine/audit/",
  "src/runtime/runtime.ts",
  "src/runtime/presentation-preference-store.ts",
  "src/api/operations.ts",
  "src/api/repository-operations.ts",
];
const MODULES = new Set(["node:fs", "node:fs/promises", "fs", "fs/promises"]);
const REALPATHS = new Set(["realpath", "realpathSync"]);
const CODE = /\.(?:[cm]?[jt]s)$/u;

const keyed = path => KEYED.some(entry => (entry.endsWith("/") ? path.startsWith(entry) : path === entry));

/** The names a declaration binds to a whole fs module (`* as fs`, a default import, `{ promises }`). */
function moduleBindings(clause) {
  const names = [];
  if (clause.name !== undefined) names.push(clause.name.text);
  const bindings = clause.namedBindings;
  if (bindings !== undefined && ts.isNamespaceImport(bindings)) names.push(bindings.name.text);
  if (bindings !== undefined && ts.isNamedImports(bindings)) {
    for (const element of bindings.elements) if ((element.propertyName ?? element.name).text === "promises") names.push(element.name.text);
  }
  return names;
}

/** The lines of `text` that read a realpath of node:fs directly. */
function bareRealpaths(path, text) {
  const source = ts.createSourceFile(path, text, ts.ScriptTarget.Latest, false, /\.[cm]?js$/u.test(path) ? ts.ScriptKind.JS : ts.ScriptKind.TS);
  const lines = [];
  const namespaces = new Set();
  const line = node => source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1;
  const visit = (node) => {
    if (ts.isImportDeclaration(node) && ts.isStringLiteral(node.moduleSpecifier) && MODULES.has(node.moduleSpecifier.text)) {
      const clause = node.importClause;
      if (clause !== undefined && !clause.isTypeOnly) {
        for (const name of moduleBindings(clause)) namespaces.add(name);
        const bindings = clause.namedBindings;
        if (bindings !== undefined && ts.isNamedImports(bindings)
          && bindings.elements.some(element => !element.isTypeOnly && REALPATHS.has((element.propertyName ?? element.name).text))) {
          lines.push(line(node));
        }
      }
    }
    if (ts.isPropertyAccessExpression(node) && REALPATHS.has(node.name.text)) {
      const target = node.expression;
      const root = ts.isPropertyAccessExpression(target) && target.name.text === "promises" ? target.expression : target;
      if (ts.isIdentifier(root) && namespaces.has(root.text)) lines.push(line(node));
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return lines;
}

/** Every problem in `files` (`{ path, text }`, repository-relative `/`-separated paths), as `{ path, line, rule, message }`. */
export function checkoutPathProblems(files) {
  const problems = [];
  for (const { path, text } of files) {
    if (!keyed(path) || !CODE.test(path) || path === HELPER) continue;
    for (const line of bareRealpaths(path, text)) {
      problems.push({
        path, line, rule: "bypass",
        message: `resolves a path with node:fs's realpath; use checkoutRealpath or physicalPath from ${HELPER}, which spells a Windows path as Git does`,
      });
    }
  }
  return problems;
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
  // Untracked files too, so a new module is checked before its first commit.
  const listed = spawnSync("git", ["ls-files", "-z", "--cached", "--others", "--exclude-standard", "src"], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 });
  if (listed.status !== 0) throw new Error(`git ls-files failed: ${listed.stderr}`);
  const files = listed.stdout.split("\0").filter(path => CODE.test(path) && keyed(path))
    .map(path => ({ path, text: readFileSync(resolve(root, path), "utf8") }));
  const problems = checkoutPathProblems(files);
  for (const { path, line, rule, message } of problems) console.error(`${path}:${line}: [${rule}] ${message}`);
  console.log(problems.length === 0
    ? `check-checkout-paths: ${files.length} files resolve checkout paths only through ${HELPER}`
    : `check-checkout-paths: ${problems.length} problem(s)`);
  process.exit(problems.length === 0 ? 0 : 1);
}
