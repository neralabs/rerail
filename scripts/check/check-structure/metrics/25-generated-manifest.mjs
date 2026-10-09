// #25 The generated-file manifest is consistent.
//
// Manifest: scripts/generate/README.md, a markdown table whose first column is the generated path and
// second the generator, each in backticks (see generated.mjs). A first-column path ending in `/` is a
// directory of generated files.
// Predicate:
//   - the manifest exists whenever any tracked file is named `*.generated.*`;
//   - every row's generated path is a tracked file, or for a directory row holds at least one;
//   - every row's generator is a tracked file (#6 checks that it lives under `scripts/generate/`);
//   - every `*.generated.*` file is covered by a row (its own, or a directory row above it);
//   - when a generated file's header names its generator, the header and the row name the same file.

import { finding, sortFindings } from "../lib.mjs";
import { headerGenerator, isCommentable, isGeneratedName, MANIFEST, manifestRows, rowFor } from "../generated.mjs";

export const id = 25;
export const title = "Generated-file manifest is consistent";
export const target = "0";

export function check(repo) {
  const rows = manifestRows(repo);
  const findings = [];
  const generated = repo.files.filter(isGeneratedName);
  if (!repo.has(MANIFEST) && generated.length > 0) {
    findings.push(finding(MANIFEST, `missing; ${generated.length} *.generated.* files need a manifest`));
  }
  for (const row of rows) {
    const exists = row.generated.endsWith("/")
      ? repo.files.some((path) => path.startsWith(row.generated))
      : repo.has(row.generated);
    if (!exists) findings.push(finding(MANIFEST, `row's generated path ${row.generated} is not tracked`, row.line));
    if (!repo.has(row.generator)) findings.push(finding(MANIFEST, `row's generator ${row.generator} is not tracked`, row.line));
  }
  const checked = new Set(generated);
  for (const row of rows) if (!row.generated.endsWith("/") && repo.has(row.generated)) checked.add(row.generated);
  for (const path of [...checked].sort()) {
    const row = rowFor(rows, path);
    if (row === undefined) {
      findings.push(finding(path, `not listed in ${MANIFEST}`));
      continue;
    }
    if (!isCommentable(repo, path)) continue;
    const header = headerGenerator(repo, path);
    if (header !== null && header !== row.generator) {
      findings.push(finding(path, `header names ${header}; ${MANIFEST} names ${row.generator}`));
    }
  }
  return { actual: String(findings.length), findings: sortFindings(findings) };
}
