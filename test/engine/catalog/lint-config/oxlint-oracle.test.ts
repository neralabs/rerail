import { mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { portsOf, type LinterNote, type Port, type PortVerdict } from "../../../../src/engine/catalog/lint-config/linter-config.js";
import { oxlintReader } from "../../../../src/engine/catalog/lint-config/oxlint-config.js";
import type { RuleCatalog } from "../../../../src/engine/catalog/repository-model.js";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import { fixturesPath } from "../../../support/oracles.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

/**
 * The reader against oxlint itself (fixtures/host/lint-config/oxlint/: oxlint 1.88.0's `--print-config` in each probe
 * folder of each config in corpus.json, recorded by generate_oxlint.mjs). Where the reader decides a port's oxlint rule
 * on or off in a probe folder, oxlint must agree; it may leave the rule unknown only where the case says Rerail cannot
 * read it, or for a type-aware rule, which only `--type-aware` runs. A config oxlint refuses is one the reader reads
 * nothing of.
 */
interface OracleCase {
  readonly name: string;
  readonly files: Readonly<Record<string, string>>;
  /** Per probe folder (`*` for every folder), the rules (`*` for every rule) the reader may leave unknown. */
  readonly unknown?: Readonly<Record<string, readonly string[]>>;
  /** Whether the config sets `options.typeAware`, which runs the type-aware rules. */
  readonly typeAware?: boolean;
}

/** A folder's record: the rules on, space-joined; or, where --print-config cannot tell (overrides), the rules a lint run checked and those it found on. */
type FolderRecord = string | { readonly checked: string; readonly on: string };

interface Expected {
  readonly oxlint: string;
  /** Per ESLint name a port gives, the rule oxlint configures for it, by the name --print-config gives it. */
  readonly keys: Readonly<Record<string, string | null>>;
  readonly typeAware: readonly string[];
  readonly cases: Readonly<Record<string, "error" | Readonly<Record<string, FolderRecord>>>>;
}

const ORACLE = fixturesPath("host", "lint-config", "oxlint");
const corpus = JSON.parse(readFileSync(join(ORACLE, "corpus.json"), "utf8")) as { probes: string[]; cases: OracleCase[] };
const expected = JSON.parse(readFileSync(join(ORACLE, "oxlint.expected.json"), "utf8")) as Expected;
const PACKS = ["javascript", "typescript", "react", "node", "next", "vue"];

let ruleCatalog: RuleCatalog;
/** Each port with an oxlint name, with the oxlint rules it models by oxlint's own reading of its ESLint names. */
let ports: { port: Port; rules: string[] }[];
beforeAll(() => {
  ruleCatalog = readRuleCatalog(PACKS.map(pack => fileURLToPath(new URL(`../../../../packs/${pack}`, import.meta.url))));
  ports = portsOf(ruleCatalog, PACKS).filter(port => port.suppress.oxlint.length > 0).map(port => ({
    port,
    rules: [...new Set(port.suppress.eslint.flatMap(name => expected.keys[name] ?? []))]
      .filter(rule => port.suppress.oxlint.includes(rule.slice(rule.lastIndexOf("/") + 1))),
  }));
});

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

async function repository(files: Readonly<Record<string, string>>): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-oxlint-oracle-")));
  temporaryDirectories.push(root);
  for (const [name, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, name)), { recursive: true });
    writeFileSync(join(root, name), text);
  }
  return root;
}

/** The verdict's state in `folder`: the decision of the deepest folder holding it, else the one for every file. */
function stateIn(verdict: PortVerdict | undefined, folder: string): boolean | "unknown" {
  const holding = (verdict?.folders ?? []).filter(decision => folder === decision.folder || folder.startsWith(`${decision.folder}/`));
  const deepest = holding.sort((left, right) => right.folder.length - left.folder.length)[0];
  return deepest?.on ?? verdict?.decision?.on ?? "unknown";
}

const mayBeUnknown = (oracleCase: OracleCase, folder: string, rules: readonly string[]): boolean =>
  rules.every(rule => expected.typeAware.includes(rule))
  || [oracleCase.unknown?.[folder] ?? [], oracleCase.unknown?.["*"] ?? []].some(allowed => allowed.includes("*") || rules.some(rule => allowed.includes(rule)));

/** Whether oxlint runs any of `rules` per `record`; undefined where the record does not tell. */
function oxlintRuns(record: FolderRecord, rules: readonly string[]): boolean | undefined {
  if (typeof record === "string") return rules.some(rule => record.split(" ").includes(rule));
  if (!rules.every(rule => record.checked.split(" ").includes(rule))) return undefined;
  return rules.some(rule => record.on.split(" ").includes(rule));
}

/**
 * Whether the reader's `state` agrees with oxlint's `runs`. oxlint runs a type-aware rule only with --type-aware (or
 * `options.typeAware`), where it runs as --print-config says: the reader may decide it only off where it is off both ways.
 */
function agrees(oracleCase: OracleCase, folder: string, rules: readonly string[], state: boolean | "unknown", runs: boolean): boolean {
  if (state === "unknown") return mayBeUnknown(oracleCase, folder, rules);
  const typeAwareOnly = oracleCase.typeAware !== true && rules.every(rule => expected.typeAware.includes(rule));
  return state === runs && !(typeAwareOnly && state);
}

/** Where the reader's verdicts in the case disagree with oxlint's `records`. */
function disagreements(oracleCase: OracleCase, records: Readonly<Record<string, FolderRecord>>, verdictOf: (port: Port) => PortVerdict | undefined): string[] {
  const found: string[] = [];
  for (const { port, rules } of ports) {
    const verdict = verdictOf(port);
    for (const folder of corpus.probes) {
      const runs = oxlintRuns(records[folder]!, rules);
      const state = stateIn(verdict, folder);
      if (runs === undefined || agrees(oracleCase, folder, rules, state, runs)) continue;
      found.push(`${folder || "."} ${rules.join(",")}: Rerail ${String(state)}, oxlint ${runs ? "on" : "off"}`);
    }
  }
  return found;
}

describe("the oxlint reader against oxlint 1.88.0 (fixtures/host/lint-config/oxlint)", () => {
  it("records every case of the corpus, and an oxlint rule for every ESLint name of a port", () => {
    expect(expected.oxlint).toBe("1.88.0");
    expect(Object.keys(expected.cases).sort()).toEqual(corpus.cases.map(oracleCase => oracleCase.name).sort());
    expect(ports.filter(ported => ported.rules.length === 0).map(ported => ported.port.ruleId)).toEqual([]);
  });

  it("models each port's rule as the one oxlint configures for the port's ESLint names", async () => {
    // Every plugin run, every category off: a port is on exactly where the config names its rule on.
    const plugins = ["react", "unicorn", "typescript", "oxc", "import", "jsdoc", "jest", "vitest", "jsx-a11y", "nextjs", "react-perf", "promise", "node", "vue"];
    const categories = Object.fromEntries(["correctness", "suspicious", "pedantic", "perf", "style", "restriction", "nursery"].map(category => [category, "off"]));
    const rules = Object.fromEntries(ports.flatMap(ported => ported.rules).map(rule => [rule, "error"]));
    const config = oxlintReader.read(await repository({ ".oxlintrc.json": JSON.stringify({ plugins, categories, options: { typeAware: true }, rules }) }), [])!;
    const wrong = ports.filter(ported => config.verdict(ported.port, ruleCatalog)?.decision?.on !== true).map(ported => ported.port.ruleId);
    expect(wrong).toEqual([]);
  });

  it.each(corpus.cases.map(oracleCase => [oracleCase.name, oracleCase] as const))("decides as oxlint does: %s", async (_, oracleCase) => {
    const files = { ...Object.fromEntries(corpus.probes.map(folder => [folder === "" ? "probe.js" : `${folder}/probe.js`, ""])), ...oracleCase.files };
    const config = oxlintReader.read(await repository(files), [] as LinterNote[]);
    const records = expected.cases[oracleCase.name]!;
    if (records === "error") {
      expect(config).toBeUndefined();
      return;
    }
    expect(disagreements(oracleCase, records, port => config?.verdict(port, ruleCatalog))).toEqual([]);
  });
});
