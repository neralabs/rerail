// #11 No orphan fixture families or packs.
//
// Predicate: every `fixtures/<family>/` directory belongs to a language (`languages/<family>.json`) or a
// pack (`packs/<family>/`), or is a declared shared family; and every `packs/<id>/` holds at least one
// rule document in `rules/`, or in `candidates/rules/` while it ships none because all its ports are candidates.
// Declared shared families (`SHARED_FAMILIES`), each with its reason:
//   - `ecma`: ECMAScript expected findings shared by the javascript and typescript languages and the
//     typescript, react, next and node packs;
//   - `host`: host-adapter inputs, not tied to a language;
//   - `packs`: pack-validator inputs (#1), not tied to a language;
//   - `code-rule-dsl`: the engine's query contracts and implementation goldens (`query/`), which hold every
//     language's programs, not tied to one;
//   - `simdjson-stage1`: simdjson's stage-1 structural indexes on recorded files of several kinds, the oracle of the
//     hand-written stage 1 (`stage1.rir`), not tied to a language.

import { finding, isPackRerailFile, isRuleDoc, packIds, sortFindings } from "../lib.mjs";

export const id = 11;
export const title = "No orphan fixture families or packs";
export const target = "0";

export const SHARED_FAMILIES = ["ecma", "host", "packs", "code-rule-dsl", "simdjson-stage1"];

export function check(repo) {
  const families = new Set();
  for (const path of repo.files) {
    const match = /^fixtures\/([^/]+)\//.exec(path);
    if (match) families.add(match[1]);
  }
  const findings = [];
  for (const family of [...families].sort()) {
    if (SHARED_FAMILIES.includes(family)) continue;
    if (repo.has(`languages/${family}.json`)) continue;
    if (repo.files.some((path) => path.startsWith(`packs/${family}/`))) continue;
    findings.push(finding(`fixtures/${family}`, "fixture family with no languages/<family>.json, no packs/<family>/, and not a declared shared family"));
  }
  for (const pack of packIds(repo)) {
    const holdsRules = repo.files.some(
      (path) => (path.startsWith(`packs/${pack}/rules/`) || path.startsWith(`packs/${pack}/candidates/rules/`)) && (isRuleDoc(repo, path) || isPackRerailFile(path)),
    );
    if (!holdsRules) findings.push(finding(`packs/${pack}`, "pack holds no rule documents in rules/ or candidates/rules/"));
  }
  return { actual: String(findings.length), findings: sortFindings(findings) };
}
