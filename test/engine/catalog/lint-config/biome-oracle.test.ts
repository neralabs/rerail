import { mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { biomeReader } from "../../../../src/engine/catalog/lint-config/biome-config.js";
import { BIOME_RULES } from "../../../../src/engine/catalog/lint-config/generated/biome-rules.generated.js";
import { portsOf, type LinterNote, type Port, type PortVerdict } from "../../../../src/engine/catalog/lint-config/linter-config.js";
import type { RuleCatalog } from "../../../../src/engine/catalog/repository-model.js";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import { fixturesPath } from "../../../support/oracles.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

/**
 * The reader against Biome itself (fixtures/host/lint-config/biome/: what Biome 2.5.15 runs in each probe folder of
 * each config in corpus.json, recorded by generate_biome.mjs from `biome rage --linter` and a lint run). Where the
 * reader decides a port's Biome rule on or off in a probe folder, Biome must agree; it may leave the rule unknown only
 * where the case says Rerail cannot read it. A config Biome refuses is one the reader reads nothing of, unless Biome
 * refuses it only for want of an extended config the case leaves out (`unresolved`).
 */
interface OracleCase {
  readonly name: string;
  readonly files: Readonly<Record<string, string>>;
  /**
   * Per probe folder (`*` for every folder), the rules (`group/rule`; `*` for every rule; `@domains` for a rule a
   * domain holds, which a package.json may turn on) the reader may leave unknown.
   */
  readonly unknown?: Readonly<Record<string, readonly string[]>>;
  readonly unresolved?: string;
}

/** A folder's record: the rules on, space-joined; or, where rage cannot tell, the rules a lint run checked and those it found on. */
type FolderRecord = string | { readonly checked: string; readonly on: string };

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

/** The verdict's state in `folder`: the decision of the deepest folder holding it, else the one for every file. */
function stateIn(verdict: PortVerdict | undefined, folder: string): boolean | "unknown" {
  const holding = (verdict?.folders ?? []).filter(decision => folder === decision.folder || folder.startsWith(`${decision.folder}/`));
  const deepest = holding.sort((left, right) => right.folder.length - left.folder.length)[0];
  return deepest?.on ?? verdict?.decision?.on ?? "unknown";
}

const inDomain = (rule: string): boolean => (BIOME_RULES[rule] ?? []).some(([, domains]) => domains.length > 0);

const mayBeUnknown = (oracleCase: OracleCase, folder: string, rule: string): boolean =>
  [oracleCase.unknown?.[folder] ?? [], oracleCase.unknown?.["*"] ?? []]
    .some(allowed => allowed.includes("*") || allowed.includes(rule) || (allowed.includes("@domains") && inDomain(rule)));

/** Whether Biome runs `rule` per `record`; undefined where the record does not tell. */
function biomeRuns(record: FolderRecord, rule: string): boolean | undefined {
  if (typeof record === "string") return record.split(" ").includes(rule);
  return record.checked.split(" ").includes(rule) ? record.on.split(" ").includes(rule) : undefined;
}

/** Where the reader's verdicts in the case disagree with Biome's `records`. */
function disagreements(oracleCase: OracleCase, records: Readonly<Record<string, FolderRecord>>, verdictOf: (port: Port) => PortVerdict | undefined): string[] {
  const found: string[] = [];
  for (const { port, rule } of ports) {
    const verdict = verdictOf(port);
    for (const folder of corpus.probes) {
      const runs = biomeRuns(records[folder]!, rule);
      const state = stateIn(verdict, folder);
      if (runs === undefined || (state === "unknown" ? mayBeUnknown(oracleCase, folder, rule) : state === runs)) continue;
      found.push(`${folder || "."} ${rule}: Rerail ${String(state)}, Biome ${runs ? "on" : "off"}`);
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
    const files = { ...Object.fromEntries(corpus.probes.map(folder => [folder === "" ? "probe.js" : `${folder}/probe.js`, ""])), ...oracleCase.files };
    const config = biomeReader.read(await repository(files), [] as LinterNote[]);
    const records = expected.cases[oracleCase.name]!;
    if (records === "error") {
      expect(oracleCase.unresolved !== undefined || config === undefined).toBe(true);
      return;
    }
    expect(disagreements(oracleCase, records, port => config?.verdict(port, ruleCatalog))).toEqual([]);
  });
});
