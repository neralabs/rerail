// #16 `rules/` directories holding things other than rules.
//
// Predicate: every tracked file with a `rules` directory in its path (at any depth below it) is a rule
// document (JSON whose `format` is `rerail.policy/v1`); the metric counts the offending `rules/`
// directories.
// Exemption `data-roots`: `fixtures/**` and `packs/<id>/upstream/**` keep upstream projects' own trees,
// whose `rules/` directories hold upstream sources and recordings.
// Exemption `vendored`: `native/grammars/**` is third-party code kept byte-identical.

import { finding, isDataPath, isRuleDoc, isVendored, sortFindings } from "../lib.mjs";

export const id = 16;
export const title = "`rules/` dirs holding non-rules";
export const target = "0";

export function check(repo) {
  const findings = [];
  const dirs = new Set();
  for (const path of repo.files) {
    if (isDataPath(path) || isVendored(path)) continue;
    const segments = path.split("/");
    const index = segments.slice(0, -1).indexOf("rules");
    if (index < 0 || isRuleDoc(repo, path)) continue;
    dirs.add(segments.slice(0, index + 1).join("/"));
    findings.push(finding(path, "not a rule document, in a rules/ directory"));
  }
  return { actual: String(dirs.size), findings: sortFindings(findings) };
}
