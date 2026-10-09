// #8 Packs that pass the pack-format layout (docs/pack-format.md).
//
// Predicate, the file-level half of the validation contract (the compile half runs in the pack tests):
// every `packs/<id>/` directory
//   - has `pack.json`, `package.json`, `README.md`, `LICENSE` and `NOTICE`;
//   - has an `<id>` of lowercase letters, digits and dashes;
//   - has a `pack.json` whose `format` is `rerail.rule-pack/v1` and whose `id` is `<id>`, and whose every
//     `rules[].document` is a tracked file, while every file in `rules/` is listed;
//   - has a `package.json` named `@neralabs/rerail-pack-<id>` with a `license`, no `dependencies`, and
//     `files` publishing `pack.json`, `rules`, `LICENSE`.
// `packs/README.md` is the directory's index, not a pack.

import { finding, packIds, readJson, RULE_PACK_FORMAT, sortFindings } from "../lib.mjs";

export const id = 8;
export const title = "Packs pass the pack-format layout";
export const target = "100%";

const REQUIRED = ["pack.json", "package.json", "README.md", "LICENSE", "NOTICE"];

function packFindings(repo, pack) {
  const dir = `packs/${pack}`;
  const problems = [];
  for (const file of REQUIRED) {
    if (!repo.has(`${dir}/${file}`)) problems.push(finding(`${dir}/${file}`, "missing"));
  }
  if (!/^[a-z0-9]+(-[a-z0-9]+)*$/.test(pack)) problems.push(finding(dir, "pack id must be lowercase letters, digits and dashes"));
  const manifest = readJson(repo, `${dir}/pack.json`);
  if (repo.has(`${dir}/pack.json`)) {
    if (manifest?.format !== RULE_PACK_FORMAT) problems.push(finding(`${dir}/pack.json`, `format must be ${RULE_PACK_FORMAT}`));
    if (manifest?.id !== pack) problems.push(finding(`${dir}/pack.json`, `id must be the directory name "${pack}"`));
    const listed = new Set();
    for (const rule of Array.isArray(manifest?.rules) ? manifest.rules : []) {
      if (typeof rule?.document !== "string") continue;
      listed.add(`${dir}/${rule.document}`);
      if (!repo.has(`${dir}/${rule.document}`)) problems.push(finding(`${dir}/pack.json`, `lists missing document ${rule.document}`));
    }
    for (const path of repo.files) {
      if (path.startsWith(`${dir}/rules/`) && !listed.has(path)) problems.push(finding(path, "in rules/ but not listed in pack.json"));
    }
  }
  const pkg = readJson(repo, `${dir}/package.json`);
  if (repo.has(`${dir}/package.json`)) {
    if (pkg?.name !== `@neralabs/rerail-pack-${pack}`) problems.push(finding(`${dir}/package.json`, `name must be @neralabs/rerail-pack-${pack}`));
    if (typeof pkg?.license !== "string") problems.push(finding(`${dir}/package.json`, "license missing"));
    if (pkg?.dependencies && Object.keys(pkg.dependencies).length > 0) problems.push(finding(`${dir}/package.json`, "a pack has no dependencies"));
    const files = Array.isArray(pkg?.files) ? pkg.files : [];
    for (const required of ["pack.json", "rules", "LICENSE"]) {
      if (!files.includes(required)) problems.push(finding(`${dir}/package.json`, `files must publish ${required}`));
    }
  }
  return problems;
}

export function check(repo) {
  const packs = packIds(repo);
  const findings = [];
  let passing = 0;
  for (const pack of packs) {
    const problems = packFindings(repo, pack);
    if (problems.length === 0) passing += 1;
    findings.push(...problems);
  }
  const percent = packs.length === 0 ? 100 : Math.floor((passing * 100) / packs.length);
  return { actual: `${percent}% (${passing}/${packs.length})`, findings: sortFindings(findings) };
}
