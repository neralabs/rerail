// #17 Test helpers in a support directory.
//
// Predicate: every tracked script (.ts .tsx .mts .cts .js .mjs .cjs) under `test/` or `integration/` that
// is not a test file (`*.test.*`/`*.spec.*`) has a `support` directory in its path (`test/support/`,
// `integration/support/`, or a `support/` inside an area such as `test/engine/support/`).
// Exemption `ambient-declarations`: `*.d.ts` files declare types for the test root's compiler; they hold no
// helper code.
// Exemption `fixture-trees`: files below a `fixtures/` directory are data, not helpers; #3 decides where
// they may live.

import { finding, isScript, isTestFile, sortFindings } from "../lib.mjs";

export const id = 17;
export const title = "Test helpers in a support dir";
export const target = "100%";

export function check(repo) {
  const findings = [];
  let helpers = 0;
  for (const path of repo.files) {
    if (!path.startsWith("test/") && !path.startsWith("integration/")) continue;
    if (!isScript(path) || isTestFile(path) || path.split("/").includes("fixtures")) continue;
    helpers += 1;
    if (path.split("/").slice(1, -1).includes("support")) continue;
    findings.push(finding(path, "test helper outside a support/ directory"));
  }
  const inSupport = helpers - findings.length;
  const percent = helpers === 0 ? 100 : Math.floor((inSupport * 100) / helpers);
  return { actual: `${percent}% (${inSupport}/${helpers})`, findings: sortFindings(findings) };
}
