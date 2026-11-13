// Records biome.expected.json: for each config of corpus.json, the lint rules Biome 2.5.15 runs in each probe folder,
// as the oracle of set-up's reading of Biome configs (src/engine/catalog/lint-config/biome-config.ts). Run by hand,
// not in CI, against the pinned tool:
//   npm install --prefix <tools> @biomejs/biome@2.5.15
//   node fixtures/host/lint-config/biome/generate_biome.mjs <tools>
// Any other installed version is refused.
//
// Each case's files are written to <tools>/cases/<name>/ with a probe.js in every probe folder, holding one line that
// breaks each rule of `checked`, and read there. `biome lint --reporter=json` over the case gives the checked rules
// Biome reports in each folder; a case it refuses (no report) is recorded as "error". Where Biome takes one config for
// every file (no nested config, no `overrides`, no package.json, whose dependencies turn domains on per file) and the
// config sets no `domains` (whose recommended rules rage leaves out of its list), each folder's record is every rule
// `biome rage --linter` lists as enabled (none where it reports the linter disabled), after checking the lint run
// reports exactly the checked rules of that list; elsewhere it is `{ checked, on }`, the
// checked rules and those the lint run reports. Rules are named `<group>/<rule>`, sorted and space-joined.
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";

const HERE = import.meta.dirname;
const PIN = "2.5.15";
const NESTED_FILES = ["biome.json", "biome.jsonc"];

const tools = process.argv[2];
if (tools === undefined) throw new Error("usage: node fixtures/host/lint-config/biome/generate_biome.mjs <tools>");
const BIOME = join(tools, "node_modules", ".bin", "biome");
const version = execFileSync(BIOME, ["--version"], { encoding: "utf8" }).trim();
if (version !== `Version: ${PIN}`) throw new Error(`${version} found; the oracle is pinned to Biome ${PIN}`);

/**
 * The rules `biome rage --linter` in `directory` lists as enabled, none where it reports the linter disabled;
 * undefined where it loaded no config file in the case (a config that is not the root one).
 */
function rageRules(directory) {
  const text = execFileSync(BIOME, ["rage", "--linter"], { cwd: directory, encoding: "utf8", env: { ...process.env, NO_COLOR: "1" } });
  if (!/^\s*Status:\s+Loaded successfully\.$/mu.test(text)) return undefined;
  if (/^\s*Linter enabled:\s+false$/mu.test(text)) return [];
  const listed = text.slice(text.indexOf("Enabled rules:"));
  return [...listed.matchAll(/^ {4}(\w+\/\w+)$/gmu)].map(match => match[1]).sort();
}

/** Per probe folder, the checked rules a lint run over the case reports; undefined when Biome refuses the case. */
function linted(directory, probes) {
  const run = spawnSync(BIOME, ["lint", "--reporter=json", "--max-diagnostics=none", "."], { cwd: directory, encoding: "utf8" });
  const report = run.stdout.split("\n").find(line => line.startsWith("{\"summary\""));
  if (report === undefined) return undefined;
  const found = Object.fromEntries(probes.map(folder => [folder, new Set()]));
  for (const diagnostic of JSON.parse(report).diagnostics) {
    const folder = dirname(diagnostic.location?.path ?? "");
    if (diagnostic.category.startsWith("lint/") && basename(diagnostic.location.path) === "probe.js") found[folder === "." ? "" : folder].add(diagnostic.category.slice("lint/".length));
  }
  return found;
}

/**
 * Whether rage, run at the root, may not show what Biome runs: folders linted by different configs or rules, or
 * domains, whose recommended rules rage leaves out.
 */
const beyondRage = files => Object.keys(files).some(path => (path.includes("/") && NESTED_FILES.includes(basename(path))) || basename(path) === "package.json")
  || Object.values(files).some(text => text.includes("\"overrides\"") || text.includes("\"domains\""));

/** The record of every probe folder: the rules rage lists, checked against the lint run. */
function wholeRecord(directory, probes, checked, rules, lint) {
  for (const folder of probes) {
    const disagreeing = checked.filter(rule => rules.includes(rule) !== lint[folder].has(rule));
    if (disagreeing.length > 0) throw new Error(`${directory} ${folder}: rage and the lint run disagree on ${disagreeing.join(", ")}`);
  }
  return Object.fromEntries(probes.map(folder => [folder, rules.join(" ")]));
}

/** What Biome runs in each probe folder of the case in `directory`, or "error". */
function caseRules(directory, probes, checked, files) {
  const lint = linted(directory, probes);
  if (lint === undefined) return "error";
  const rules = beyondRage(files) ? undefined : rageRules(directory);
  if (rules !== undefined) return wholeRecord(directory, probes, checked, rules, lint);
  return Object.fromEntries(probes.map(folder => [folder, { checked: checked.join(" "), on: [...lint[folder]].filter(rule => checked.includes(rule)).sort().join(" ") }]));
}

function main() {
  const corpus = JSON.parse(readFileSync(join(HERE, "corpus.json"), "utf8"));
  const checked = Object.keys(corpus.checked).sort();
  const root = join(tools, "cases");
  rmSync(root, { recursive: true, force: true });
  const expected = { biome: PIN, cases: {} };
  const probe = `${Object.values(corpus.checked).join("\n")}\n`;
  for (const { name, files } of corpus.cases) {
    const directory = join(root, name);
    const probeFiles = Object.fromEntries(corpus.probes.map(folder => [join(folder, "probe.js"), probe]));
    for (const [path, text] of Object.entries({ ...probeFiles, ...files })) {
      mkdirSync(dirname(join(directory, path)), { recursive: true });
      writeFileSync(join(directory, path), text);
    }
    expected.cases[name] = caseRules(directory, corpus.probes, checked, files);
  }
  rmSync(root, { recursive: true, force: true });
  writeFileSync(join(HERE, "biome.expected.json"), `${JSON.stringify(expected, undefined, 1)}\n`);
}

main();
