// #2 Rule documents (`rerail.policy/v1`) outside `packs/*/{rules,candidates}`.
//
// Predicate: every tracked JSON file whose top-level `format` is `rerail.policy/v1` sits under
// `packs/<id>/rules/` (shipped) or `packs/<id>/candidates/` (scored, never shipped).
// Exemption `engine-test-inputs`: a rule document may also be an engine or host test input under
// `fixtures/` (`fixtures/<family>/compile/…`, `fixtures/host/…`): the input of a test, never shipped (#23)
// and never a second rule set. So no directory on its path below `fixtures/` may be named `rules`,
// `candidates`, `probes` or `policies`: those names mark rule sets, which belong in a pack. A benchmark or
// test that needs a pack's rule references the pack's document rather than copying it.
// Exemption `validator-inputs`: `fixtures/packs/**` holds pack-shaped inputs to the pack validator and the
// conformance runner; the pack format puts their rule documents in `rules/` (as #1 allows their pack.json).

import { finding, isRuleDoc, sortFindings } from "../lib.mjs";

export const id = 2;
export const title = "Rule docs outside `packs/*/{rules,candidates}`";
export const target = "0";

const RULE_SET_DIRS = ["rules", "candidates", "probes", "policies"];

function isTestInput(path) {
  if (path.startsWith("fixtures/packs/")) return true;
  if (!path.startsWith("fixtures/")) return false;
  return !path.split("/").slice(1, -1).some((segment) => RULE_SET_DIRS.includes(segment));
}

export function check(repo) {
  const findings = [];
  const dirs = new Set();
  for (const path of repo.files) {
    if (!isRuleDoc(repo, path)) continue;
    if (/^packs\/[^/]+\/(rules|candidates)\//.test(path) || isTestInput(path)) continue;
    const message = path.startsWith("fixtures/")
      ? "rule document in a fixtures rule-set directory (rules/, candidates/, probes/, policies/): rule sets belong in packs/<id>/"
      : "rule document outside packs/<id>/rules/, packs/<id>/candidates/ and fixtures/ test inputs";
    findings.push(finding(path, message));
    dirs.add(path.slice(0, path.lastIndexOf("/")));
  }
  return { actual: `${findings.length} in ${dirs.size} dirs`, findings: sortFindings(findings) };
}
