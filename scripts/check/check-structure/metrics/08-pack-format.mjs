// #8 Packs that pass the pack-format layout (docs/pack-format.md).
//
// Predicate, the file-level half of the validation contract (the compile half runs in the pack tests):
// every `packs/<id>/` directory
//   - has `pack.json`, `package.json`, `README.md`, `LICENSE` and `NOTICE`;
//   - has an `<id>` of lowercase letters, digits and dashes;
//   - has a `pack.json` whose `format` is `rerail.rule-pack/v1` and whose `id` is `<id>`, and whose every
//     `rules[].document` is a tracked file, while every file in `rules/` is listed;
//   - has a `pack.json` `name` with no brackets, digits or ported tool, repeated exactly in its README headings and,
//     for a published pack, in docs/rule-packs.md's table;
//   - has a `package.json` named `@neralabs/rerail-pack-<id>` with a `license`, no `dependencies`, and
//     `files` publishing `pack.json`, `rules`, `LICENSE`.
// `packs/README.md` is the directory's index, not a pack.

import { finding, packIds, readJson, RULE_PACK_FORMAT, sortFindings } from "../lib.mjs";

export const id = 8;
export const title = "Packs pass the pack-format layout";
export const target = "100%";

const REQUIRED = ["pack.json", "package.json", "README.md", "LICENSE", "NOTICE"];

function headingFindings(repo, path, heading) {
  if (!repo.has(path) || repo.text(path).split("\n", 1)[0] === heading) return [];
  return [finding(path, `first line must be "${heading}"`)];
}

function toolsNamed(repo, dir, name) {
  const tools = readJson(repo, `${dir}/conformance.json`)?.tools;
  if (!Array.isArray(tools)) return [];
  return tools.map((tool) => tool?.name).filter((tool) => typeof tool === "string" && name.toLowerCase().includes(tool.toLowerCase()));
}

function docsRowFindings(repo, dir, name) {
  if (readJson(repo, `${dir}/package.json`)?.private === true) return [];
  if ((repo.text("docs/rule-packs.md") ?? "").includes(`\n| ${name} |`)) return [];
  return [finding("docs/rule-packs.md", `no table row names the pack "${name}"`)];
}

/** A pack's name says what it is about, and each place repeating it says the same. The schema requires it to exist. */
function nameFindings(repo, dir, name) {
  if (typeof name !== "string") return [];
  const problems = toolsNamed(repo, dir, name).map((tool) => finding(`${dir}/pack.json`, `name "${name}" names the tool ${tool}: tools go in covers`));
  if (/[()[\]{}\d]/.test(name)) {
    problems.push(finding(`${dir}/pack.json`, `name "${name}" has brackets or digits: a name says what the pack is about, and limits and versions go in covers`));
  }
  return [
    ...problems,
    ...headingFindings(repo, `${dir}/README.md`, `# Rerail rule pack: ${name}`),
    ...headingFindings(repo, `${dir}/candidates/README.md`, `# Rerail rule pack: ${name}, candidates`),
    ...docsRowFindings(repo, dir, name),
  ];
}

/** Every document pack.json lists must exist; what it lists goes into `listed`. */
function listedDocumentFindings(repo, dir, manifest, listed) {
  const problems = [];
  for (const rule of Array.isArray(manifest?.rules) ? manifest.rules : []) {
    if (typeof rule?.document !== "string") continue;
    listed.add(`${dir}/${rule.document}`);
    if (!repo.has(`${dir}/${rule.document}`)) problems.push(finding(`${dir}/pack.json`, `lists missing document ${rule.document}`));
  }
  return problems;
}

function unlistedRuleFindings(repo, dir, listed) {
  const problems = [];
  for (const path of repo.files) {
    if (path.startsWith(`${dir}/rules/`) && !listed.has(path)) problems.push(finding(path, "in rules/ but not listed in pack.json"));
  }
  return problems;
}

function manifestFindings(repo, dir, pack) {
  const manifest = readJson(repo, `${dir}/pack.json`);
  if (!repo.has(`${dir}/pack.json`)) return [];
  const problems = [];
  if (manifest?.format !== RULE_PACK_FORMAT) problems.push(finding(`${dir}/pack.json`, `format must be ${RULE_PACK_FORMAT}`));
  if (manifest?.id !== pack) problems.push(finding(`${dir}/pack.json`, `id must be the directory name "${pack}"`));
  const listed = new Set();
  problems.push(...listedDocumentFindings(repo, dir, manifest, listed));
  problems.push(...unlistedRuleFindings(repo, dir, listed));
  problems.push(...nameFindings(repo, dir, manifest?.name));
  return problems;
}

function publishedFileFindings(dir, pkg) {
  const files = Array.isArray(pkg?.files) ? pkg.files : [];
  return ["pack.json", "rules", "LICENSE"]
    .filter((required) => !files.includes(required))
    .map((required) => finding(`${dir}/package.json`, `files must publish ${required}`));
}

function packageFindings(repo, dir, pack) {
  const pkg = readJson(repo, `${dir}/package.json`);
  if (!repo.has(`${dir}/package.json`)) return [];
  const problems = [];
  if (pkg?.name !== `@neralabs/rerail-pack-${pack}`) problems.push(finding(`${dir}/package.json`, `name must be @neralabs/rerail-pack-${pack}`));
  if (typeof pkg?.license !== "string") problems.push(finding(`${dir}/package.json`, "license missing"));
  if (pkg?.dependencies && Object.keys(pkg.dependencies).length > 0) problems.push(finding(`${dir}/package.json`, "a pack has no dependencies"));
  return [...problems, ...publishedFileFindings(dir, pkg)];
}

function packFindings(repo, pack) {
  const dir = `packs/${pack}`;
  const problems = [];
  for (const file of REQUIRED) {
    if (!repo.has(`${dir}/${file}`)) problems.push(finding(`${dir}/${file}`, "missing"));
  }
  if (!/^[a-z0-9]+(-[a-z0-9]+)*$/.test(pack)) problems.push(finding(dir, "pack id must be lowercase letters, digits and dashes"));
  return [...problems, ...manifestFindings(repo, dir, pack), ...packageFindings(repo, dir, pack)];
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
