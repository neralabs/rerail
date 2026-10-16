// The nera-data-layer files the benchmark evaluates: tracked Python files, labelled repo-relative.
import { spawnSync } from "node:child_process";
import { readFileSync, realpathSync } from "node:fs";
import { resolve } from "node:path";
import { compareStrings, type Engine, type SourceFile } from "./engine.js";

function git(root: string, ...args: string[]): string {
  const result = spawnSync("git", ["-C", root, ...args], { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 });
  if (result.status !== 0) throw new Error(`git -C ${root} ${args.join(" ")} failed: ${result.stderr.trim()}`);
  return result.stdout;
}

export interface GitState {
  commit: string;
  /** Paths `git status --porcelain` reports: what the working tree adds to `commit`. */
  changes: string[];
}

/** A tracked file that is not evaluated, and why. Never dropped silently: reports list these. */
export interface SkippedFile {
  path: string;
  reason: string;
}

export interface Repository {
  root: string;
  git: GitState;
  files: SourceFile[];
  bytes: number;
  skipped: SkippedFile[];
  /** Tracked files that are not Python: never evaluated (a rule scope that includes one is reported). */
  otherPaths: string[];
}

/** The Python module path the enforcer would send for `path` (package markers on disk), or none. */
export function modulePath(engine: Engine, root: string, path: string): Pick<SourceFile, "module"> {
  const module = engine.binding.modulePath("python", resolve(root, path));
  return module === null ? {} : { module };
}

/**
 * The files the data-layer linters read: tracked `.py` files (`git ls-files`), labelled by their path relative to the
 * repository root, with their working-tree contents.
 */
export function collectRepository(engine: Engine, repository: string): Repository {
  const root = realpathSync(repository);
  const tracked = git(root, "ls-files", "-z").split("\0").filter(path => path.length > 0).sort(compareStrings);
  const decoder = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });
  const files: SourceFile[] = [];
  const skipped: SkippedFile[] = [];
  let bytes = 0;
  for (const path of tracked.filter(path => path.endsWith(".py"))) {
    let contents: Buffer;
    try {
      contents = readFileSync(resolve(root, path));
    } catch (error) {
      skipped.push({ path, reason: `unreadable: ${error instanceof Error ? error.message : String(error)}` });
      continue;
    }
    let source: string;
    try {
      source = decoder.decode(contents);
    } catch {
      skipped.push({ path, reason: "not valid UTF-8" });
      continue;
    }
    bytes += contents.length;
    files.push({ path, language: "python", source, ...modulePath(engine, root, path) });
  }
  return {
    root,
    git: {
      commit: git(root, "rev-parse", "HEAD").trim(),
      changes: git(root, "status", "--porcelain", "--untracked-files=no").split("\n").filter(line => line.length > 0),
    },
    files,
    bytes,
    skipped,
    otherPaths: tracked.filter(path => !path.endsWith(".py")),
  };
}
