import { mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { banditReader } from "../../../../src/engine/catalog/lint-config/bandit-config.js";
import { portsOf, type LinterNote, type Port, type PortVerdict } from "../../../../src/engine/catalog/lint-config/linter-config.js";
import type { RuleCatalog } from "../../../../src/engine/catalog/repository-model.js";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import { fixturesPath, repositoryPath } from "../../../support/oracles.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

/** One config of fixtures/host/lint-config/bandit/corpus.json; generate_bandit.py records what Bandit 1.9.4 does with it. */
interface OracleCase {
  readonly name: string;
  readonly files: Readonly<Record<string, string>>;
  /** Folders (`""`: every file) where the reader may leave the pack's value: Bandit stops, or reads what Rerail does not. */
  readonly unknown?: readonly string[];
  /** Files Bandit skips that the reader cannot set rules for alone; a note must name each. */
  readonly noted?: readonly string[];
}

const ORACLE = fixturesPath("host", "lint-config", "bandit");
const corpus = JSON.parse(readFileSync(join(ORACLE, "corpus.json"), "utf8")) as { probes: string[]; cases: OracleCase[] };
/** Per case, per file Bandit reports in, the test ids it reports there, space-separated; "error" where Bandit stops. */
const expected = JSON.parse(readFileSync(join(ORACLE, "bandit.expected.json"), "utf8")) as Record<string, Record<string, string> | "error" | string>;

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

let ruleCatalog: RuleCatalog;
/** One port per Bandit test id the python pack ports, matching that id alone. */
let ports: Port[];
beforeAll(() => {
  ruleCatalog = readRuleCatalog([repositoryPath("packs", "python")]);
  const python = portsOf(ruleCatalog, ["python"]).filter(port => port.suppress.nosec.length > 0);
  const ids = [...new Set(python.flatMap(port => port.suppress.nosec))].sort();
  ports = ids.map(id => ({ ...python[0]!, ruleId: id, suppress: { ...python[0]!.suppress, nosec: [id] } }));
});

/** The case's repository: the probe in every probe folder, and the case's files. */
async function repository(oracleCase: OracleCase): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-bandit-oracle-")));
  temporaryDirectories.push(root);
  const files = { ...Object.fromEntries(corpus.probes.map(folder => [join(folder, "probe.py"), "assert True\n"])), ...oracleCase.files };
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

describe("banditReader against Bandit 1.9.4 (fixtures/host/lint-config/bandit)", () => {
  it("records every case of the corpus", () => {
    expect(expected.bandit).toBe("1.9.4");
    expect(Object.keys(expected).filter(name => name !== "bandit").sort()).toEqual(corpus.cases.map(oracleCase => oracleCase.name).sort());
  });

  it.each(corpus.cases)("decides as Bandit does: $name", async oracleCase => {
    const root = await repository(oracleCase);
    const notes: LinterNote[] = [];
    const config = banditReader.read(root, notes);
    const reported = expected[oracleCase.name]!;
    const files = [...corpus.probes.map(folder => (folder === "" ? "probe.py" : `${folder}/probe.py`)), ...Object.keys(oracleCase.files).filter(name => name.endsWith(".py"))];
    const wrong: string[] = [];
    for (const port of ports) {
      const id = port.suppress.nosec[0]!;
      const verdict = config?.verdict(port, ruleCatalog) ?? {};
      for (const file of files) {
        const bandit = typeof reported === "object" && (reported[file] ?? "").split(" ").includes(id);
        const reader = atFile(verdict, file) ?? true;
        if (reader === "unknown" ? (oracleCase.unknown ?? []).some(folder => within(file, folder)) : reader === bandit) continue;
        const noted = !bandit && reader === true && (oracleCase.noted ?? []).includes(file) && notes.some(note => note.text.includes(file));
        if (!noted) wrong.push(`${id} in ${file}: Bandit ${bandit ? "reports" : "does not report"} it, the reader says ${String(reader)}`);
      }
    }
    expect(wrong).toEqual([]);
  });
});
