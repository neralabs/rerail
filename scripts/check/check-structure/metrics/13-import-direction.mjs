// #13 Import-direction violations.
//
// Delegation: when the layer boundary lint `scripts/check/check-boundaries.mjs` (or `.js`) is tracked, this metric
// is its result: `checkBoundaries(root, { files })` → `{ violations: [{ file, line, rule, message }] }`.
// Fallback predicate, when no boundary lint exists — relative imports (static, dynamic, `require`) of
// tracked JS/TS files, resolved NodeNext-style:
//   - `engine-to-host`: nothing under `src/engine/` imports `src/hosts/`;
//   - `host-to-host`: nothing under `src/hosts/<a>/` imports `src/hosts/<b>/`;
//   - `src-outward`: nothing under `src/` imports a path outside `src/`;
//   - `public-api`: a file outside `src/` imports only a layer entry point (`src/index.*` or
//     `src/<layer>/index.*`), never a `src/` internal.
// Exemption `unit-tests`: `test/**` mirrors the layers and tests their internals directly.
// Exemption `data-roots`: `fixtures/**` and `packs/<id>/upstream/**` are recorded data, not code.

import { pathToFileURL } from "node:url";
import { join } from "node:path";
import { extractImports, finding, isDataPath, isScript, resolveImport, sortFindings } from "../lib.mjs";

export const id = 13;
export const title = "Import-direction violations";
export const target = "0";

const LINTS = ["scripts/check/check-boundaries.mjs", "scripts/check-boundaries.js"];

function isLayerEntry(path) {
  return /^src\/(?:[^/]+\/)?index\.[cm]?[jt]sx?$/.test(path);
}

export function fallbackViolations(repo) {
  const violations = [];
  for (const path of repo.files) {
    if (!isScript(path) || isDataPath(path) || path.startsWith("test/")) continue;
    for (const { specifier, line } of extractImports(repo.text(path))) {
      const target = resolveImport(repo, path, specifier);
      if (target === null) continue;
      const fromHost = /^src\/hosts\/([^/]+)\//.exec(path);
      const toHost = /^src\/hosts\/([^/]+)\//.exec(target);
      let rule = null;
      if (path.startsWith("src/engine/") && toHost) rule = ["engine-to-host", "the engine imports a host"];
      else if (fromHost && toHost && fromHost[1] !== toHost[1]) rule = ["host-to-host", "a host imports another host"];
      else if (path.startsWith("src/") && !target.startsWith("src/")) rule = ["src-outward", "src/ imports outside src/"];
      else if (!path.startsWith("src/") && target.startsWith("src/") && !isLayerEntry(target)) {
        rule = ["public-api", "imports a src/ internal instead of a layer's index"];
      }
      if (rule) violations.push({ file: path, line, rule: rule[0], message: `${rule[1]}: ${specifier}` });
    }
  }
  return violations;
}

export async function check(repo) {
  const lint = LINTS.find((path) => repo.has(path));
  let violations;
  let source;
  if (lint) {
    const module = await import(pathToFileURL(join(repo.root, lint)).href);
    violations = (await module.checkBoundaries(repo.root, { files: repo.files })).violations;
    source = lint;
  } else {
    violations = fallbackViolations(repo);
    source = "fallback scan";
  }
  const findings = violations.map((violation) =>
    finding(violation.file, `[${violation.rule}] ${violation.message}`, violation.line ?? undefined),
  );
  const files = new Set(violations.map((violation) => violation.file));
  return {
    actual: `${violations.length} in ${files.size} files (${source})`,
    findings: sortFindings(findings),
  };
}
