// #22 Stale paths left from earlier layouts.
//
// Predicate: no tracked text file mentions a retired location: `rerail-fixtures/` (the old fixtures
// repository), `tempo-oracles` (its earlier name), `code-rule-dsl/rules/opt-in` and `policies/rule-packs`
// (the pre-pack rule homes). Each occurrence is a finding at its line.
// Exemption `history`: `CHANGELOG.md` and `.changeset/*.md` record what changed, old paths included.
// Exemption `structure-gate`: this gate (`scripts/check-structure*`), its tests in `test/structure/` and
// docs/repository-structure.md name the stale paths in order to forbid them.

import { finding, isStructureGate, lineAt, sortFindings } from "../lib.mjs";

export const id = 22;
export const title = "Stale paths left from the import";
export const target = "0";

export const STALE = ["rerail-fixtures/", "tempo-oracles", "code-rule-dsl/rules/opt-in", "policies/rule-packs"];

function exempt(path) {
  return (
    path === "CHANGELOG.md" ||
    /^\.changeset\/[^/]+\.md$/.test(path) ||
    path === "docs/repository-structure.md" ||
    isStructureGate(path)
  );
}

export function check(repo) {
  const findings = [];
  const files = new Set();
  for (const path of repo.files) {
    if (exempt(path) || repo.isBinary(path)) continue;
    const text = repo.text(path);
    for (const stale of STALE) {
      for (let index = text.indexOf(stale); index >= 0; index = text.indexOf(stale, index + stale.length)) {
        files.add(path);
        findings.push(finding(path, `stale path ${stale}`, lineAt(text, index)));
      }
    }
  }
  return { actual: `${findings.length} in ${files.size} files`, findings: sortFindings(findings) };
}
