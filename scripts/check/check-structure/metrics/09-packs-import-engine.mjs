// #9 Pack directories that import the engine.
//
// Predicate: no tracked JS/TS file under `packs/` imports (static, dynamic or `require`) a path that
// resolves into `src/`, nor the package's own entry points `@neralabs/rerail` / `@neralabs/rerail/*`. A pack
// is data the engine reads; it never links against the engine.

import { extractImports, finding, isScript, resolveRelative, sortFindings } from "../lib.mjs";

export const id = 9;
export const title = "Pack dirs that import the engine";
export const target = "0";

export function check(repo) {
  const findings = [];
  const files = new Set();
  for (const path of repo.files) {
    if (!path.startsWith("packs/") || !isScript(path)) continue;
    for (const { specifier, line } of extractImports(repo.text(path))) {
      const resolved = resolveRelative(path, specifier);
      const engine = resolved?.startsWith("src/") || /^@neralabs\/rerail(\/|$)/.test(specifier);
      if (!engine) continue;
      files.add(path);
      findings.push(finding(path, `imports the engine: ${specifier}`, line));
    }
  }
  return { actual: String(files.size), findings: sortFindings(findings) };
}
