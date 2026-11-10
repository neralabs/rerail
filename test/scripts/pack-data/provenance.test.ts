/**
 * scripts/pack-data/provenance.mjs: the per-file provenance.json and LICENSES.md a generator writes into its tool
 * directory, what the writer classifies itself, what it refuses, rename variants' entries, the Python generators' way
 * in (scripts/s3/pack_fixtures.py `write_provenance`) and derive's check that a generator wrote them.
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { afterEach, describe, test } from "vitest";

import { assertProvenanceWritten, LICENSES, PROVENANCE, provenanceTexts, variantEntries, writeProvenance, type ProvenanceEntry } from "../../../scripts/pack-data/provenance.mjs";
import { COMMIT, LICENCE_TEXT, PACK, PROJECT, scratchCheckout, TOOL, TOOL_DIRECTORY, type ScratchCheckout } from "./support/provenance-root.js";

const REPOSITORY = resolve(import.meta.dirname, "..", "..", "..");
const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function checkout(options?: ScratchCheckout): string {
  const root = scratchCheckout(options);
  roots.push(root);
  return root;
}

const extracted: ProvenanceEntry = { detail: "invalid[0]", how: "extracted", from: "tests/a.js", source: PROJECT };
const ENTRIES: Record<string, ProvenanceEntry> = {
  "fixtures/a.js": extracted,
  "fixtures/B.js": { source: PROJECT, from: "tests/b.js", how: "verbatim" },
  "fixtures/renamed/a.js": { detail: "invalid[0]", renamedFrom: "fixtures/a.js", how: "renamed", from: "tests/a.js", source: PROJECT },
};
const COMMITS = { [PROJECT]: COMMIT };

/** The refusal `provenanceTexts` throws for these entries and commits, after checking it wrote nothing. */
function refusal(root: string, entries: Record<string, ProvenanceEntry>, commits: Record<string, string> = COMMITS): string {
  assert.throws(() => writeProvenance({ root, pack: PACK, tool: TOOL, entries, commits }));
  assert.equal(existsSync(join(root, TOOL_DIRECTORY, PROVENANCE)), false);
  assert.equal(existsSync(join(root, TOOL_DIRECTORY, LICENSES)), false);
  try {
    provenanceTexts({ root, pack: PACK, tool: TOOL, entries, commits });
  } catch (error) {
    return (error as Error).message;
  }
  return assert.fail("no refusal");
}

describe("provenance.json", () => {
  test("is format, pack, tool, sources, files, with byte-sorted keys, entry keys in order and Rerail's files classified", () => {
    const root = checkout();
    writeProvenance({ root, pack: PACK, tool: TOOL, entries: ENTRIES, commits: COMMITS });
    const expected = {
      format: "rerail.pack-provenance/v1",
      pack: PACK,
      tool: TOOL,
      sources: {
        [PROJECT]: { repository: "https://github.com/example/proj", tag: "v1.0.0", commit: COMMIT, license: "MIT" },
        rerail: { license: "Apache-2.0" },
      },
      files: {
        "authored/own.js": { source: "rerail", how: "authored" },
        "fixtures.expected.json": { source: "rerail", how: "recorded" },
        "fixtures/B.js": { source: PROJECT, from: "tests/b.js", how: "verbatim" },
        "fixtures/a.js": { source: PROJECT, from: "tests/a.js", how: "extracted", detail: "invalid[0]" },
        "fixtures/renamed/a.js": { source: PROJECT, from: "tests/a.js", how: "renamed", renamedFrom: "fixtures/a.js", detail: "invalid[0]" },
      },
    };
    assert.equal(readFileSync(join(root, TOOL_DIRECTORY, PROVENANCE), "utf8"), `${JSON.stringify(expected, null, 2)}\n`);
  });

  test("leaves out the tag of a source whose Source row has none (—)", () => {
    const root = checkout({ tag: "—" });
    const { provenance } = provenanceTexts({ root, pack: PACK, tool: TOOL, entries: ENTRIES, commits: COMMITS });
    assert.deepEqual(JSON.parse(provenance).sources[PROJECT], { repository: "https://github.com/example/proj", commit: COMMIT, license: "MIT" });
  });

  test("keeps a generator's Rerail entry with its detail for a file the tool's UPSTREAM.md lists as Rerail's", () => {
    const root = checkout({ files: ["fixtures/package.json"], authored: ["fixtures/package.json"], taken: [] });
    const own: ProvenanceEntry = { source: "rerail", how: "authored", detail: "the fixtures' manifest" };
    const { provenance } = provenanceTexts({ root, pack: PACK, tool: TOOL, entries: { "fixtures/package.json": own }, commits: {} });
    assert.deepEqual(JSON.parse(provenance).files, { "fixtures/package.json": own });
  });
});

describe("LICENSES.md", () => {
  test("lists each upstream source with its counts and licence text, then Rerail's files", () => {
    const root = checkout();
    const { licenses } = provenanceTexts({ root, pack: PACK, tool: TOOL, entries: ENTRIES, commits: COMMITS });
    const lines = licenses.split("\n");
    assert.equal(lines[0], `# Licences of the ${TOOL} test data`);
    assert.match(lines[2] ?? "", /how it was made\. Each upstream source below is listed .* under `packs\/demo\/licenses\/`; the rest is Rerail's\.$/u);
    assert.equal(lines[4], `## ${PROJECT}`);
    assert.equal(lines[6], `Repository https://github.com/example/proj, tag \`v1.0.0\`, commit \`${COMMIT}\`. License: MIT.`);
    assert.match(lines[8] ?? "", /^3 file\(s\): 1 verbatim, 1 extracted, 1 renamed\. They remain under proj's licence\./u);
    assert.ok(licenses.includes(`### LICENSE\n\n\`\`\`\n${LICENCE_TEXT.trimEnd()}\n\`\`\`\n`));
    assert.match(licenses, /\n## Rerail\n\n2 file\(s\): 1 recorded, 1 authored\. Rerail's, under Apache-2\.0 /u);
    assert.ok(licenses.endsWith(".\n") && !licenses.endsWith("\n\n"));
  });

  test("says every file is Rerail's when no upstream source has a file", () => {
    const root = checkout({ files: ["fixtures.expected.json", "authored/own.js"] });
    const { provenance, licenses } = provenanceTexts({ root, pack: PACK, tool: TOOL, entries: {}, commits: {} });
    assert.deepEqual(Object.keys(JSON.parse(provenance).sources), ["rerail"]);
    assert.match(licenses.split("\n")[2] ?? "", /how it was made\. Every file in it is Rerail's\.$/u);
    assert.ok(!licenses.includes(`## ${PROJECT}`));
  });
});

describe("refusals", () => {
  test("1: pack test data with no entry and no Rerail classification", () => {
    const root = checkout({ files: [...Object.keys(ENTRIES), "fixtures/stray.js"] });
    assert.match(refusal(root, ENTRIES), /fixtures\/stray\.js: pack test data with no entry, not listed as Rerail's/u);
  });

  test("1: an upstream entry for a file the tool's UPSTREAM.md lists as Rerail's", () => {
    const root = checkout({ authored: ["fixtures/B.js"] });
    assert.match(refusal(root, ENTRIES), /fixtures\/B\.js: an entry from proj, but the tool's UPSTREAM\.md or authored\/ makes it Rerail's/u);
  });

  test("2: an entry naming a file the tool directory lacks, or a missing renamedFrom", () => {
    const root = checkout({ files: ["fixtures/B.js", "fixtures/renamed/a.js"] });
    const message = refusal(root, ENTRIES);
    assert.match(message, /fixtures\/a\.js: an entry for a file the tool directory does not hold/u);
    assert.match(message, /fixtures\/renamed\/a\.js: renamed from fixtures\/a\.js, which the tool directory does not hold/u);
  });

  test("3: a source that is no licenses/<project>/ of the pack", () => {
    const root = checkout();
    assert.match(refusal(root, { ...ENTRIES, "fixtures/B.js": { source: "elsewhere", from: "b.js", how: "verbatim" } }), /fixtures\/B\.js: source "elsewhere" is neither rerail nor a directory of packs\/demo\/licenses\//u);
  });

  test("4: an upstream entry outside its source's Taken-from-it globs", () => {
    const root = checkout({ files: [...Object.keys(ENTRIES), "tests/x.js"] });
    assert.match(refusal(root, { ...ENTRIES, "tests/x.js": { source: PROJECT, from: "tests/x.js", how: "verbatim" } }), /upstream\/tool\/tests\/x\.js: matches none of the "Taken from it" globs of packs\/demo\/licenses\/proj\/UPSTREAM\.md/u);
  });

  test("5: an upstream entry whose upstream path is Not copied", () => {
    const root = checkout({ notCopied: ["tests/gpl/"] });
    assert.match(refusal(root, { ...ENTRIES, "fixtures/B.js": { source: PROJECT, from: "tests/gpl/b.js", how: "verbatim" } }), /fixtures\/B\.js: its upstream path tests\/gpl\/b\.js is under `tests\/gpl\/`, which .* lists as Not copied/u);
  });

  test("6: a Taken-from-it glob under the tool's directory that matches no file", () => {
    const root = checkout({ taken: [`upstream/${TOOL}/fixtures/**`, `upstream/${TOOL}/tests/**`, "upstream/other/tests/**"] });
    const message = refusal(root, ENTRIES);
    assert.match(message, /its "Taken from it" glob `upstream\/tool\/tests\/\*\*` matches no file/u);
    assert.ok(!message.includes("upstream/other/"));
  });

  test("7: a used project's commit missing or differing from its Source row", () => {
    const root = checkout();
    assert.match(refusal(root, ENTRIES, {}), /proj: no commit given for it/u);
    assert.match(refusal(root, ENTRIES, { [PROJECT]: "f".repeat(40) }), new RegExp(`proj: the generator checked out f{40}, but .*'s Source row names ${COMMIT}`, "u"));
  });
});

describe("variantEntries", () => {
  const originals = new Map<string, ProvenanceEntry>([
    ["fixtures/r/1.js", { source: PROJECT, from: "tests/r.js", how: "extracted", detail: "valid[0]" }],
    ["fixtures/u/index.js", { source: PROJECT, from: "tests/u.js", how: "extracted" }],
    ["fixtures/u/package.json", { source: PROJECT, from: "tests/u/package.json", how: "verbatim" }],
  ]);
  const recorded = {
    files: ["r/1.js", ["u/index.js", "u/package.json"], "renamed/r/1.js", ["renamed/u/index.js", "renamed/u/package.json"]],
    units: { "r/1.js": {}, "u/index.js": {}, "renamed/r/1.js": { renamedFrom: "r/1.js" }, "renamed/u/index.js": { renamedFrom: "u/index.js" } },
  };

  test("gives every path of every variant unit its original's entry, renamed from it", () => {
    assert.deepEqual(
      variantEntries(originals, recorded),
      new Map([
        ["fixtures/renamed/r/1.js", { source: PROJECT, from: "tests/r.js", how: "renamed", detail: "valid[0]", renamedFrom: "fixtures/r/1.js" }],
        ["fixtures/renamed/u/index.js", { source: PROJECT, from: "tests/u.js", how: "renamed", renamedFrom: "fixtures/u/index.js" }],
        ["fixtures/renamed/u/package.json", { source: PROJECT, from: "tests/u/package.json", how: "renamed", renamedFrom: "fixtures/u/package.json" }],
      ]),
    );
  });

  test("lets an override replace a path's entry", () => {
    const entries = variantEntries(originals, recorded, (path, entry) => (path.endsWith("/package.json") ? { ...originals.get("fixtures/u/package.json")! } : entry));
    assert.deepEqual(entries.get("fixtures/renamed/u/package.json"), { source: PROJECT, from: "tests/u/package.json", how: "verbatim" });
    assert.equal(entries.get("fixtures/renamed/u/index.js")?.how, "renamed");
  });

  test("throws for a variant whose original has no entry", () => {
    const partial = new Map([...originals].filter(([path]) => path !== "fixtures/u/package.json"));
    assert.throws(() => variantEntries(partial, recorded), /fixtures\/renamed\/u\/package\.json: a rename variant of fixtures\/u\/package\.json, which has no entry/u);
  });
});

describe("write_provenance (scripts/s3/pack_fixtures.py)", () => {
  const script = (root: string, entries: object): string =>
    [
      "import json, pathlib, sys",
      `sys.path.insert(0, ${JSON.stringify(join(REPOSITORY, "scripts", "s3"))})`,
      "import pack_fixtures",
      `pack_fixtures.ROOT = pathlib.Path(${JSON.stringify(root)})`,
      `pack_fixtures.write_provenance(${JSON.stringify(PACK)}, ${JSON.stringify(TOOL)}, json.loads(${JSON.stringify(JSON.stringify(entries))}), json.loads(${JSON.stringify(JSON.stringify(COMMITS))}))`,
    ].join("\n");

  test("writes through the node writer what writeProvenance writes", () => {
    const root = checkout();
    const expected = provenanceTexts({ root, pack: PACK, tool: TOOL, entries: ENTRIES, commits: COMMITS });
    const run = spawnSync("python3", ["-B", "-c", script(root, ENTRIES)], { encoding: "utf8" });
    assert.equal(run.status, 0, run.stderr);
    assert.equal(readFileSync(join(root, TOOL_DIRECTORY, PROVENANCE), "utf8"), expected.provenance);
    assert.equal(readFileSync(join(root, TOOL_DIRECTORY, LICENSES), "utf8"), expected.licenses);
  });

  test("stops the generator with the writer's refusal", () => {
    const root = checkout();
    const run = spawnSync("python3", ["-B", "-c", script(root, { "fixtures/missing.js": ENTRIES["fixtures/B.js"] })], { encoding: "utf8" });
    assert.equal(run.status, 1);
    assert.match(run.stderr, /fixtures\/missing\.js: an entry for a file the tool directory does not hold/u);
    assert.equal(existsSync(join(root, TOOL_DIRECTORY, PROVENANCE)), false);
  });
});

describe("assertProvenanceWritten (derive)", () => {
  test("fails a derivation whose generator wrote either output not at all", () => {
    const root = checkout();
    assert.throws(() => assertProvenanceWritten(TOOL_DIRECTORY, root), new RegExp(`^Error: ${TOOL_DIRECTORY}: the generator wrote no provenance \\(provenance\\.json and LICENSES\\.md missing\\)`, "u"));
    writeFileSync(join(root, TOOL_DIRECTORY, PROVENANCE), "{}\n");
    assert.throws(() => assertProvenanceWritten(TOOL_DIRECTORY, root), /the generator wrote no provenance \(LICENSES\.md missing\)/u);
    writeFileSync(join(root, TOOL_DIRECTORY, LICENSES), "# x\n");
    assertProvenanceWritten(TOOL_DIRECTORY, root);
  });
});
