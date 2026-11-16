import { mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { settleVerdicts, type FolderDecision, type LinterNote, type Port, type PortDecision, type PortVerdict } from "../../../../src/engine/catalog/lint-config/linter-config.js";
import { combineDecisions } from "../../../../src/engine/catalog/lint-config/linter-decisions.js";
import { ruffReader } from "../../../../src/engine/catalog/lint-config/ruff-config.js";
import type { RuleCatalog } from "../../../../src/engine/catalog/repository-model.js";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import { fixturesPath } from "../../../support/oracles.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

/** One config of fixtures/host/lint-config/ruff/corpus.json; generate_ruff.py records what Ruff 0.16.9 does with it. */
interface OracleCase {
  readonly name: string;
  readonly files: Readonly<Record<string, string>>;
  /** Folders (`""`: every file) where the reader may leave the pack's value: Ruff refuses the config, or reads what Rerail does not. */
  readonly unknown?: readonly string[];
  /** Per code, whether the reader's decision for it, wherever it decides one, is explicit: the config names it, or only defaults and presets decide it. */
  readonly explicit?: Readonly<Record<string, boolean>>;
}

const ORACLE = fixturesPath("host", "lint-config", "ruff");
const corpus = JSON.parse(readFileSync(join(ORACLE, "corpus.json"), "utf8")) as { codes: string[]; probes: string[]; cases: OracleCase[] };
/** Per case, per probe file, the codes Ruff runs there, space-separated; "error" where Ruff refuses the config. */
const expected = JSON.parse(readFileSync(join(ORACLE, "ruff.expected.json"), "utf8")) as Record<string, Record<string, string> | "error" | string>;

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

// The Ruff reader does not look into the catalog; an empty one stands in.
let ruleCatalog: RuleCatalog;
beforeAll(() => {
  ruleCatalog = readRuleCatalog([]);
});

/** A port honouring noqa code `code` alone, starting on or off as `startsOn`. */
const port = (code: string, startsOn = true): Port => ({
  ruleId: code,
  suppress: { eslint: [], noqa: [code], nosec: [], oxlint: [], biome: [], phpstan: [], psalm: [], phpcs: [], clippy: [], sqlfluff: [] },
  declared: {},
  startsOn,
});

const probeFile = (folder: string): string => (folder === "" ? "probe.py" : `${folder}/probe.py`);

/** The case's repository: the probe in every probe folder, and the case's files. */
async function repository(oracleCase: OracleCase): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-ruff-oracle-")));
  temporaryDirectories.push(root);
  const files = { ...Object.fromEntries(corpus.probes.map(folder => [probeFile(folder), "x = 1\n"])), ...oracleCase.files };
  for (const [name, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, name)), { recursive: true });
    writeFileSync(join(root, name), text);
  }
  return root;
}

const within = (path: string, folder: string): boolean => folder === "" || path.startsWith(`${folder}/`);

/** `verdict`'s decision at file `file`: the deepest folders deciding it, settled as one, else every file's decision. */
function atFile(verdict: PortVerdict, file: string): PortDecision | FolderDecision | undefined {
  const folders = (verdict.folders ?? []).filter(decision => within(file, decision.folder));
  const deepest = Math.max(-1, ...folders.map(decision => decision.folder.length));
  return combineDecisions(folders.filter(decision => decision.folder.length === deepest)) ?? verdict.decision;
}

/** Whether port `code`, starting as `startsOn`, is on in file `file` once `verdict` alone is settled into settings. */
function settledAt(verdict: PortVerdict, code: string, startsOn: boolean, file: string): boolean {
  const { settings } = settleVerdicts(port(code, startsOn), [verdict], []);
  const holding = settings.filter(setting => setting.on !== null && within(file, setting.folder));
  return holding.toSorted((left, right) => right.folder.length - left.folder.length)[0]?.on ?? startsOn;
}

describe("ruffReader against Ruff 0.16.9 (fixtures/host/lint-config/ruff)", () => {
  it("records every case of the corpus", () => {
    expect(expected.ruff).toBe("0.16.9");
    expect(Object.keys(expected).filter(name => name !== "ruff").sort()).toEqual(corpus.cases.map(oracleCase => oracleCase.name).sort());
  });

  it.each(corpus.cases)("decides as Ruff does: $name", async oracleCase => {
    const root = await repository(oracleCase);
    const notes: LinterNote[] = [];
    const config = ruffReader.read(root, notes);
    const ran = expected[oracleCase.name]!;
    // A config Ruff refuses is one the reader reads nothing of.
    expect(config === undefined).toBe(ran === "error");
    const wrong: string[] = [];
    for (const code of corpus.codes) {
      const verdict = config?.verdict(port(code), ruleCatalog) ?? {};
      for (const file of corpus.probes.map(probeFile)) {
        const ruff = typeof ran === "object" && ran[file]!.split(" ").includes(code);
        const decided = atFile(verdict, file);
        const reader = decided?.on ?? "unknown";
        const stated = oracleCase.explicit?.[code];
        if (stated !== undefined && decided !== undefined && decided.on !== "unknown" && decided.explicit !== stated) wrong.push(`${code} in ${file}: the reader's decision is ${stated ? "not " : ""}explicit`);
        // Settled alone, the verdict leaves the pack's value where it is unknown, and Ruff's state elsewhere.
        for (const startsOn of [true, false]) {
          const settled = settledAt(verdict, code, startsOn, file);
          if (settled !== (reader === "unknown" ? startsOn : ruff)) wrong.push(`${code} in ${file}, starting ${startsOn ? "on" : "off"}: settled ${settled ? "on" : "off"}`);
        }
        if (reader === "unknown" ? (oracleCase.unknown ?? []).some(folder => within(file, folder)) : reader === ruff) continue;
        wrong.push(`${code} in ${file}: Ruff ${ruff ? "runs" : "does not run"} it, the reader says ${String(reader)}`);
      }
    }
    expect(wrong).toEqual([]);
  });
});
