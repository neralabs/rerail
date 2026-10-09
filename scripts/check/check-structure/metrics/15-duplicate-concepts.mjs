// #15 Duplicate concepts.
//
// Predicate, four parts:
//   - `rule-id`: no rule `id` is defined by two rule documents across `packs/*/rules/**` and
//     `packs/*/candidates/**`. Exemption `option-variant`: documents under `packs/<p>/candidates/<rule>/`
//     (any depth) whose id is `<rule>` or ends with `:<rule>` are option variants of that rule; they may
//     share the id with each other and with `packs/<p>/rules/<rule>.json`.
//   - `pack-format`: every tracked `pack.json` has the same `format`.
//   - `conformance-format`: every tracked `conformance.json` has the same `format` (absent counts as
//     `untagged`). For both, the files whose format differs from the most common one are reported.
//   - `harness-import`: code in a pack-specific directory — `packs/<id>/`, `conformance/<id>/` or
//     `test/conformance/<id>/`, for `<id>` a pack — is imported only by code for the same pack, so no
//     shared runner depends on one pack's harness.
// Exemption `validator-inputs`: `fixtures/**` holds pack-validator inputs, deliberately malformed ones
// included; the format parts skip it.

import { basename, compare, extractImports, finding, isDataPath, isRuleDoc, isScript, packIds, readJson, resolveImport, sortFindings } from "../lib.mjs";

export const id = 15;
export const title = "Duplicate concepts";
export const target = "0 unexplained";

function ruleIdFindings(repo) {
  const byId = new Map();
  for (const path of repo.files) {
    const match = /^packs\/([^/]+)\/(rules|candidates)\//.exec(path);
    if (!match || !isRuleDoc(repo, path)) continue;
    const ruleId = readJson(repo, path).id;
    if (typeof ruleId !== "string") continue;
    if (!byId.has(ruleId)) byId.set(ruleId, []);
    byId.get(ruleId).push(path);
  }
  const findings = [];
  for (const [ruleId, paths] of [...byId].sort(([a], [b]) => compare(a, b))) {
    if (paths.length < 2) continue;
    const name = ruleId.slice(ruleId.lastIndexOf(":") + 1);
    const variantGroup = paths.every((path) => {
      const candidate = /^packs\/([^/]+)\/candidates\/([^/]+)\/./.exec(path);
      if (candidate) return candidate[2] === name;
      return /^packs\/[^/]+\/rules\/([^/]+)\.json$/.exec(path)?.[1] === name;
    });
    const packs = new Set(paths.map((path) => path.split("/")[1]));
    const shipped = paths.filter((path) => /^packs\/[^/]+\/rules\//.test(path));
    if (variantGroup && packs.size === 1 && shipped.length <= 1) continue;
    for (const path of paths) {
      findings.push(finding(path, `[rule-id] ${ruleId} is defined by ${paths.length} documents`));
    }
  }
  return findings;
}

function formatFindings(repo, name, rule) {
  const kinds = new Map();
  for (const path of repo.files) {
    if (basename(path) !== name || path.startsWith("fixtures/")) continue;
    const value = readJson(repo, path);
    const kind = typeof value?.format === "string" ? value.format : "untagged";
    if (!kinds.has(kind)) kinds.set(kind, []);
    kinds.get(kind).push(path);
  }
  if (kinds.size <= 1) return [];
  const [common] = [...kinds].sort(([a, x], [b, y]) => y.length - x.length || compare(a, b))[0];
  const findings = [];
  for (const [kind, paths] of kinds) {
    if (kind === common) continue;
    for (const path of paths) findings.push(finding(path, `[${rule}] format ${kind}; the others use ${common}`));
  }
  return findings;
}

function harnessFindings(repo) {
  const packs = packIds(repo);
  const harness = (path) => {
    const match = /^(packs|conformance|test\/conformance)\/([^/]+)\//.exec(path);
    return match && packs.includes(match[2]) ? { pack: match[2], dir: `${match[1]}/${match[2]}/` } : null;
  };
  const findings = [];
  for (const path of repo.files) {
    if (!isScript(path) || isDataPath(path)) continue;
    const own = harness(path);
    for (const { specifier, line } of extractImports(repo.text(path))) {
      const target = resolveImport(repo, path, specifier);
      const targetHarness = target === null ? null : harness(target);
      if (targetHarness !== null && targetHarness.pack !== own?.pack) {
        findings.push(finding(path, `[harness-import] imports ${targetHarness.dir}, a pack-specific harness`, line));
      }
    }
  }
  return findings;
}

export function check(repo) {
  const parts = {
    "rule ids": ruleIdFindings(repo),
    "pack.json formats": formatFindings(repo, "pack.json", "pack-format"),
    "conformance.json formats": formatFindings(repo, "conformance.json", "conformance-format"),
    "harness imports": harnessFindings(repo),
  };
  const findings = Object.values(parts).flat();
  const failing = Object.entries(parts)
    .filter(([, list]) => list.length > 0)
    .map(([part, list]) => `${part} ${list.length}`);
  return { actual: failing.length === 0 ? "0" : failing.join(", "), findings: sortFindings(findings) };
}
