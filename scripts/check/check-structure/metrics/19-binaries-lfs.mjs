// #19 Large binaries committed without LFS.
//
// Predicate: every tracked binary larger than 100 KB is stored with Git LFS, i.e. the last
// `.gitattributes` line whose pattern matches it sets `filter=lfs` (a later `-filter`, `!filter` or other
// `filter=` unsets it). A file is binary when its first 8000 bytes hold a NUL byte (git's own test) or its
// extension is a known binary format (BINARY_EXTENSIONS). `.gitattributes` patterns follow gitattributes
// rules: without a `/` a pattern matches the basename at any depth; with one it is anchored to the
// directory of the `.gitattributes` file that declares it. Nested `.gitattributes` files apply below their
// directory and override the root's.
// A checkout without LFS content holds small text pointers in place of the binaries; those pass, and the
// rule still catches any binary committed outside LFS.

import { basename, compare, dirname, finding, globToRegExp, sortFindings } from "../lib.mjs";

export const id = 19;
export const title = "Large binaries committed without LFS";
export const target = "0 (>100 KB)";

export const LIMIT_BYTES = 100 * 1024;
export const BINARY_EXTENSIONS = [
  ".bin", ".safetensors", ".onnx", ".pt", ".wasm", ".node", ".so", ".dylib", ".dll", ".exe",
  ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".pdf", ".zip", ".gz", ".tgz", ".zst", ".7z", ".sqlite", ".db",
];

/** The `.gitattributes` rules in precedence order (root first; deeper files later, so they win). */
export function parseGitattributes(repo) {
  const files = repo.files
    .filter((path) => basename(path) === ".gitattributes")
    .sort((a, b) => a.split("/").length - b.split("/").length || compare(a, b));
  const rules = [];
  for (const file of files) {
    const base = dirname(file);
    for (const raw of repo.text(file).split("\n")) {
      const line = raw.trim();
      if (line === "" || line.startsWith("#")) continue;
      const [pattern, ...attributes] = line.split(/\s+/);
      let lfs = null;
      for (const attribute of attributes) {
        if (attribute === "filter=lfs") lfs = true;
        else if (attribute === "-filter" || attribute === "!filter" || attribute.startsWith("filter=")) lfs = false;
      }
      if (lfs === null) continue;
      const anchored = pattern.replace(/^\//, "").includes("/");
      const glob = pattern.replace(/^\//, "");
      rules.push({ base, anchored, regexp: globToRegExp(glob), lfs });
    }
  }
  return rules;
}

export function isLfs(rules, path) {
  let lfs = false;
  for (const rule of rules) {
    if (rule.base !== "" && !path.startsWith(`${rule.base}/`)) continue;
    const relative = rule.base === "" ? path : path.slice(rule.base.length + 1);
    const subject = rule.anchored ? relative : basename(relative);
    if (rule.regexp.test(subject)) lfs = rule.lfs;
  }
  return lfs;
}

export function check(repo) {
  const rules = parseGitattributes(repo);
  const findings = [];
  for (const path of repo.files) {
    const size = repo.size(path);
    if (size <= LIMIT_BYTES) continue;
    const ext = basename(path).includes(".") ? basename(path).slice(basename(path).lastIndexOf(".")) : "";
    if (!BINARY_EXTENSIONS.includes(ext) && !repo.isBinary(path)) continue;
    if (isLfs(rules, path)) continue;
    findings.push(finding(path, `${(size / 1024).toFixed(0)} KB binary not tracked by LFS in .gitattributes`));
  }
  return { actual: String(findings.length), findings: sortFindings(findings) };
}
