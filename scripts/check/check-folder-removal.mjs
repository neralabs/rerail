#!/usr/bin/env node
// Every folder a test makes is removed through test/support/platform/remove-folder.ts.
//
// On Windows a folder cannot be removed while a process runs in it or holds a file in it (EBUSY, EPERM). A test that
// removed its temp folder with one `rm` failed whenever something still held it: a Git a session left running, a
// daemon or database not closed, or antivirus scanning a file just written. The helper fails at once, naming the
// process, when one the test started still runs there, and otherwise retries as Rerail's own removals do; this check
// keeps every test on it.
//
// Rule, over every tracked .ts, .mts, .js and .mjs file under test/ but the helper and the files EXEMPT names:
//   direct   No call removes a folder itself: `rm` or `rmSync` given `recursive` (or options this check cannot read),
//            and `rmdir` or `rmdirSync` in any form, imported from node:fs, node:fs/promises, fs or fs/promises or read
//            from a namespace, default or `promises` import of them (`fs.rmSync`, `promises.rm`); nor is rimraf
//            imported. Removing one file (`rm(file)`, `rm(file, { force: true })`) is not a folder removal.
//
//   node scripts/check/check-folder-removal.mjs   (npm run check:folder-removal; part of npm run check)
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

export const HELPER = "test/support/platform/remove-folder.ts";
/** Files left to remove folders themselves, each with the reason. */
export const EXEMPT = new Map([
  ["test/scripts/ensure-sqlite.test.mjs", "runs under `node --test`, which cannot load the TypeScript helper"],
  ["test/support/platform/remove-folder.test.ts", "shows the one try the helper replaces failing while a process runs in the folder"],
]);
const MODULES = new Set(["node:fs", "node:fs/promises", "fs", "fs/promises"]);
const REMOVALS = new Set(["rm", "rmSync", "rmdir", "rmdirSync"]);
const CODE = /\.(?:[cm]?[jt]s)$/u;

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

/** Each local name an fs import binds to a removal (`rm`, `rmSync as remove`), with the removal it is. */
function removalBindings(clause) {
  const bindings = clause.namedBindings;
  if (bindings === undefined || !ts.isNamedImports(bindings)) return [];
  return bindings.elements
    .filter(element => !element.isTypeOnly && REMOVALS.has((element.propertyName ?? element.name).text))
    .map(element => [element.name.text, (element.propertyName ?? element.name).text]);
}

/** Whether a call of `removal` with `args` removes a folder: any `rmdir`, or an `rm` whose options ask for `recursive`. */
function removesFolder(removal, args) {
  if (removal === "rmdir" || removal === "rmdirSync") return true;
  const options = args[1];
  if (options === undefined) return false;
  if (!ts.isObjectLiteralExpression(options)) return true;
  return options.properties.some(property => {
    // Spread options may hold `recursive`.
    if (ts.isSpreadAssignment(property)) return true;
    const name = property.name;
    if (name === undefined || !(ts.isIdentifier(name) || ts.isStringLiteral(name)) || name.text !== "recursive") return false;
    return !ts.isPropertyAssignment(property) || property.initializer.kind !== ts.SyntaxKind.FalseKeyword;
  });
}

/** The removal a call's callee names, through `bound` names or `namespaces`; undefined for any other call. */
function calledRemoval(callee, bound, namespaces) {
  if (ts.isIdentifier(callee)) return bound.get(callee.text);
  if (!ts.isPropertyAccessExpression(callee) || !REMOVALS.has(callee.name.text)) return undefined;
  const target = callee.expression;
  const root = ts.isPropertyAccessExpression(target) && target.name.text === "promises" ? target.expression : target;
  return ts.isIdentifier(root) && namespaces.has(root.text) ? callee.name.text : undefined;
}

/** The lines of `text` that remove a folder without the helper. */
function directRemovals(path, text) {
  const source = ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true, /\.[cm]?js$/u.test(path) ? ts.ScriptKind.JS : ts.ScriptKind.TS);
  const lines = [];
  const namespaces = new Set();
  const bound = new Map();
  const line = node => source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1;
  for (const statement of source.statements) {
    if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier)) continue;
    const module = statement.moduleSpecifier.text;
    if (module === "rimraf") lines.push(line(statement));
    const clause = statement.importClause;
    if (!MODULES.has(module) || clause === undefined || clause.isTypeOnly) continue;
    for (const name of moduleBindings(clause)) namespaces.add(name);
    for (const [name, removal] of removalBindings(clause)) bound.set(name, removal);
  }
  const visit = (node) => {
    if (ts.isCallExpression(node)) {
      const removal = calledRemoval(node.expression, bound, namespaces);
      if (removal !== undefined && removesFolder(removal, node.arguments)) lines.push(line(node));
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return lines;
}

/** Every problem in `files` (`{ path, text }`, repository-relative `/`-separated paths), as `{ path, line, rule, message }`. */
export function folderRemovalProblems(files) {
  const problems = [];
  for (const { path, text } of files) {
    if (!path.startsWith("test/") || !CODE.test(path) || path === HELPER || EXEMPT.has(path)) continue;
    for (const line of directRemovals(path, text)) {
      problems.push({
        path, line, rule: "direct",
        message: `removes a folder directly; use removeFolder or removeFolderSync from ${HELPER}, which names a process the test left running in it and waits out any other holder on Windows`,
      });
    }
  }
  return problems;
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
  // Untracked files too, so a new test is checked before its first commit.
  const listed = spawnSync("git", ["ls-files", "-z", "--cached", "--others", "--exclude-standard", "test"], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 });
  if (listed.status !== 0) throw new Error(`git ls-files failed: ${listed.stderr}`);
  const files = listed.stdout.split("\0").filter(path => CODE.test(path)).map(path => ({ path, text: readFileSync(resolve(root, path), "utf8") }));
  const problems = folderRemovalProblems(files);
  for (const { path, line, rule, message } of problems) console.error(`${path}:${line}: [${rule}] ${message}`);
  console.log(problems.length === 0
    ? `check-folder-removal: ${files.length} files remove folders only through ${HELPER}`
    : `check-folder-removal: ${problems.length} problem(s)`);
  process.exit(problems.length === 0 ? 0 : 1);
}
