import { mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { eslintReader } from "../../../../src/engine/catalog/lint-config/eslint-config.js";
import { globPattern } from "../../../../src/engine/catalog/rule-pack-matcher.js";
import { portsOf, type LinterNote, type Port, type PortVerdict } from "../../../../src/engine/catalog/lint-config/linter-config.js";
import type { RuleCatalog } from "../../../../src/engine/catalog/repository-model.js";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import { fixturesPath } from "../../../support/oracles.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

/**
 * The reader against ESLint itself (fixtures/host/lint-config/eslint/: ESLint 10.11.0's own config for each probe file
 * of each config in corpus.json, recorded by generate_eslint.mjs). Where the reader decides a ported rule on or off at
 * a probe, ESLint must agree; it may leave the rule unknown only where the case says Rerail cannot read it.
 */
interface OracleCase {
  readonly name: string;
  readonly files: Readonly<Record<string, string>>;
  readonly probes: readonly string[];
  /** Per probe (`*` for every probe), the rules (`*` for every rule) the reader may leave unknown. */
  readonly unknown?: Readonly<Record<string, readonly string[]>>;
  /** Rules the reader decides even where `unknown` allows any. */
  readonly known?: readonly string[];
}

const ORACLE = fixturesPath("host", "lint-config", "eslint");
const { cases } = JSON.parse(readFileSync(join(ORACLE, "corpus.json"), "utf8")) as { cases: OracleCase[] };
const expected = JSON.parse(readFileSync(join(ORACLE, "eslint.expected.json"), "utf8")) as Record<string, Record<string, string>>;
const PACKS = ["javascript", "typescript", "react", "node", "next", "vue"];

let ruleCatalog: RuleCatalog;
/** Each ported ESLint rule with its port. */
let ports: [string, Port][];
beforeAll(() => {
  ruleCatalog = readRuleCatalog(PACKS.map(pack => fileURLToPath(new URL(`../../../../packs/${pack}`, import.meta.url))));
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

/** The verdict's state at `probe`: the decision of the deepest folder holding it, else the one for every file. */
function stateAt(verdict: PortVerdict | undefined, probe: string): boolean | "unknown" {
  const folders = (verdict?.folders ?? []).filter(decision => probe.startsWith(`${decision.folder}/`));
  const deepest = folders.sort((left, right) => right.folder.length - left.folder.length)[0];
  return deepest?.on ?? verdict?.decision?.on ?? "unknown";
}

const mayBeUnknown = (oracleCase: OracleCase, probe: string, key: string): boolean =>
  !(oracleCase.known ?? []).includes(key)
  && [oracleCase.unknown?.[probe] ?? [], oracleCase.unknown?.["*"] ?? []].some(keys => keys.includes("*") || keys.includes(key));

/** Whether `port` runs on `probe`: its document's scope holds it. */
const runsOn = (port: Port, probe: string): boolean =>
  globPattern(port.scope?.include ?? ["**/*"]).test(probe) && !globPattern(port.scope?.exclude ?? []).test(probe);

describe("the ESLint reader against ESLint 10.11.0", () => {
  it.each(cases.map(oracleCase => [oracleCase.name, oracleCase] as const))("%s", async (_, oracleCase) => {
    // The probe files are the repository's files the ports run on.
    const root = await repository({ ...Object.fromEntries(oracleCase.probes.map(probe => [probe, ""])), ...oracleCase.files });
    const config = eslintReader.read(root, [] as LinterNote[])!;
    const disagreements: string[] = [];
    for (const probe of oracleCase.probes) {
      const on = new Set(expected[oracleCase.name]![probe]!.split(" "));
      for (const [key, port] of ports.filter(([, port]) => runsOn(port, probe))) {
        const state = stateAt(config.verdict(port, ruleCatalog), probe);
        const eslint = on.has(key);
        if (state === "unknown" ? !mayBeUnknown(oracleCase, probe, key) : state !== eslint) disagreements.push(`${probe} ${key}: Rerail ${state}, ESLint ${eslint ? "on" : "off"}`);
      }
    }
    expect(disagreements).toEqual([]);
  });
});
