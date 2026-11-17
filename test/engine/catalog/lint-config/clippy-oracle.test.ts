import { mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { clippyReader } from "../../../../src/engine/catalog/lint-config/clippy-config.js";
import { portsOf, settleVerdicts, type FolderDecision, type LinterNote, type Port, type PortDecision, type PortVerdict } from "../../../../src/engine/catalog/lint-config/linter-config.js";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import { fixturesPath, repositoryPath } from "../../../support/oracles.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

/** A workspace of the corpus, the folders where Rerail is expected to leave lints to the pack, and where it names a lint. */
interface CorpusCase {
  readonly name: string;
  readonly files: Readonly<Record<string, string>>;
  /** Folder → the lints Rerail cannot tell there, or `"*"` for every one. */
  readonly unknown?: Readonly<Record<string, readonly string[] | "*">>;
  /** Lint → whether the config names it (its own name, or a group of it) wherever the reader decides it. */
  readonly explicit?: Readonly<Record<string, boolean>>;
}

/** What `cargo clippy` decided for one package: the lints on in every target, and those on in some only. */
interface PackageRecord {
  readonly flags: readonly string[];
  readonly on: readonly string[];
  readonly mixed: readonly string[];
}

type CaseRecord = { readonly name: string; readonly error: string } | { readonly name: string; readonly packages: Readonly<Record<string, PackageRecord>> };

/** The oracle file `name`, written by `generate_clippy.py` or by hand (the corpus), as its shape. */
function oracle<T>(name: string): T {
  return JSON.parse(readFileSync(fixturesPath("host", "lint-config", "clippy", name), "utf8")) as T;
}

const corpus = oracle<{ cases: CorpusCase[] }>("corpus.json").cases;
const recorded = new Map(oracle<{ cases: CaseRecord[] }>("corpus.expected.json").cases.map(record => [record.name, record]));

const ruleCatalog = readRuleCatalog([repositoryPath("packs", "rust")]);
const ports = portsOf(ruleCatalog, ["rust"]);

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

async function repository(files: Readonly<Record<string, string>>): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-clippy-oracle-")));
  temporaryDirectories.push(root);
  for (const [name, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, name)), { recursive: true });
    writeFileSync(join(root, name), text);
  }
  return root;
}

const within = (path: string, folder: string): boolean => folder === "" || path.startsWith(`${folder}/`);

/** A file of package folder `folder`, where its decisions are looked up. */
const probeFile = (folder: string): string => (folder === "" ? "src/rerail_probe.rs" : `${folder}/src/rerail_probe.rs`);

/** What `verdict` decides at `file`: the deepest folder decision holding it, else the decision for every file. */
function decisionAt(verdict: PortVerdict | undefined, file: string): PortDecision | FolderDecision | undefined {
  const holding = (verdict?.folders ?? []).filter(decided => within(file, decided.folder));
  return holding.toSorted((a, b) => b.folder.length - a.folder.length)[0] ?? verdict?.decision;
}

/** Whether `port`, starting as `startsOn`, is on at `file` once `verdict` alone is settled into settings. */
function settledAt(verdict: PortVerdict | undefined, port: Port, startsOn: boolean, file: string): boolean {
  const { settings } = settleVerdicts({ ...port, startsOn }, verdict === undefined ? [] : [verdict], []);
  const holding = settings.filter(setting => setting.on !== null && within(file, setting.folder));
  return holding.toSorted((a, b) => b.folder.length - a.folder.length)[0]?.on ?? startsOn;
}

const expectedUnknown = (entry: CorpusCase, folder: string, lint: string): boolean => {
  const lints = entry.unknown?.[folder];
  return lints === "*" || (lints?.includes(lint) ?? false);
};

/** What `cargo clippy` decided for one port in one package folder, and what the reader decides there. */
interface Probe {
  readonly entry: CorpusCase;
  readonly port: Port;
  readonly lint: string;
  readonly folder: string;
  readonly clippy: boolean | "mixed";
  readonly decided: PortDecision | FolderDecision | undefined;
}

const CLIPPY_STATES: Readonly<Record<string, string>> = { mixed: "differs by target", true: "on", false: "off" };

/** Where the reader's own decision disagrees with Clippy's state, or with whether the case says the config names the lint. */
function decisionDisagreements({ entry, port, lint, folder, clippy, decided }: Probe): string[] {
  const state = decided?.on ?? "unknown";
  const agrees = state === "unknown" ? clippy === "mixed" || expectedUnknown(entry, folder, lint) : state === clippy;
  const stated = entry.explicit?.[lint];
  const named = decided === undefined || decided.on === "unknown" || stated === undefined || decided.explicit === stated;
  return [
    ...(agrees ? [] : [`${port.ruleId} in "${folder}": Clippy ${CLIPPY_STATES[String(clippy)]}, reader says ${state}`]),
    ...(named ? [] : [`${port.ruleId} in "${folder}": the reader's decision is not as explicit as stated (${String(stated)})`]),
  ];
}

/** Where `verdict` alone, settled for a port starting on and one starting off, leaves other than Clippy's state, or the pack's where unknown. */
function settledDisagreements({ port, folder, clippy, decided }: Probe, verdict: PortVerdict | undefined): string[] {
  return [true, false].flatMap(startsOn => {
    const settled = settledAt(verdict, port, startsOn, probeFile(folder));
    const expected = decided === undefined || decided.on === "unknown" ? startsOn : clippy;
    return settled === expected ? [] : [`${port.ruleId} in "${folder}", starting ${String(startsOn)}: settled ${String(settled)}`];
  });
}

/** Where the reader disagrees with `cargo clippy` on `entry`, one line each; empty when it agrees everywhere. */
async function disagreements(entry: CorpusCase): Promise<string[]> {
  const record = recorded.get(entry.name);
  if (record === undefined) return [`${entry.name}: no record; run generate_clippy.py`];
  const notes: LinterNote[] = [];
  const config = clippyReader.read(await repository(entry.files), notes);
  const found: string[] = [];
  for (const port of ports) {
    const lint = port.suppress.clippy[0]!;
    const verdict = config?.verdict(port, ruleCatalog);
    for (const folder of Object.keys(entry.unknown ?? {})) {
      const state = decisionAt(verdict, probeFile(folder))?.on ?? "unknown";
      if (expectedUnknown(entry, folder, lint) && state !== "unknown") found.push(`${port.ruleId} in "${folder}": expected unknown, reader says ${state}`);
    }
    if ("error" in record) continue;
    for (const [folder, cargo] of Object.entries(record.packages)) {
      const probe: Probe = { entry, port, lint, folder, clippy: cargo.mixed.includes(lint) ? "mixed" : cargo.on.includes(lint), decided: decisionAt(verdict, probeFile(folder)) };
      found.push(...decisionDisagreements(probe), ...settledDisagreements(probe, verdict));
    }
  }
  return found;
}

describe("clippyReader against cargo clippy 0.1.99", () => {
  it("has a record of every case", () => {
    expect([...recorded.keys()]).toEqual(corpus.map(entry => entry.name));
  });

  it.each(corpus.map(entry => [entry.name, entry] as const))("agrees with Clippy on %s", async (_, entry) => {
    expect(await disagreements(entry)).toEqual([]);
  });
});
