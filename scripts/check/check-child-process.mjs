#!/usr/bin/env node
// Every child process Rerail starts goes through src/engine/child-process.ts, which sets `windowsHide: true`.
//
// On Windows, Node starts a child with a visible window unless told otherwise. A console program (git, node, npm,
// PowerShell) started by a process with no console, such as the daemon or the session-start autostart, then opens
// a new console window, and Windows 11 hands it to Windows Terminal, which takes focus. One missed option opened a
// window every few seconds while the daemon ran Git, so the option is set in one place and this check keeps it there.
//
// Windows also looks for a program named without a folder in the current folder before PATH, and Rerail runs inside
// repositories it does not trust, so on Windows the helper refuses a program that is not named by absolute path.
//
// Rules, over every tracked .ts, .mts, .js and .mjs file under src/:
//   bypass   Only the helper loads `node:child_process` (or `child_process`) as a value: an `import` or `export`
//            declaration that is not type-only, `import x = require(...)`, `require(...)` or `import(...)`.
//   hidden   The helper sets `windowsHide: true` in the options it passes on.
//   path     The helper refuses a program not named by absolute path (`assertProgramPath`), and no call to one of its
//            starters names a program by a literal that is not an absolute path.
//
//   node scripts/check/check-child-process.mjs   (npm run check:child-process; part of npm run check)
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

export const HELPER = "src/engine/child-process.ts";
const MODULES = new Set(["node:child_process", "child_process"]);
const STARTERS = new Set(["spawn", "spawnSync", "execFile", "execFileSync", "spawnShown"]);
/** An absolute POSIX path, a drive-letter path or a UNC path. */
const ABSOLUTE = /^(?:\/|[A-Za-z]:[\\/]|\\\\)/u;
const CODE = /\.(?:[cm]?[jt]s)$/u;

/** Whether an import or export declaration brings in values, not only types. */
function loadsValues(node) {
  if (ts.isExportDeclaration(node)) return !node.isTypeOnly;
  const clause = node.importClause;
  if (clause === undefined) return true;
  if (clause.isTypeOnly) return false;
  if (clause.name !== undefined) return true;
  const bindings = clause.namedBindings;
  if (bindings === undefined || ts.isNamespaceImport(bindings)) return true;
  return bindings.elements.length === 0 || bindings.elements.some(element => !element.isTypeOnly);
}

/** The module a node loads as a value, as a string literal, or undefined. */
function loadedModule(node) {
  if ((ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) && node.moduleSpecifier !== undefined) {
    return loadsValues(node) ? node.moduleSpecifier : undefined;
  }
  if (ts.isImportEqualsDeclaration(node) && ts.isExternalModuleReference(node.moduleReference)) {
    return node.isTypeOnly ? undefined : node.moduleReference.expression;
  }
  if (ts.isCallExpression(node)) {
    const callee = node.expression;
    const loads = callee.kind === ts.SyntaxKind.ImportKeyword || (ts.isIdentifier(callee) && callee.text === "require");
    return loads ? node.arguments[0] : undefined;
  }
  return undefined;
}

/** The lines of `text` that load node:child_process as a value. */
function childProcessLoads(path, text) {
  const source = ts.createSourceFile(path, text, ts.ScriptTarget.Latest, false, /\.[cm]?js$/u.test(path) ? ts.ScriptKind.JS : ts.ScriptKind.TS);
  const lines = [];
  const visit = (node) => {
    const loaded = loadedModule(node);
    if (loaded !== undefined && ts.isStringLiteralLike(loaded) && MODULES.has(loaded.text)) {
      lines.push(source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1);
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return lines;
}

/** The lines of `text` that start a program named by a literal that is not an absolute path. */
function bareProgramNames(path, text) {
  const source = ts.createSourceFile(path, text, ts.ScriptTarget.Latest, false, /\.[cm]?js$/u.test(path) ? ts.ScriptKind.JS : ts.ScriptKind.TS);
  const lines = [];
  const visit = (node) => {
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && STARTERS.has(node.expression.text)) {
      const [command] = node.arguments;
      if (command !== undefined && ts.isStringLiteralLike(command) && !ABSOLUTE.test(command.text)) {
        lines.push(source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1);
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return lines;
}

/** Every problem in `files` (`{ path, text }`, repository-relative `/`-separated paths), as `{ path, line, rule, message }`. */
export function childProcessProblems(files) {
  const problems = [];
  for (const { path, text } of files) {
    if (!path.startsWith("src/") || !CODE.test(path)) continue;
    if (path === HELPER) {
      if (!/\bwindowsHide: true\b/u.test(text)) {
        problems.push({ path, line: 1, rule: "hidden", message: "the child-process helper must set windowsHide: true" });
      }
      if (!/\bassertProgramPath\(args\)/u.test(text)) {
        problems.push({ path, line: 1, rule: "path", message: "the child-process helper must refuse a program not named by absolute path (assertProgramPath)" });
      }
      continue;
    }
    for (const line of childProcessLoads(path, text)) {
      problems.push({
        path, line, rule: "bypass",
        message: `loads node:child_process; import spawn, spawnSync, execFile or execFileSync from ${HELPER} instead, which hides the child's window on Windows`,
      });
    }
    for (const line of bareProgramNames(path, text)) {
      problems.push({
        path, line, rule: "path",
        message: "names a program without its folder, which Windows looks for in the current folder first; pass an absolute path (windowsSystemProgram, resolveExecutable with binaryOnly)",
      });
    }
  }
  if (!files.some(file => file.path === HELPER)) problems.push({ path: HELPER, line: 1, rule: "hidden", message: "the child-process helper is missing" });
  return problems;
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
  // Untracked files too, so a new module is checked before its first commit.
  const listed = spawnSync("git", ["ls-files", "-z", "--cached", "--others", "--exclude-standard", "src"], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 });
  if (listed.status !== 0) throw new Error(`git ls-files failed: ${listed.stderr}`);
  const files = listed.stdout.split("\0").filter(path => CODE.test(path)).map(path => ({ path, text: readFileSync(resolve(root, path), "utf8") }));
  const problems = childProcessProblems(files);
  for (const { path, line, rule, message } of problems) console.error(`${path}:${line}: [${rule}] ${message}`);
  console.log(problems.length === 0
    ? `check-child-process: ${files.length} files start child processes only through ${HELPER}`
    : `check-child-process: ${problems.length} problem(s)`);
  process.exit(problems.length === 0 ? 0 : 1);
}
