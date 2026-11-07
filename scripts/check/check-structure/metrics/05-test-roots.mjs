// #5 Test roots.
//
// Predicate: every tracked test file (`*.test.*`/`*.spec.*` scripts, pytest `test_*.py`/`*_test.py`,
// PHPUnit `*Test.php`) sits under `test/` (unit), `integration/` (integration) or `e2e/` (Playwright browser
// specs of the dashboard; kept out of `test/` because vitest runs every spec file under it).
// Exemption `playground-package`: `playground/` is its own npm package with its own lockfile and test
// roots, run by its own scripts.
// Exemption `data-roots`: `fixtures/**` and `packs/<id>/upstream/**` hold upstream projects' copied tests
// as recorded data; they are never run as Rerail's tests.
// Rust tests are not files of their own kind: `#[cfg(test)]` modules (`tests.rs`, `*_tests.rs`) are part of
// the crate by Cargo's convention and are not counted.

import { finding, isDataPath, isTestFile, sortFindings } from "../lib.mjs";

export const id = 5;
export const title = "Test roots";
export const target = "test/, integration/, e2e/ (+ playground/)";

const ROOTS = ["test/", "integration/", "e2e/", "playground/"];

export function check(repo) {
  const findings = [];
  const roots = new Set();
  for (const path of repo.files) {
    if (!isTestFile(path) || isDataPath(path)) continue;
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
