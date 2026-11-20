// Records oxlint.expected.json: for each config of corpus.json, the rules oxlint 1.88.0 runs in each probe folder, as
// the oracle of set-up's reading of oxlint configs (src/engine/catalog/lint-config/oxlint-config.ts). Run by hand, not
// in CI, against the pinned tool:
//   npm install --prefix <tools> oxlint@1.88.0
//   node fixtures/host/lint-config/oxlint/generate_oxlint.mjs <tools>
// Any other installed version is refused.
//
// Each case's files are written to <tools>/cases/<name>/ and read there. A folder's rules are oxlint's own
// `--print-config`, run in the nearest folder holding a config file (the case's root when none does): oxlint takes
// the nearest config for each file, alone, not merged with the root's. A lint run over a probe file in every probe
// folder checks that for three rules (no-debugger, eqeqeq, no-console), except in a case whose `printOnly` says why
// it cannot be linted. --print-config does not apply `overrides`: in a folder whose config has any, only those three
// rules are recorded, from the lint run, as `{ checked, on }`. A case oxlint refuses, at --print-config or in the lint
// run, is recorded as "error". Per probe folder, the rules on (warn or error), by the names --print-config gives them
// (`no-debugger`, `react/jsx-key`), sorted and space-joined.
//
// `keys` maps each ESLint name the packs' oxlint ports give (`suppress.eslint`) to the rule oxlint configures for that
// key, from --print-config of a config setting that key alone; eslint-plugin-n's `n/<rule>` is asked as `node/<rule>`,
// the oxlint plugin that ports it. `typeAware` lists the rules `oxlint --rules --format=json` marks type-aware, which
// run only with --type-aware however --print-config shows them.
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

const HERE = import.meta.dirname;
const PACKS = join(HERE, "../../../../packs");
const PIN = "1.88.0";
const CONFIG_FILES = [".oxlintrc.json", ".oxlintrc.jsonc", "oxlint.config.ts"];
const PLUGINS = ["react", "unicorn", "typescript", "oxc", "import", "jsdoc", "jest", "vitest", "jsx-a11y", "nextjs", "react-perf", "promise", "node", "vue"];
const CATEGORIES = ["correctness", "suspicious", "pedantic", "perf", "style", "restriction", "nursery"];
/** The rules the lint run checks, each with a line that breaks it. */
const CHECKED = { "no-debugger": "debugger;", eqeqeq: "if (globalThis.a == globalThis.b) {}", "no-console": "console.log(1);" };

const tools = process.argv[2];
if (tools === undefined) throw new Error("usage: node fixtures/host/lint-config/oxlint/generate_oxlint.mjs <tools>");
const OXLINT = join(tools, "node_modules", ".bin", "oxlint");
const version = execFileSync(OXLINT, ["--version"], { encoding: "utf8" }).trim();
if (version !== `Version: ${PIN}`) throw new Error(`${version} found; the oracle is pinned to oxlint ${PIN}`);

/** `oxlint --print-config` in `cwd`: the config, or undefined when oxlint refuses it. */
function printConfig(cwd) {
  const run = spawnSync(OXLINT, ["--print-config"], { cwd, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  return run.status === 0 ? JSON.parse(run.stdout) : undefined;
}

const isOn = value => ["warn", "error", "deny", 1, 2].includes(Array.isArray(value) ? value[0] : value);
const onRules = config => Object.entries(config.rules).filter(([, value]) => isOn(value)).map(([key]) => key).sort();

/** Every ESLint name the packs' ports with an oxlint name give in `suppress.eslint`. */
function portedNames() {
  const names = new Set();
  const visit = value => {
    if (Array.isArray(value)) value.forEach(visit);
    else if (value !== null && typeof value === "object") {
      if (value.suppress?.oxlint !== undefined) for (const name of value.suppress.eslint?.rules ?? []) names.add(name);
      Object.values(value).forEach(visit);
    }
  };
  for (const pack of readdirSync(PACKS, { withFileTypes: true }).filter(entry => entry.isDirectory())) {
    const rules = join(PACKS, pack.name, "rules");
    if (!existsSync(rules)) continue;
    for (const file of readdirSync(rules).filter(name => name.endsWith(".json"))) visit(JSON.parse(readFileSync(join(rules, file), "utf8")));
  }
  return [...names].sort();
}

/** Per ported ESLint name, the rule oxlint configures for it; null where oxlint refuses the key. */
function keysOf(names, directory) {
  mkdirSync(directory, { recursive: true });
  const keys = {};
  for (const name of names) {
    const key = name.startsWith("n/") ? `node/${name.slice(2)}` : name;
    const config = { plugins: PLUGINS, categories: Object.fromEntries(CATEGORIES.map(category => [category, "off"])), rules: { [key]: "error" } };
    writeFileSync(join(directory, ".oxlintrc.json"), JSON.stringify(config));
    const printed = printConfig(directory);
    const on = printed === undefined ? [] : onRules(printed);
    if (on.length > 1) throw new Error(`${name}: --print-config turns on ${on.join(", ")}`);
    keys[name] = on[0] ?? null;
  }
  return keys;
}

/** The type-aware rules, by the names --print-config gives them. */
function typeAware() {
  const rules = JSON.parse(execFileSync(OXLINT, ["--rules", "--format=json"], { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 }));
  return rules.filter(rule => rule.type_aware).map(rule => (rule.scope === "eslint" ? rule.value : `${rule.scope}/${rule.value}`)).sort();
}

/** The nearest folder at or above `folder` holding a config file, `""` (the root) when none does. */
function configFolder(directory, folder) {
  for (let at = folder; at !== ""; at = at.includes("/") ? dirname(at) : "") {
    if (CONFIG_FILES.some(name => existsSync(join(directory, at, name)))) return at;
  }
  return "";
}

/** The checked rules oxlint reports per probe folder, from a lint run over the case; undefined when oxlint refuses it. */
function linted(directory, probes) {
  const run = spawnSync(OXLINT, ["-f", "unix", ...probes.map(folder => join(folder, "probe.js"))], { cwd: directory, encoding: "utf8" });
  if (run.stdout.includes("Failed to") || run.stderr.includes("Failed to")) return undefined;
  const found = Object.fromEntries(probes.map(folder => [folder, new Set()]));
  for (const [, file, rule] of run.stdout.matchAll(/^(.+?):\d+:\d+: .*\[\w+\/eslint\(([\w-]+)\)\]$/gmu)) found[dirname(file) === "." ? "" : dirname(file)].add(rule);
  return found;
}

/**
 * What oxlint runs in `folder`: the rules --print-config turns on, checked against the lint run's `found` (undefined
 * where the case is not linted). --print-config does not apply `overrides`, so where the config has any, only the
 * checked rules the lint run reports are known: `{ checked, on }`.
 */
function folderRecord(directory, folder, printed, found) {
  const rules = onRules(printed);
  if (found === undefined) return rules.join(" ");
  if ((printed.overrides ?? []).length > 0) return { checked: Object.keys(CHECKED).sort().join(" "), on: [...found].sort().join(" ") };
  const disagreeing = Object.keys(CHECKED).filter(rule => rules.includes(rule) !== found.has(rule));
  if (disagreeing.length > 0) throw new Error(`${directory} ${folder}: --print-config and the lint run disagree on ${disagreeing.join(", ")}`);
  return rules.join(" ");
}

/** What oxlint runs in each probe folder of the case in `directory`, or "error"; `printOnly` skips the lint run. */
function caseRules(directory, probes, printOnly) {
  const printed = probes.map(folder => printConfig(join(directory, configFolder(directory, folder))));
  const lint = printOnly === undefined ? linted(directory, probes) : {};
  if (printed.includes(undefined) || lint === undefined) return "error";
  return Object.fromEntries(probes.map((folder, index) => [folder, folderRecord(directory, folder, printed[index], lint[folder])]));
}

function main() {
  const { probes, cases } = JSON.parse(readFileSync(join(HERE, "corpus.json"), "utf8"));
  const root = join(tools, "cases");
  rmSync(root, { recursive: true, force: true });
  const expected = { oxlint: PIN, keys: keysOf(portedNames(), join(root, "_keys")), typeAware: typeAware(), cases: {} };
  for (const { name, files, printOnly } of cases) {
    const directory = join(root, name);
    const probeFiles = Object.fromEntries(probes.map(folder => [join(folder, "probe.js"), `${Object.values(CHECKED).join("\n")}\n`]));
    for (const [path, text] of Object.entries({ ...probeFiles, ...files })) {
      mkdirSync(dirname(join(directory, path)), { recursive: true });
      writeFileSync(join(directory, path), text);
    }
    expected.cases[name] = caseRules(directory, probes, printOnly);
  }
  rmSync(root, { recursive: true, force: true });
  writeFileSync(join(HERE, "oxlint.expected.json"), `${JSON.stringify(expected, undefined, 1)}\n`);
}

main();
