/** Filesystem discovery for explicit matching roots; snapshots retain edited file text. */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { isAbsolute, join, relative, resolve, sep } from "node:path";

export type SourceFile = { path: string; source: string; language: string };
const posix = (path: string): string => path.split(sep).join("/");
const tests = (path: string): boolean => path.split("/").includes("tests");

export function matchingApplications(root: string, input: readonly SourceFile[], requested: readonly string[]) {
  const roots = [...new Set(requested.map(name => {
    const absolute = resolve(root, name);
    let directory = false;
    try { directory = statSync(absolute).isDirectory(); } catch { /* Match the original missing-directory failure. */ }
    if (!directory) throw new Error(`Matching application root does not exist: ${name}`);
    const path = posix(relative(root, absolute));
    if (path === ".." || path.startsWith("../") || isAbsolute(path)) throw new Error(`Matching application root is outside repository: ${name}`);
    return path;
  }))];
  const files = new Map(input.map(file => [file.path, file]));
  function walk(directory: string): void {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const absolute = join(directory, entry.name), path = posix(relative(root, absolute));
      if (tests(path)) continue;
      if (entry.isDirectory()) { walk(absolute); continue; }
      if (!path.endsWith(".py") || files.has(path)) continue;
      if (entry.isSymbolicLink() && !statSync(absolute).isFile()) continue;
      files.set(path, { path, language: "python", source: new TextDecoder("utf-8", { fatal: true }).decode(readFileSync(absolute)) });
    }
  }
  for (const path of roots) walk(resolve(root, path));
  return { roots, files: [...files.values()] };
}
