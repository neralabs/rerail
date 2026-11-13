// Writes src/engine/catalog/lint-config/generated/ruff-rules.generated.ts: what set-up needs to replay the pinned Ruff's
// rule selection over a config it reads as data. Each rule's name, status (those of stable, preview, deprecated and
// removed the pinned Ruff has, the status type naming only those) and category, and Ruff's default rule selection with
// and without preview (`ruff rule --all --output-format json`, and `ruff check --isolated --show-settings` in an empty
// folder, with and without `--preview`); the linters' code prefixes (`ruff linter --output-format json`), which decide
// what a selector names; the D rules each `lint.pydocstyle.convention` turns off (as `--show-settings` reports them);
// and, from ruff_linter's source at the commit packs/python/upstream/ruff/gates.yaml pins, the selectors Ruff remaps
// before selecting (`REDIRECTS` in `rule_redirects.rs`, less its test-only entries) and the rules it never runs together
// (`INCOMPATIBLE_CODES` in `registry.rs`), each checked against the binary: selecting the old selector enables what
// selecting its target does, and selecting both of a pair enables only the first. Run offline, not in CI:
//   node scripts/generate/lint-config/ruff-rules.mjs <ruff 0.16.9>
// (`uvx ruff@0.16.9`'s binary, or any ruff 0.16.9); it fetches those two source files from GitHub at the pinned commit.
// A different Ruff version, or a gates.yaml that pins another, is refused; bump PIN and regenerate.
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const PIN = "0.16.9";
/** The gated tool-pack that pins Ruff (`<pack>/<tool>`, packs/<pack>/upstream/<tool>/gates.yaml). */
const GATE = "python/ruff";
const STATUS = { Stable: "stable", Preview: "preview", Deprecated: "deprecated", Removed: "removed" };
const BUFFER = 64 * 1024 * 1024;

const ruff = process.argv[2];
if (ruff === undefined) throw new Error("usage: node scripts/generate/lint-config/ruff-rules.mjs <ruff binary>");
const version = execFileSync(ruff, ["version"], { encoding: "utf8" }).trim();
if (!version.startsWith(`ruff ${PIN} `)) throw new Error(`${version} found; the table is pinned to ruff ${PIN}`);
const gates = readFileSync(join(import.meta.dirname, `../../../packs/${GATE.split("/")[0]}/upstream/${GATE.split("/")[1]}/gates.yaml`), "utf8");
const gated = /^ {2}version: (\S+)$/mu.exec(gates)?.[1];
if (gated !== PIN) throw new Error(`${GATE}'s gates.yaml pins ruff ${gated}, not ${PIN}`);
const commit = /^ {2}commit: ([0-9a-f]{40})$/mu.exec(gates)?.[1];
if (commit === undefined) throw new Error(`${GATE}'s gates.yaml pins no commit`);

const rules = JSON.parse(execFileSync(ruff, ["rule", "--all", "--output-format", "json"], { encoding: "utf8", maxBuffer: BUFFER }));
const linters = JSON.parse(execFileSync(ruff, ["linter", "--output-format", "json"], { encoding: "utf8", maxBuffer: BUFFER }));

const folder = mkdtempSync(join(tmpdir(), "rerail-ruff-rules-"));
writeFileSync(join(folder, "a.py"), "");

/** The codes Ruff enables with every option at its default but `flags`, in an empty folder; undefined where Ruff refuses them. */
function enabledCodes(flags) {
  const run = spawnSync(ruff, ["check", "--isolated", ...flags, "--show-settings", "a.py"], { cwd: folder, encoding: "utf8", maxBuffer: BUFFER });
  if (run.status !== 0) return undefined;
  const enabled = /^linter\.rules\.enabled = \[(?:\]|\n([\s\S]*?)^\])/mu.exec(run.stdout);
  if (enabled === null) throw new Error("ruff --show-settings printed no linter.rules.enabled");
  return [...(enabled[1] ?? "").matchAll(/\(([A-Z]+[0-9]+)\),$/gmu)].map(match => match[1]).sort().join(" ");
}

const stable = new Set(enabledCodes([]).split(" "));
const preview = new Set(enabledCodes(["--preview"]).split(" "));
// A rule without a code (selected by name, in preview only) has no noqa code a port could name.
const rows = rules.filter(rule => rule.code !== null).map(rule => {
  const status = STATUS[Object.keys(rule.status)[0]];
  if (status === undefined) throw new Error(`${rule.code}: unknown status ${JSON.stringify(rule.status)}`);
  return [rule.code, rule.name, status, rule.category, stable.has(rule.code), preview.has(rule.code)];
}).sort(([left], [right]) => left.localeCompare(right));
for (const code of [...stable, ...preview]) if (!rows.some(([known]) => known === code)) throw new Error(`${code} is enabled by default but not listed`);
// The reader takes a selector equal to a code as selecting that rule alone, as Ruff does when no other code extends it.
for (const [code] of rows) if (rows.some(([other]) => other !== code && other.startsWith(code))) throw new Error(`${code} is a prefix of another code`);
// The status type names only the statuses the pin has, so that one a bump brings fails type-checking until handled.
const statuses = Object.values(STATUS).filter(status => rows.some(row => row[2] === status));
const namesWithoutCode = rules.filter(rule => rule.code === null).map(rule => rule.name).sort();
const categories = [...new Set(rules.map(rule => rule.category))].sort();
// pycodestyle has no common prefix: its codes start with `E` or `W`, each its own prefix.
const prefixes = linters.flatMap(linter => (linter.prefix === "" ? linter.categories.map(category => category.prefix) : [linter.prefix])).sort();

/** A file of Ruff's source at the pinned commit. */
const ruffSource = async path => (await fetch(`https://raw.githubusercontent.com/astral-sh/ruff/${commit}/crates/ruff_linter/src/${path}`)).text();

const table = /static REDIRECTS[\s\S]*?HashMap::from_iter\(\[([\s\S]*?)\]\)/u.exec(await ruffSource("rule_redirects.rs"))?.[1];
if (table === undefined) throw new Error("rule_redirects.rs holds no REDIRECTS table");
// An entry under `#[cfg(...)]` exists only in Ruff's own test builds.
const redirects = [...table.replaceAll(/#\[cfg\([^\n]*\n\s*\("[^"]*",\s*"[^"]*"\),/gu, "").matchAll(/\("([^"]+)",\s*"([^"]+)"\)/gu)]
  .map(([, from, to]) => [from, to])
  .sort(([left], [right]) => left.localeCompare(right));
for (const [from, to] of redirects) {
  for (const mode of [[], ["--preview"]]) {
    const [old, target] = [enabledCodes([...mode, "--select", from]), enabledCodes([...mode, "--select", to])];
    if (old !== target) throw new Error(`${mode.join(" ")} --select ${from} enables ${old ?? "(refused)"}, --select ${to} ${target ?? "(refused)"}`);
  }
}

// `INCOMPATIBLE_CODES` in ruff_linter's `registry.rs`: where both are on, Ruff turns the second off. Rules are named by
// their variant there (`BlankLineBeforeClass`), which is the rule's name in kebab case.
const codeByName = new Map(rows.map(([code, name]) => [name, code]));
const codeOfVariant = variant => codeByName.get(variant.replaceAll(/(?<=[a-z0-9])(?=[A-Z])/gu, "-").toLowerCase());
const pairs = /INCOMPATIBLE_CODES[^=]*=\s*&\[([\s\S]*?)\n\];/u.exec(await ruffSource("registry.rs"))?.[1];
if (pairs === undefined) throw new Error("registry.rs holds no INCOMPATIBLE_CODES");
const incompatible = [...pairs.matchAll(/Rule::(\w+),\s*Rule::(\w+),/gu)].map(([, preferred, expendable]) => [codeOfVariant(preferred), codeOfVariant(expendable)]);
for (const [preferred, expendable] of incompatible) {
  if (preferred === undefined || expendable === undefined) throw new Error(`INCOMPATIBLE_CODES names a rule without a code: ${pairs}`);
  if (enabledCodes(["--select", `${preferred},${expendable}`]) !== preferred) throw new Error(`${preferred} does not turn ${expendable} off`);
}

// `lint.pydocstyle.convention`: the D rules each convention turns off unless selected by their code, as Ruff reports
// them, every D rule selected (preview's too), less one of each incompatible pair at a time.
const D_PAIRS = incompatible.filter(pair => pair.every(code => code.startsWith("D")));
const conventionOff = (convention, ignored) => {
  const flags = ["--preview", "--select", "D", "--ignore", ignored.join(",")];
  const all = enabledCodes(flags).split(" ");
  const kept = new Set(enabledCodes([...flags, "--config", `lint.pydocstyle.convention = "${convention}"`]).split(" "));
  return all.filter(code => !kept.has(code));
};
const conventions = Object.fromEntries(["google", "numpy", "pep257"].map(convention => [convention, [...new Set([
  ...conventionOff(convention, D_PAIRS.map(([preferred]) => preferred)),
  ...conventionOff(convention, D_PAIRS.map(([, expendable]) => expendable)),
])].sort()]));
rmSync(folder, { recursive: true, force: true });

const text = `// Generated by scripts/generate/lint-config/ruff-rules.mjs; do not edit.
// From ruff ${PIN}: \`ruff rule --all --output-format json\`, \`ruff linter --output-format json\`,
// \`ruff check --isolated [--preview] [--config lint.pydocstyle.convention=…] --show-settings\`, and, at ${commit},
// \`REDIRECTS\` in crates/ruff_linter/src/rule_redirects.rs and \`INCOMPATIBLE_CODES\` in crates/ruff_linter/src/registry.rs.

/** The Ruff release these tables are made from, by the gated tool-pack that pins it (\`<pack>/<tool>\`). */
export const UPSTREAM_VERSIONS: Readonly<Record<string, string>> = ${JSON.stringify({ [GATE]: PIN })};

/** A Ruff rule: its name, status, category, and whether Ruff's default selection enables it without and with preview. */
export interface RuffRuleInfo {
  readonly name: string;
  readonly status: ${statuses.map(status => JSON.stringify(status)).join(" | ")};
  readonly category: string;
  readonly byDefault: boolean;
  readonly byPreviewDefault: boolean;
}

const ROWS: readonly (readonly [string, string, RuffRuleInfo["status"], string, boolean, boolean])[] = [
${rows.map(row => `  ${JSON.stringify(row)},`).join("\n")}
];

/** Every rule of Ruff ${PIN} with a code, by its code. */
export const RUFF_RULES: ReadonlyMap<string, RuffRuleInfo> = new Map(ROWS.map(([code, name, status, category, byDefault, byPreviewDefault]) => [code, { name, status, category, byDefault, byPreviewDefault }]));

/** The names of Ruff's rules without a code, which only a selector by name (with preview) selects. */
export const RUFF_NAMES_WITHOUT_CODE: readonly string[] = ${JSON.stringify(namesWithoutCode)};

/** Ruff's rule categories (\`Category\` in ruff_linter's \`codes.rs\`). */
export const RUFF_CATEGORIES: readonly string[] = ${JSON.stringify(categories)};

/** Each linter's code prefix (\`E\` and \`W\` for pycodestyle, \`PL\` for Pylint); a code belongs to the longest that starts it. */
export const RUFF_LINTER_PREFIXES: readonly string[] = ${JSON.stringify(prefixes)};

/** The selectors Ruff remaps before selecting, to the selector it reads instead (\`TCH\` is \`TC\`, \`TRY200\` is \`B904\`). */
export const RUFF_REDIRECTS: ReadonlyMap<string, string> = new Map(${JSON.stringify(redirects, undefined, 0).replaceAll("],[", "],\n  [").replace(/^\[\[/u, "[\n  [").replace(/\]\]$/u, "],\n]")});

/** Per \`lint.pydocstyle.convention\`, the rules it turns off unless a config selects them by their code. */
export const RUFF_CONVENTIONS: Readonly<Record<string, readonly string[]>> = ${JSON.stringify(conventions)};

/** Pairs of rules Ruff never runs together: where both are on, it turns the second off. */
export const RUFF_INCOMPATIBLE: readonly (readonly [string, string])[] = ${JSON.stringify(incompatible)};
`;
writeFileSync(join(import.meta.dirname, "../../../src/engine/catalog/lint-config/generated/ruff-rules.generated.ts"), text);
