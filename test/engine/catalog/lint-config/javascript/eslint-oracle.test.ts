import { mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { eslintReader } from "../../../../../src/engine/catalog/lint-config/javascript/eslint-config.js";
import { globPattern } from "../../../../../src/engine/catalog/rule-pack-matcher.js";
import { portsOf, settleVerdicts, type FolderDecision, type LinterNote, type Port, type PortDecision, type PortVerdict } from "../../../../../src/engine/catalog/lint-config/linter-config.js";
import type { RuleCatalog } from "../../../../../src/engine/catalog/repository-model.js";
import { readRuleCatalog } from "../../../../../src/engine/catalog/rule-catalog.js";
import { fixturesPath } from "../../../../support/oracles.js";
import { removeFolder } from "../../../../support/platform/remove-folder.js";

/**
 * The reader against ESLint itself (fixtures/host/lint-config/eslint/: ESLint 10.11.0's own config for each probe file
 * of each config in corpus.json, recorded by generate_eslint.mjs). Where the reader decides a ported rule on or off at
 * a probe, ESLint must agree; it may leave the rule unknown only where the case says Rerail cannot read it. The
 * verdict settled alone gives ESLint's state at each probe, or the pack's where the verdict is unknown.
 */
interface OracleCase {
  readonly name: string;
  readonly files: Readonly<Record<string, string>>;
  readonly probes: readonly string[];
  /** Per probe (`*` for every probe), the rules (`*` for every rule) the reader may leave unknown. */
  readonly unknown?: Readonly<Record<string, readonly string[]>>;
  /** Rules the reader decides even where `unknown` allows any. */
  readonly known?: readonly string[];
  /** Per rule, whether the reader decides it explicitly (the config names it) wherever it decides it. */
  readonly explicit?: Readonly<Record<string, boolean>>;
}

const ORACLE = fixturesPath("host", "lint-config", "eslint");
const { cases } = JSON.parse(readFileSync(join(ORACLE, "corpus.json"), "utf8")) as { cases: OracleCase[] };
const expected = JSON.parse(readFileSync(join(ORACLE, "eslint.expected.json"), "utf8")) as Record<string, Record<string, string>>;
const PACKS = ["javascript", "typescript", "react", "node", "next", "vue"];

let ruleCatalog: RuleCatalog;
/** Each ported ESLint rule with its port. */
let ports: [string, Port][];
beforeAll(() => {
  ruleCatalog = readRuleCatalog(PACKS.map(pack => fileURLToPath(new URL(`../../../../../packs/${pack}`, import.meta.url))));
  ports = portsOf(ruleCatalog, PACKS).flatMap(port => port.suppress.eslint.map((key): [string, Port] => [key, port]));
});

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

async function repository(files: Readonly<Record<string, string>>): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-eslint-oracle-")));
  temporaryDirectories.push(root);
  for (const [name, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, name)), { recursive: true });
    writeFileSync(join(root, name), text);
  }
  return root;
}

/** The verdict's decision at `probe`: the one of the deepest folder holding it, else the one for every file. */
function decisionAt(verdict: PortVerdict | undefined, probe: string): PortDecision | FolderDecision | undefined {
  const folders = (verdict?.folders ?? []).filter(decision => probe.startsWith(`${decision.folder}/`));
  return folders.sort((left, right) => right.folder.length - left.folder.length)[0] ?? verdict?.decision;
}

/** The state at `probe` of the settings `verdict` alone settles to for `port` (taken as not held off): the deepest setting holding it, else the pack's. */
function settledAt(port: Port, verdict: PortVerdict | undefined, probe: string): boolean {
  const free: Port = { ruleId: port.ruleId, suppress: port.suppress, declared: port.declared, startsOn: port.startsOn };
  const settings = settleVerdicts(free, verdict === undefined ? [] : [verdict], []).settings
    .filter(setting => setting.on !== null && (setting.folder === "" || probe.startsWith(`${setting.folder}/`)));
  return settings.sort((left, right) => right.folder.length - left.folder.length)[0]?.on ?? port.startsOn;
}

const mayBeUnknown = (oracleCase: OracleCase, probe: string, key: string): boolean =>
  !(oracleCase.known ?? []).includes(key)
  && [oracleCase.unknown?.[probe] ?? [], oracleCase.unknown?.["*"] ?? []].some(keys => keys.includes("*") || keys.includes(key));

/** Whether `port` runs on `probe`: its document's scope holds it. */
const runsOn = (port: Port, probe: string): boolean =>
  globPattern(port.scope?.include ?? ["**/*"]).test(probe) && !globPattern(port.scope?.exclude ?? []).test(probe);

/**
 * How the reader disagrees with ESLint (`eslint`: on at `probe` or not) on `key` of `port`: its state, how explicit it
 * is where the case says, and the state it settles to alone for a port starting either way (the pack's where unknown).
 */
function disagreements(oracleCase: OracleCase, probe: string, key: string, port: Port, verdict: PortVerdict | undefined, eslint: boolean): string[] {
  const found: string[] = [];
  const decision = decisionAt(verdict, probe);
  const state = decision?.on ?? "unknown";
  if (state === "unknown" ? !mayBeUnknown(oracleCase, probe, key) : state !== eslint) found.push(`${probe} ${key}: Rerail ${state}, ESLint ${eslint ? "on" : "off"}`);
  const explicit = oracleCase.explicit?.[key];
  if (explicit !== undefined && decision !== undefined && decision.on !== "unknown" && decision.explicit !== explicit) found.push(`${probe} ${key}: explicit ${decision.explicit}, not ${explicit}`);
  for (const startsOn of [true, false]) {
    const settled = settledAt({ ...port, startsOn }, verdict, probe);
    if (settled !== (state === "unknown" ? startsOn : eslint)) found.push(`${probe} ${key} starting ${startsOn ? "on" : "off"}: settles ${settled ? "on" : "off"}`);
  }
  return found;
}

describe("the ESLint reader against ESLint 10.11.0", () => {
  it.each(cases.map(oracleCase => [oracleCase.name, oracleCase] as const))("%s", async (_, oracleCase) => {
    // The probe files are the repository's files the ports run on.
    const root = await repository({ ...Object.fromEntries(oracleCase.probes.map(probe => [probe, ""])), ...oracleCase.files });
    const config = eslintReader.read(root, [] as LinterNote[])!;
    const found: string[] = [];
    for (const probe of oracleCase.probes) {
      const on = new Set(expected[oracleCase.name]![probe]!.split(" "));
      for (const [key, port] of ports.filter(([, port]) => runsOn(port, probe))) {
        found.push(...disagreements(oracleCase, probe, key, port, config.verdict(port, ruleCatalog), on.has(key)));
      }
    }
    expect(found).toEqual([]);
  });
});
