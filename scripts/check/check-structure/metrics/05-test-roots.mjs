// #5 Test roots.
//
// Predicate: every tracked test file (`*.test.*`/`*.spec.*` scripts, pytest `test_*.py`/`*_test.py`,
// PHPUnit `*Test.php`) sits under `test/` (unit), `integration/` (integration) or `e2e/` (Playwright browser
// specs of the dashboard; kept out of `test/` because vitest runs every spec file under it).
// Exemption `data-roots`: `fixtures/**` and `packs/<id>/upstream/**` hold upstream projects' copied tests
// as recorded data; they are never run as Rerail's tests.
// Exemption `workbench-samples` (the `.rerail` workbench, kept in git as it is: docs/repository-structure.md, "Kept as they
// are"): a scenario of the workbench's explorer (`docs/rule-syntax/site/explorer/scenarios/`) holds the sample source
// a rule is shown on, named as the code it stands for (`test_parse.py` for a pytest rule); it is never run as a test.
// Exemption `rule-cases` (the same workbench): a pack rule's cases (`docs/rule-syntax/packs/<pack>/cases/<rule>/`) are
// the small repository its `.test` blocks read (`.fixture … from`), named as the code they stand for
// (`shop/tests/test_views.py`, a Django test module a rule leaves alone); never run as a test.
// Rust tests are not files of their own kind: `#[cfg(test)]` modules (`tests.rs`, `*_tests.rs`) are part of
// the crate by Cargo's convention and are not counted.

import { finding, isDataPath, isTestFile, sortFindings } from "../lib.mjs";

export const id = 5;
export const title = "Test roots";
export const target = "test/, integration/, e2e/";

const ROOTS = ["test/", "integration/", "e2e/"];

/** Exemption `workbench-samples`. */
const WORKBENCH_SAMPLES = "docs/rule-syntax/site/explorer/scenarios/";
/** Exemption `rule-cases`. */
const RULE_CASES = /^docs\/rule-syntax\/packs\/[^/]+\/cases\//;

export function check(repo) {
  const findings = [];
  const roots = new Set();
  for (const path of repo.files) {
    if (!isTestFile(path) || isDataPath(path) || path.startsWith(WORKBENCH_SAMPLES) || RULE_CASES.test(path)) continue;
    const root = ROOTS.find((prefix) => path.startsWith(prefix));
    if (root) {
      roots.add(root.slice(0, -1));
      continue;
    }
    roots.add(path.includes("/") ? path.slice(0, path.indexOf("/")) : ".");
    findings.push(finding(path, "test file outside test/, integration/ and e2e/"));
  }
  const listed = [...roots].sort();
  return { actual: `${listed.length} (${listed.join(", ")})`, findings: sortFindings(findings) };
}
