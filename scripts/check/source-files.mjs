// What the checks that read source with the TypeScript parser share: which files are code, how one is parsed, the walk
// over every node, the line a node starts on, the name a specifier imports or exports, and the files of a folder Git
// would commit.
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import ts from "typescript";

/** A JavaScript or TypeScript file, of any module kind. */
export const CODE = /\.(?:[cm]?[jt]s)$/u;

/** Node's fs modules, by every name an import gives them. */
export const FS_MODULES = ["node:fs", "node:fs/promises", "fs", "fs/promises"];

/** `text` parsed as the file at `path`: JavaScript for a `.js`, `.cjs` or `.mjs` file, else TypeScript. */
export function parseSource(path, text) {
  return ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true, /\.[cm]?js$/u.test(path) ? ts.ScriptKind.JS : ts.ScriptKind.TS);
}

/** Calls `visit` with `root` and every node below it, each before its children, in source order. */
export function eachNode(root, visit) {
  const walk = (node) => {
    visit(node);
    ts.forEachChild(node, walk);
  };
  walk(root);
}

/** The 1-based line `node` starts on in `source`. */
export function lineOf(source, node) {
  return source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1;
}

/** The name an import or export specifier takes before any `as`: `a` in both `{ a }` and `{ a as b }`. */
export const originalName = specifier => (specifier.propertyName ?? specifier.name).text;

/**
 * Every code file under `folder` in the repository at `root`, as `{ path, text }` with a repository-relative path:
 * untracked ones too, so a new file is checked before its first commit.
 */
export function codeFiles(root, folder) {
  const listed = spawnSync("git", ["ls-files", "-z", "--cached", "--others", "--exclude-standard", folder], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 });
  if (listed.status !== 0) throw new Error(`git ls-files failed: ${listed.stderr}`);
  return listed.stdout.split("\0").filter(path => CODE.test(path)).map(path => ({ path, text: readFileSync(resolve(root, path), "utf8") }));
}
