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
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { CODE, codeFiles, eachNode, FS_MODULES, lineOf, originalName, parseSource } from "./source-files.mjs";

export const HELPER = "test/support/platform/remove-folder.ts";
/** Files left to remove folders themselves, each with the reason. */
export const EXEMPT = new Map([
  ["test/scripts/ensure-sqlite.test.mjs", "runs under `node --test`, which cannot load the TypeScript helper"],
  ["test/support/platform/remove-folder.test.ts", "shows the one try the helper replaces failing while a process runs in the folder"],
  ["test/support/temporary/run-folder.ts", "runs in vitest's own process, not in a test: it makes writable a read-only folder a killed test left, puts back the owner file of a run folder it removed only in part, and leaves a folder it cannot remove for the next run after about 1 s of tries for its own and 100 ms for an ended run's, which the end of each run waits for, where the helper retries over 5.5 s"],
]);
const MODULES = new Set(FS_MODULES);
const REMOVALS = new Set(["rm", "rmSync", "rmdir", "rmdirSync"]);
const DIRECT_MESSAGE = `removes a folder directly; use removeFolder or removeFolderSync from ${HELPER}, which names a process the test left running in it and waits out any other holder on Windows`;

/** The `{ ... }` elements of an import clause; none for a default or namespace import alone. */
function namedElements(clause) {
  const bindings = clause.namedBindings;
  return bindings !== undefined && ts.isNamedImports(bindings) ? bindings.elements : [];
}

/** The names a declaration binds to a whole fs module (`* as fs`, a default import, `{ promises }`). */
function moduleBindings(clause) {
  const bindings = clause.namedBindings;
  const namespace = bindings !== undefined && ts.isNamespaceImport(bindings) ? [bindings.name] : [];
  const promises = namedElements(clause).filter(element => originalName(element) === "promises").map(element => element.name);
  return [clause.name, ...namespace, ...promises].filter(name => name !== undefined).map(name => name.text);
}

/** Each local name an fs import binds to a removal (`rm`, `rmSync as remove`), with the removal it is. */
function removalBindings(clause) {
  return namedElements(clause)
    .filter(element => !element.isTypeOnly && REMOVALS.has(originalName(element)))
    .map(element => [element.name.text, originalName(element)]);
}

/** The text of an options property's name, when it is a plain name or a string. */
function propertyKey(property) {
  const name = property.name;
  return name !== undefined && (ts.isIdentifier(name) || ts.isStringLiteral(name)) ? name.text : undefined;
}

/** Whether an options property may set `recursive`: a spread may hold it, and any value but `false` asks for it. */
function mayRecurse(property) {
  if (ts.isSpreadAssignment(property)) return true;
  if (propertyKey(property) !== "recursive") return false;
  return !ts.isPropertyAssignment(property) || property.initializer.kind !== ts.SyntaxKind.FalseKeyword;
}

/** Whether a call of `removal` with `args` removes a folder: any `rmdir`, or an `rm` whose options ask for `recursive`. */
function removesFolder(removal, args) {
  if (removal === "rmdir" || removal === "rmdirSync") return true;
  const options = args[1];
  if (options === undefined) return false;
  return !ts.isObjectLiteralExpression(options) || options.properties.some(mayRecurse);
}

/** The plain name a call's target starts from, through one `.promises` (`fs`, `fs.promises`); undefined for any other. */
function rootName(target) {
  const root = ts.isPropertyAccessExpression(target) && target.name.text === "promises" ? target.expression : target;
  return ts.isIdentifier(root) ? root.text : undefined;
}

/** The removal a call's callee names, through `bound` names or `namespaces`; undefined for any other call. */
function calledRemoval(callee, bound, namespaces) {
  if (ts.isIdentifier(callee)) return bound.get(callee.text);
  if (!ts.isPropertyAccessExpression(callee) || !REMOVALS.has(callee.name.text)) return undefined;
  return namespaces.has(rootName(callee.expression)) ? callee.name.text : undefined;
}

/** The module an import statement names; undefined for any other statement. */
function importedModule(statement) {
  return ts.isImportDeclaration(statement) && ts.isStringLiteral(statement.moduleSpecifier) ? statement.moduleSpecifier.text : undefined;
}

/**
 * What `source`'s imports bind: the names of whole fs modules (`namespaces`), each name bound to a removal (`bound`),
 * and every import of rimraf. A type-only import binds nothing.
 */
function fsImports(source) {
  const imports = { namespaces: new Set(), bound: new Map(), rimraf: [] };
  for (const statement of source.statements) {
    const module = importedModule(statement);
    if (module === "rimraf") imports.rimraf.push(statement);
    if (MODULES.has(module) && statement.importClause?.isTypeOnly === false) addFsBindings(imports, statement.importClause);
  }
  return imports;
}

/** Adds what an fs import's clause binds to `imports`. */
function addFsBindings(imports, clause) {
  for (const name of moduleBindings(clause)) imports.namespaces.add(name);
  for (const [name, removal] of removalBindings(clause)) imports.bound.set(name, removal);
}

/** The lines of `text` that remove a folder without the helper: each rimraf import, then each removal call. */
function directRemovals(path, text) {
  const source = parseSource(path, text);
  const { namespaces, bound, rimraf } = fsImports(source);
  const lines = rimraf.map(statement => lineOf(source, statement));
  eachNode(source, (node) => {
    const removal = ts.isCallExpression(node) ? calledRemoval(node.expression, bound, namespaces) : undefined;
    if (removal !== undefined && removesFolder(removal, node.arguments)) lines.push(lineOf(source, node));
  });
  return lines;
}

/** Whether the check reads `path`: a code file under test/, but the helper and the files EXEMPT names. */
const checked = path => path.startsWith("test/") && CODE.test(path) && path !== HELPER && !EXEMPT.has(path);

/** Every problem in `files` (`{ path, text }`, repository-relative `/`-separated paths), as `{ path, line, rule, message }`. */
export function folderRemovalProblems(files) {
  return files
    .filter(({ path }) => checked(path))
    .flatMap(({ path, text }) => directRemovals(path, text).map(line => ({ path, line, rule: "direct", message: DIRECT_MESSAGE })));
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const files = codeFiles(resolve(dirname(fileURLToPath(import.meta.url)), "..", ".."), "test");
  const problems = folderRemovalProblems(files);
  for (const { path, line, rule, message } of problems) console.error(`${path}:${line}: [${rule}] ${message}`);
  console.log(problems.length === 0
    ? `check-folder-removal: ${files.length} files remove folders only through ${HELPER}`
    : `check-folder-removal: ${problems.length} problem(s)`);
  process.exit(problems.length === 0 ? 0 : 1);
}
