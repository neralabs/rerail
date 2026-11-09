#!/usr/bin/env node
// Every rename in src/ goes through src/engine/file-replace.ts, which retries one Windows refuses for a moment.
//
// On Windows, renaming onto or from a file another process holds open fails with EPERM, EACCES or EBUSY: a status line
// reading daemon.json, an editor with a source file open, antivirus or the indexer scanning a file just written. One
// unretried rename made the daemon start fail and a rewind leave the agent's edit in place, so renames go through one
// helper and this check keeps them there.
//
// Rule, over every tracked .ts, .mts, .js and .mjs file under src/ but the helper:
//   bypass   No `rename` or `renameSync` is imported from node:fs, node:fs/promises, fs or fs/promises, and none is
//            read from a namespace, default or `promises` import of them (`fs.rename`, `promises.rename`).
//
//   node scripts/check/check-file-replace.mjs   (npm run check:file-replace; part of npm run check)
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

export const HELPER = "src/engine/file-replace.ts";
const MODULES = new Set(["node:fs", "node:fs/promises", "fs", "fs/promises"]);
const RENAMES = new Set(["rename", "renameSync"]);
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

/** The lines of `text` that rename a file without the helper. */
function bareRenames(path, text) {
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
          && bindings.elements.some(element => !element.isTypeOnly && RENAMES.has((element.propertyName ?? element.name).text))) {
          lines.push(line(node));
        }
      }
    }
    if (ts.isPropertyAccessExpression(node) && RENAMES.has(node.name.text)) {
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
export function fileReplaceProblems(files) {
  const problems = [];
  for (const { path, text } of files) {
    if (!path.startsWith("src/") || !CODE.test(path) || path === HELPER) continue;
    for (const line of bareRenames(path, text)) {
      problems.push({
        path, line, rule: "bypass",
        message: `renames a file directly; use renameWithRetry or renameWithRetrySync from ${HELPER}, which retries a file Windows holds for a moment`,
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
  const files = listed.stdout.split("\0").filter(path => CODE.test(path)).map(path => ({ path, text: readFileSync(resolve(root, path), "utf8") }));
  const problems = fileReplaceProblems(files);
  for (const { path, line, rule, message } of problems) console.error(`${path}:${line}: [${rule}] ${message}`);
  console.log(problems.length === 0
    ? `check-file-replace: ${files.length} files rename only through ${HELPER}`
    : `check-file-replace: ${problems.length} problem(s)`);
  process.exit(problems.length === 0 ? 0 : 1);
}
