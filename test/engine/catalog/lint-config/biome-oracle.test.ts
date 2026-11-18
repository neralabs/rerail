import { mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { biomeReader } from "../../../../src/engine/catalog/lint-config/biome-config.js";
import { BIOME_RULES } from "../../../../src/engine/catalog/lint-config/generated/biome-rules.generated.js";
import { portsOf, settleVerdicts, type FolderDecision, type LinterNote, type Port, type PortDecision, type PortVerdict } from "../../../../src/engine/catalog/lint-config/linter-config.js";
import type { RuleCatalog } from "../../../../src/engine/catalog/repository-model.js";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import { fixturesPath } from "../../../support/oracles.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

/**
 * The reader against Biome itself (fixtures/host/lint-config/biome/: what Biome 2.5.15 runs in each probe folder of
 * each config in corpus.json, recorded by generate_biome.mjs from `biome rage --linter` and a lint run). Where the
 * reader decides a port's Biome rule on or off in a probe folder, Biome must agree on every probe file there, and the
 * reader's verdict settled alone must give that state whatever the pack's value; where Biome decides the probe files
 * of a folder otherwise, the reader must leave the rule unknown, and elsewhere it may only where the case says Rerail
 * cannot read it. A config Biome refuses is one the reader reads nothing of, unless Biome refuses it only for want of
 * an extended config the case leaves out (`unresolved`).
 */
interface OracleCase {
  readonly name: string;
  readonly files: Readonly<Record<string, string>>;
  /** The probe files in every probe folder; `probe.js` alone when absent. */
  readonly probeNames?: readonly string[];
  /**
   * Per probe folder (`*` for every folder), the rules (`group/rule`; `*` for every rule; `@domains` for a rule a
   * domain holds, which a package.json may turn on) the reader may leave unknown.
   */
  readonly unknown?: Readonly<Record<string, readonly string[]>>;
  /** Per probe folder, rules the reader decides there, by whether the config names them (explicit) or not. */
  readonly explicit?: Readonly<Record<string, Readonly<Record<string, boolean>>>>;
  readonly unresolved?: string;
}

/**
 * A folder's record: the rules on, space-joined; or, where rage cannot tell, the rules a lint run checked and those it
 * found on, or those it found on per probe file.
 */
type FolderRecord = string | { readonly checked: string; readonly on: string } | { readonly checked: string; readonly files: Readonly<Record<string, string>> };

const ORACLE = fixturesPath("host", "lint-config", "biome");
const corpus = JSON.parse(readFileSync(join(ORACLE, "corpus.json"), "utf8")) as { probes: string[]; checked: Record<string, string>; cases: OracleCase[] };
const expected = JSON.parse(readFileSync(join(ORACLE, "biome.expected.json"), "utf8")) as { biome: string; cases: Record<string, "error" | Record<string, FolderRecord>> };
const PACKS = ["javascript", "typescript", "react", "node", "next", "vue"];

let ruleCatalog: RuleCatalog;
/** Each port with a Biome name, with that name as `group/rule`. */
let ports: { port: Port; rule: string }[];
beforeAll(() => {
  ruleCatalog = readRuleCatalog(PACKS.map(pack => fileURLToPath(new URL(`../../../../packs/${pack}`, import.meta.url))));
  ports = portsOf(ruleCatalog, PACKS).flatMap(port => port.suppress.biome.map(name => ({ port, rule: name.slice("lint/".length) })));
});

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

async function repository(files: Readonly<Record<string, string>>): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-biome-oracle-")));
  temporaryDirectories.push(root);
  for (const [name, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, name)), { recursive: true });
    writeFileSync(join(root, name), text);
  }
  return root;
}

/** The verdict's decision in `folder`: the deepest folder's holding it, else the one for every file. */
function decisionIn(verdict: PortVerdict | undefined, folder: string): PortDecision | FolderDecision | undefined {
  const holding = (verdict?.folders ?? []).filter(decision => folder === decision.folder || folder.startsWith(`${decision.folder}/`));
  return holding.sort((left, right) => right.folder.length - left.folder.length)[0] ?? verdict?.decision;
}

/** The port's state in `folder` once its verdict alone settles, the port starting `startsOn`. */
function settledIn(port: Port, verdict: PortVerdict | undefined, folder: string, startsOn: boolean): boolean {
  const { settings } = settleVerdicts({ ...port, startsOn }, verdict === undefined ? [] : [verdict], []);
  const holding = settings.filter(setting => setting.folder === "" || folder === setting.folder || folder.startsWith(`${setting.folder}/`));
  return holding.sort((left, right) => right.folder.length - left.folder.length)[0]?.on ?? startsOn;
}

const inDomain = (rule: string): boolean => (BIOME_RULES[rule] ?? []).some(([, domains]) => domains.length > 0);

const mayBeUnknown = (oracleCase: OracleCase, folder: string, rule: string): boolean =>
  [oracleCase.unknown?.[folder] ?? [], oracleCase.unknown?.["*"] ?? []]
    .some(allowed => allowed.includes("*") || allowed.includes(rule) || (allowed.includes("@domains") && inDomain(rule)));

/** Whether Biome runs `rule` on each probe file of the folder `record` is for; undefined where the record does not tell. */
function biomeRuns(record: FolderRecord, rule: string): boolean[] | undefined {
  if (typeof record === "string") return [record.split(" ").includes(rule)];
  if (!record.checked.split(" ").includes(rule)) return undefined;
  return "on" in record ? [record.on.split(" ").includes(rule)] : Object.values(record.files).map(on => on.split(" ").includes(rule));
}

/**
 * Where the reader's verdict on `port` disagrees in `folder` with Biome running `rule` on each probe file (`runs`): its
 * state, its explicitness where the case states it, and its state settled alone, the port starting on or off (a
 * held-off port's turned on taking the pack's value).
 */
function folderDisagreements(oracleCase: OracleCase, port: Port, rule: string, verdict: PortVerdict | undefined, folder: string, runs: readonly boolean[]): string[] {
  const where = `${folder || "."} ${rule}`;
  const decision = decisionIn(verdict, folder);
  const state = decision?.on ?? "unknown";
  const mixed = new Set(runs).size > 1;
  const agrees = state === "unknown" ? mixed || mayBeUnknown(oracleCase, folder, rule) : !mixed && state === runs[0];
  const found = agrees ? [] : [`${where}: Rerail ${String(state)}, Biome ${runs.map(on => (on ? "on" : "off")).join("/")}`];
  const explicit = oracleCase.explicit?.[folder]?.[rule];
  const decidedExplicit = decision === undefined || decision.on === "unknown" ? undefined : decision.explicit;
  if (explicit !== undefined && decidedExplicit !== explicit) found.push(`${where}: explicit ${String(decidedExplicit)}, the case says ${String(explicit)}`);
  for (const startsOn of [true, false]) {
    const settled = settledIn(port, verdict, folder, startsOn);
    const wanted = state === "unknown" || (state && port.heldOff !== undefined) ? startsOn : state;
    if (settled !== wanted) found.push(`${where}: settles ${String(settled)} from ${String(startsOn)}, not ${String(wanted)}`);
  }
  return found;
}

/** Where the reader's verdicts in the case disagree with Biome's `records`. */
function disagreements(oracleCase: OracleCase, records: Readonly<Record<string, FolderRecord>>, verdictOf: (port: Port) => PortVerdict | undefined): string[] {
  const found: string[] = [];
  for (const { port, rule } of ports) {
    const verdict = verdictOf(port);
    for (const folder of corpus.probes) {
      const runs = biomeRuns(records[folder]!, rule);
      if (runs !== undefined) found.push(...folderDisagreements(oracleCase, port, rule, verdict, folder, runs));
    }
  }
  return found;
}

describe("the Biome reader against Biome 2.5.15 (fixtures/host/lint-config/biome)", () => {
  it("records every case of the corpus, and checks ported rules in every folder", () => {
    expect(expected.biome).toBe("2.5.15");
    expect(Object.keys(expected.cases).sort()).toEqual(corpus.cases.map(oracleCase => oracleCase.name).sort());
    const ported = new Set(ports.map(({ rule }) => rule));
    expect(Object.keys(corpus.checked).filter(rule => !ported.has(rule))).toEqual([]);
  });

  it.each(corpus.cases.map(oracleCase => [oracleCase.name, oracleCase] as const))("decides as Biome does: %s", async (_, oracleCase) => {
    const probes = corpus.probes.flatMap(folder => (oracleCase.probeNames ?? ["probe.js"]).map(name => [folder === "" ? name : `${folder}/${name}`, ""]));
    const files = { ...Object.fromEntries(probes), ...oracleCase.files };
    const config = biomeReader.read(await repository(files), [] as LinterNote[]);
    const records = expected.cases[oracleCase.name]!;
    if (records === "error") {
      expect(oracleCase.unresolved !== undefined || config === undefined).toBe(true);
      return;
    }
    expect(disagreements(oracleCase, records, port => config?.verdict(port, ruleCatalog))).toEqual([]);
  });
});
