// #13 Import-direction violations.
//
// This metric is the result of the layer boundary lint `scripts/check/check-boundaries.mjs` (its rules:
// engine-isolation, host-isolation, layer-order, src-self-contained, entry-points;
// see its header and test/structure/boundaries.test.mjs): `checkBoundaries(root, { files })` →
// `{ violations: [{ file, line, rule, message }] }`. A checkout without the lint fails the metric.

import { pathToFileURL } from "node:url";
import { join } from "node:path";
import { finding, sortFindings } from "../lib.mjs";

export const id = 13;
export const title = "Import-direction violations";
export const target = "0";

const LINT = "scripts/check/check-boundaries.mjs";

export async function check(repo) {
  if (!repo.has(LINT)) {
    return { actual: "no boundary lint", findings: [finding(LINT, "missing: the layer boundary lint this metric runs")] };
  }
  const module = await import(pathToFileURL(join(repo.root, LINT)).href);
  const { violations } = await module.checkBoundaries(repo.root, { files: repo.files });
  const findings = violations.map((violation) =>
    finding(violation.file, `[${violation.rule}] ${violation.message}`, violation.line ?? undefined),
  );
  const files = new Set(violations.map((violation) => violation.file));
  return { actual: `${violations.length} in ${files.size} files`, findings: sortFindings(findings) };
}
