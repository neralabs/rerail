import { mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { portsOf, settleVerdicts, type FolderDecision, type LinterNote, type Port, type PortDecision, type PortVerdict } from "../../../../../src/engine/catalog/lint-config/linter-config.js";
import { combineDecisions } from "../../../../../src/engine/catalog/lint-config/linter-decisions.js";
import { sqlfluffReader } from "../../../../../src/engine/catalog/lint-config/sql/sqlfluff-config.js";
import type { RuleCatalog } from "../../../../../src/engine/catalog/repository-model.js";
import { readRuleCatalog } from "../../../../../src/engine/catalog/rule-catalog.js";
import { fixturesPath, repositoryPath } from "../../../../support/oracles.js";
import { removeFolder } from "../../../../support/platform/remove-folder.js";

/** One config of fixtures/host/lint-config/sqlfluff/corpus.json; generate_sqlfluff.py records what SQLFluff 4.4.0 runs with it. */
interface OracleCase {
  readonly name: string;
  readonly files: Readonly<Record<string, string>>;
  /** Folders (`""`: every file) where the reader may leave the pack's value: SQLFluff stops, or reads what Rerail does not. */
  readonly unknown?: readonly string[];
  /** SQL files SQLFluff ignores that the reader cannot set rules for alone; a note must name each. */
  readonly noted?: readonly string[];
  /** Per code, whether the reader's decision for it, wherever it decides one, is explicit: the config names it, or only defaults decide it. */
  readonly explicit?: Readonly<Record<string, boolean>>;
}

const ORACLE = fixturesPath("host", "lint-config", "sqlfluff");
const corpus = JSON.parse(readFileSync(join(ORACLE, "corpus.json"), "utf8")) as { probes: string[]; cases: OracleCase[] };
/** Per case, per probe folder, the codes of the rules SQLFluff runs there, space-separated; `error: …` where it stops. */
const expected = JSON.parse(readFileSync(join(ORACLE, "sqlfluff.expected.json"), "utf8")) as Record<string, Record<string, string> | string>;
const CODE = /^[A-Z]{2}\d{2}$/u;

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

let ruleCatalog: RuleCatalog;
let ports: Port[];
beforeAll(() => {
  ruleCatalog = readRuleCatalog([repositoryPath("packs", "sql")]);
  ports = portsOf(ruleCatalog, ["sql"]).filter(port => port.suppress.sqlfluff.length > 0);
});

/** The case's repository: probe.sql in every probe folder, and the case's files. */
async function repository(oracleCase: OracleCase): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-sqlfluff-oracle-")));
  temporaryDirectories.push(root);
  for (const folder of corpus.probes) {
    mkdirSync(join(root, folder), { recursive: true });
    writeFileSync(join(root, folder, "probe.sql"), "select 1\n");
  }
  for (const [name, text] of Object.entries(oracleCase.files)) {
    mkdirSync(dirname(join(root, name)), { recursive: true });
    writeFileSync(join(root, name), text);
  }
  return root;
}

const within = (path: string, folder: string): boolean => folder === "" || path === folder || path.startsWith(`${folder}/`);

/** `verdict`'s decision in folder `at`: the deepest folders deciding it, settled as one, else every file's decision. */
function atFolder(verdict: PortVerdict, at: string): PortDecision | FolderDecision | undefined {
  const folders = (verdict.folders ?? []).filter(decision => within(at, decision.folder));
  const deepest = Math.max(-1, ...folders.map(decision => decision.folder.length));
  return combineDecisions(folders.filter(decision => decision.folder.length === deepest)) ?? verdict.decision;
}

/** Whether `port`, starting as `startsOn`, is on in folder `at` once `verdict` alone is settled into settings. */
function settledAt(verdict: PortVerdict, port: Port, startsOn: boolean, at: string): boolean {
  const { settings } = settleVerdicts({ ruleId: port.ruleId, suppress: port.suppress, declared: port.declared, startsOn }, [verdict], []);
  const holding = settings.filter(setting => setting.on !== null && within(at, setting.folder));
  return holding.toSorted((left, right) => right.folder.length - left.folder.length)[0]?.on ?? startsOn;
}

describe("sqlfluffReader against SQLFluff 4.4.0 (fixtures/host/lint-config/sqlfluff)", () => {
  it("records every case of the corpus", () => {
    expect(expected.sqlfluff).toBe("4.4.0");
    expect(Object.keys(expected).filter(name => name !== "sqlfluff").sort()).toEqual(corpus.cases.map(oracleCase => oracleCase.name).sort());
  });

  it.each(corpus.cases)("decides as SQLFluff does: $name", async oracleCase => {
    const root = await repository(oracleCase);
    const notes: LinterNote[] = [];
    const config = sqlfluffReader.read(root, notes);
    const runs = expected[oracleCase.name] as Record<string, string>;
    const codeOf = (port: Port): string => port.suppress.sqlfluff.find(name => CODE.test(name))!;
    expect(Object.keys(oracleCase.explicit ?? {}).filter(code => !ports.some(port => codeOf(port) === code))).toEqual([]);
    const wrong: string[] = [];
    for (const port of ports) {
      const code = codeOf(port);
      const verdict = config?.verdict(port, ruleCatalog) ?? {};
      for (const folder of corpus.probes) {
        // A folder SQLFluff stops on runs no rule.
        const sqlfluff = runs[folder]!.split(" ").includes(code);
        const decided = atFolder(verdict, folder);
        const reader = decided?.on ?? true;
        const stated = oracleCase.explicit?.[code];
        if (stated !== undefined && decided !== undefined && decided.on !== "unknown" && decided.explicit !== stated) wrong.push(`${code} in ${JSON.stringify(folder)}: the reader's decision is ${stated ? "not " : ""}explicit`);
        // Settled alone, the verdict leaves the pack's value where it decides nothing or is unknown, and SQLFluff's state elsewhere.
        for (const startsOn of [true, false]) {
          const settled = settledAt(verdict, port, startsOn, folder);
          if (settled !== (decided === undefined || reader === "unknown" ? startsOn : sqlfluff)) wrong.push(`${code} in ${JSON.stringify(folder)}, starting ${startsOn ? "on" : "off"}: settled ${settled ? "on" : "off"}`);
        }
        if (reader === "unknown" ? (oracleCase.unknown ?? []).some(marked => within(folder, marked)) : reader === sqlfluff) continue;
        wrong.push(`${code} in ${JSON.stringify(folder)}: SQLFluff ${sqlfluff ? "runs" : "does not run"} it, the reader says ${String(reader)}`);
      }
    }
    // Each SQL file of the case's own SQLFluff ignores while linting its folder is one a note names.
    const ignoredFiles = Object.keys(oracleCase.files).filter(name => runs[name] === "ignored");
    expect(ignoredFiles).toEqual(oracleCase.noted ?? []);
    for (const file of ignoredFiles) expect(notes.some(note => note.text.includes(file))).toBe(true);
    expect(wrong).toEqual([]);
  });
});
