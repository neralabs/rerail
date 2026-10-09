// #23 Shipped rules come only from `packs/*`.
//
// Predicate: each entry of the root package.json `files` (what npm publishes) that ships rules is a pack
// path. An entry ships rules when a tracked file it covers is a rule document or a `pack.json`, or when
// the entry itself names a `rules` directory (an empty one still declares a rule home). Such an entry must
// start with `packs/<id>/`. An entry covers a file when it equals the file's path, is a directory prefix
// of it, or matches it as a glob.

import { finding, globToRegExp, isRuleDoc, readJson, sortFindings } from "../lib.mjs";

export const id = 23;
export const title = "Shipped rules come only from `packs/*`";
export const target = "packs/* only";

function covers(entry, path) {
  const clean = entry.replace(/^\.\//, "").replace(/\/$/, "");
  if (/[*?[]/.test(clean)) return globToRegExp(clean).test(path) || globToRegExp(`${clean}/**`).test(path);
  return path === clean || path.startsWith(`${clean}/`);
}

export function check(repo) {
  const files = readJson(repo, "package.json")?.files;
  const findings = [];
  const sources = new Set();
  for (const entry of Array.isArray(files) ? files : []) {
    if (typeof entry !== "string" || entry.startsWith("!")) continue;
    const covered = repo.files.filter((path) => covers(entry, path));
    const shipsRules =
      entry.replace(/\/$/, "").split("/").includes("rules") ||
      covered.some((path) => path.endsWith("/pack.json") || path === "pack.json" || isRuleDoc(repo, path));
    if (!shipsRules) continue;
    const pack = /^(?:\.\/)?packs\/([^/*]+)\//.exec(entry);
    sources.add(pack ? `packs/${pack[1]}` : entry);
    if (!pack) findings.push(finding("package.json", `files entry "${entry}" ships rules from outside packs/<id>/`));
  }
  return { actual: [...sources].sort().join(", ") || "none", findings: sortFindings(findings) };
}
