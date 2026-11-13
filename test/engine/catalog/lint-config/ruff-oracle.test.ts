import { mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { LinterNote, Port, PortVerdict } from "../../../../src/engine/catalog/lint-config/linter-config.js";
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

/** A port honouring noqa code `code` alone. */
const port = (code: string): Port => ({
  ruleId: code,
  suppress: { eslint: [], noqa: [code], nosec: [], oxlint: [], biome: [], phpstan: [], psalm: [], phpcs: [], clippy: [], sqlfluff: [] },
  declared: {},
  startsOn: true,
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

/** What `verdict` leaves file `file` at: the deepest folder deciding it (an off first, then an on, then unknown), else every file's decision. */
function atFile(verdict: PortVerdict, file: string): boolean | "unknown" | undefined {
  const folders = (verdict.folders ?? []).filter(decision => within(file, decision.folder));
  const deepest = Math.max(-1, ...folders.map(decision => decision.folder.length));
  const decided = folders.filter(decision => decision.folder.length === deepest).map(decision => decision.on);
  if (decided.length === 0) return verdict.decision?.on;
  return decided.includes(false) ? false : decided.includes(true) ? true : "unknown";
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
        const reader = atFile(verdict, file) ?? "unknown";
        if (reader === "unknown" ? (oracleCase.unknown ?? []).some(folder => within(file, folder)) : reader === ruff) continue;
        wrong.push(`${code} in ${file}: Ruff ${ruff ? "runs" : "does not run"} it, the reader says ${String(reader)}`);
      }
    }
    expect(wrong).toEqual([]);
  });
});
