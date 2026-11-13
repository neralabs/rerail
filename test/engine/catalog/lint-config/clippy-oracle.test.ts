import { mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { clippyReader } from "../../../../src/engine/catalog/lint-config/clippy-config.js";
import { portsOf, type LinterNote, type PortVerdict } from "../../../../src/engine/catalog/lint-config/linter-config.js";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import { fixturesPath, repositoryPath } from "../../../support/oracles.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

/** A workspace of the corpus, and the folders where Rerail is expected to leave lints to the pack. */
interface CorpusCase {
  readonly name: string;
  readonly files: Readonly<Record<string, string>>;
  /** Folder → the lints Rerail cannot tell there, or `"*"` for every one. */
  readonly unknown?: Readonly<Record<string, readonly string[] | "*">>;
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

/** What `verdict` leaves in `folder`: the deepest folder decision holding it, else the decision for every file. */
function stateIn(verdict: PortVerdict | undefined, folder: string): boolean | "unknown" {
  const holding = (verdict?.folders ?? []).filter(decided => folder === decided.folder || folder.startsWith(`${decided.folder}/`));
  const deepest = holding.sort((a, b) => b.folder.length - a.folder.length)[0];
  return deepest?.on ?? verdict?.decision?.on ?? "unknown";
}

const expectedUnknown = (entry: CorpusCase, folder: string, lint: string): boolean => {
  const lints = entry.unknown?.[folder];
  return lints === "*" || (lints?.includes(lint) ?? false);
};

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
      const state = stateIn(verdict, folder);
      if (expectedUnknown(entry, folder, lint) && state !== "unknown") found.push(`${port.ruleId} in "${folder}": expected unknown, reader says ${state}`);
    }
    if ("error" in record) continue;
    for (const [folder, cargo] of Object.entries(record.packages)) {
      const expected = cargo.mixed.includes(lint) ? "mixed" : cargo.on.includes(lint);
      const state = stateIn(verdict, folder);
      if (state === "unknown" ? expected !== "mixed" && !expectedUnknown(entry, folder, lint) : state !== expected) {
        found.push(`${port.ruleId} in "${folder}": Clippy ${expected === "mixed" ? "differs by target" : expected ? "on" : "off"}, reader says ${state}`);
      }
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
