import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test, vi } from "vitest";

import type * as Cache from "../../../conformance/port-readiness/cache.js";
import type { CorpusFile } from "../../../conformance/port-readiness/corpus.js";
import { packTree } from "../../../conformance/port-readiness/recording/archive.js";
import { decodeSource, upstreamCheckoutDirectory } from "../../../conformance/port-readiness/recorded-adapter.js";
import type { ParserVerdict, UpstreamCase, UpstreamFile } from "../../../conformance/port-readiness/types.js";
import { recordToolPack, type AdapterFactory } from "../../../conformance/recorders/port-readiness/record.js";
import { demoToolPack, fakeAdapter, MANIFEST, nameRules, optionsFor, span } from "./support/demo-tool-pack.js";

// The digest of an adapter's code (archive.ts's ADAPTER_SOURCES) is part of every config id and parsers id, and moves
// with any edit to an adapter, which is its own reason to record again and no change to what the recorder writes. It is
// one value here, so the pin below moves only with the recorder's format.
vi.mock("../../../conformance/port-readiness/cache.js", async (importOriginal) => ({
  ...(await importOriginal<typeof Cache>()),
  trackedDigest: () => "0".repeat(64),
}));

/**
 * The canonical tar sha256 of the trees the recorder writes for the fixture tool-packs below. It moves exactly when the
 * recorder writes a different tree for the same upstream output, or RECORDING_FORMAT moves (the config id is in every
 * path).
 */
const PINNED_TREE = "71a872efcdf5dfcc9809597f0d4d258d0637f7fc7d954054f8b9b4932adcaab7";

const sha256 = (value: string | Buffer): string => createHash("sha256").update(value).digest("hex");
const RUFF_FIXTURES = "crates/ruff_linter/resources/test/fixtures";
const BOM = `\uFEFF${"import pickle\n"}`;
const LATIN1 = "# -*- coding: latin-1 -*-\nprint('caf\u00e9')\n";

/** Upstream's bytes of each Ruff test file a case names: UTF-8, but for one with a byte order mark and one in Latin-1. */
const RUFF_BYTES: Record<string, Buffer> = {
  [`${RUFF_FIXTURES}/pyflakes/F401_0.py`]: Buffer.from("import os\nimport sys\n", "utf8"),
  [`${RUFF_FIXTURES}/pyflakes/F401_1.py`]: Buffer.from("from typing import List\n", "utf8"),
  [`${RUFF_FIXTURES}/pyflakes/F401_2.ipynb`]: Buffer.from("{\"cells\": []}\n", "utf8"),
  [`${RUFF_FIXTURES}/pylint/import_self.py`]: Buffer.from("import import_self\n", "utf8"),
  [`${RUFF_FIXTURES}/flake8_simplify/SIM105_4.py`]: Buffer.from(BOM, "utf8"),
  [`${RUFF_FIXTURES}/pyflakes/F401_latin1.py`]: Buffer.from(LATIN1, "latin1"),
};

/** A Ruff case: its source decoded from upstream's bytes as the live Bandit adapter decodes its examples (decodeSource). */
const ruffCase = (file: string, rest: Omit<UpstreamCase, "fixture" | "source" | "path"> & { path?: string }): UpstreamCase => ({
  fixture: `${RUFF_FIXTURES}/${file}`,
  path: file.split("/").pop()!,
  source: decodeSource(RUFF_BYTES[`${RUFF_FIXTURES}/${file}`]!),
  ...rest,
});

const RUFF_CASES: Record<string, UpstreamCase[]> = {
  F401: [
    // Findings and a written expectation, findings of both span shapes.
    ruffCase("pyflakes/F401_0.py", { variant: "default", skipped: null, expected: [{ startLine: 2, startCol: 7, endLine: 2, endCol: 10 }, span], markup: [span] }),
    // Read in another language at its path, as an ECMAScript script would be: upstream silent.
    ruffCase("pyflakes/F401_1.py", { variant: "typing", parsedAs: ".pyi", script: true, skipped: null, expected: [], markup: null }),
    // Not linted by upstream.
    ruffCase("pyflakes/F401_2.ipynb", { variant: "default", skipped: "upstream does not lint notebooks here", expected: [], markup: null }),
    // Under a not-copied prefix: answered in the index.
    ruffCase("pylint/import_self.py", { variant: "default", skipped: null, expected: [span], markup: null }),
    ruffCase("pyflakes/F401_latin1.py", { variant: "default", skipped: null, expected: [], markup: null }),
  ],
  SIM105: [ruffCase("flake8_simplify/SIM105_4.py", { variant: "default", skipped: null, expected: [span], markup: null })],
};

/** An upstream test file the fixture texts name by path, beside the inline texts (one with a lone surrogate). */
const FIXTURE_FILE = { path: `${RUFF_FIXTURES}/pyflakes/F401_0.py`, text: "import os\nimport sys\n" };
const INLINE_TEXTS = ["x = '\uD800'\n", "import os\n"];

/** Verdicts on a source: a syntax error on one starting with a byte order mark, silence on the rest. */
const verdictsOn = (parser: string) => async (files: { source: string }[]): Promise<ParserVerdict[][]> =>
  files.map((file) => [{ parser, syntax: file.source.startsWith("\uFEFF") ? ["U+FEFF at 1:0"] : [], grammar: file.source.includes("caf\u00e9") ? ["a grammar error"] : [] }]);

const ruffAdapter: AdapterFactory = async (...args) => ({
  ...(await fakeAdapter(...args)),
  identity: { ruff: "0.16.9" },
  rule: (name) => (name in RUFF_CASES ? { name, implementation: [`crates/ruff_linter/src/rules/${name}.rs`], tests: [`${RUFF_FIXTURES}/${name}`], deprecation: null, recommended: name === "F401" ? { config: "default", options: [{ ignore: ["__init__"] }] } : null, notOnRealCode: null } : undefined),
  implementationNames: (name) => new Set(name === "F401" ? ["unused", "import"] : ["contextlib", "suppress"]),
  sharedImplementationNames: (name) => new Set(name === "F401" ? ["binding"] : []),
  implementationRegexes: (name) => (name === "F401" ? [/^__\w+__$/u, /import/iu] : []),
  fixtureWords: () => new Set(["import", "os", "sys", "caf\u00e9"]),
  fixtureTexts: () => [FIXTURE_FILE.text, ...INLINE_TEXTS],
  fixtureTextSources: () => [{ path: FIXTURE_FILE.path, sha256: sha256(FIXTURE_FILE.text) }, ...INLINE_TEXTS.map((inline) => ({ inline }))],
  keywords: new Set(["import", "from"]),
  commentVocabulary: new Set(["noqa", "type"]),
  cases: (name) => RUFF_CASES[name] ?? [],
  toSpan: (_name, _source, start, end) => ({ startLine: 1, startCol: start, endLine: 1, endCol: end }),
  lintCorpus: async (files: CorpusFile[], names: string[]): Promise<UpstreamFile[]> => files.map((file) => ({
    file,
    excluded: file.path.endsWith("broken.py") ? "upstream's parser rejects it" : null,
    findings: new Map(names.map((name) => [name, file.path === "pkg/mod.py" ? [span, { startLine: 3, startCol: 1, endLine: null, endCol: null }] : []])),
  })),
  parserVerdicts: verdictsOn("cpython 3.14.0"),
});

const phpstanAdapter: AdapterFactory = async (...args) => ({
  ...(await fakeAdapter(...args)),
  identity: { phpstan: "2.2.16" },
  rule: (name) => (name === "variable.undefined" || name === "array.duplicateKey" ? { name, implementation: ["src/Rules/X.php"], tests: ["tests/PHPStan/Rules/XTest.php"], deprecation: null, recommended: { config: "level0", options: [] }, notOnRealCode: null } : undefined),
  // Every ported rule's data file is a case of each rule, as PHPStan's are.
  cases: (name) => [
    { fixture: "tests/PHPStan/Rules/data/undefined.php", variant: "testRule", path: "undefined.php", source: "<?php echo $a;\n", skipped: null, expected: name === "variable.undefined" ? [{ startLine: 1, startCol: null, endLine: null, endCol: null }] : [], markup: null },
    { fixture: "tests/PHPStan/Rules/data/keys.php", variant: "testRule", path: "keys.php", source: "<?php [1 => 1, 1 => 2];\n", skipped: null, expected: name === "array.duplicateKey" ? [{ startLine: 1, startCol: null, endLine: null, endCol: null }] : [], markup: null },
  ],
  toSpan: () => ({ startLine: 1, startCol: null, endLine: null, endCol: null }),
  inCorpus: (path) => path.endsWith(".php"),
  // The answer depends on the batch: a finding on the first line for every file handed with another.
  lintCorpus: async (files: CorpusFile[], names: string[]): Promise<UpstreamFile[]> => files.map((file) => ({
    file, excluded: null, findings: new Map(names.map((name) => [name, files.length > 1 && name === "variable.undefined" ? [{ startLine: 1, startCol: null, endLine: null, endCol: null }] : []])),
  })),
  parserVerdicts: verdictsOn("nikic/php-parser 5.9.0"),
});

const place = (path: string, digit: string, task = "demo-task", source = "x = 1\n"): CorpusFile => ({ task, repository: "demo/demo", path, source, sha256: digit.repeat(64) });

/** The tree the recorder writes for one fixture tool-pack of `adapter`, recorded from nothing. */
async function recordedTree(adapter: "ruff" | "phpstan", rules: string[], factory: AdapterFactory, goldens: CorpusFile[], notCopied: string | null, bytes: (fixture: string) => Buffer | undefined): Promise<Map<string, Buffer>> {
  const root = demoToolPack(rules);
  try {
    writeFileSync(join(root, "packs/demo/upstream/bandit/gates.yaml"), MANIFEST.replace("adapter: bandit", `adapter: ${adapter}`));
    const upstreamMd = join(root, "packs/demo/licenses/bandit/UPSTREAM.md");
    if (notCopied !== null) writeFileSync(upstreamMd, `${readFileSync(upstreamMd, "utf8")}\n- Not copied: \`${notCopied}\` — a copyleft project\n`);
    nameRules(root, rules);
    const options = { ...optionsFor(root, factory, () => goldens, adapter), upstreamBytes: bytes };
    // The fixture text file, at the pinned commit's checkout the gates rebuild the texts from.
    const checkout = upstreamCheckoutDirectory(options.cacheRoot, { repository: "https://github.com/PyCQA/bandit", commit: "a".repeat(40) });
    mkdirSync(dirname(join(checkout, FIXTURE_FILE.path)), { recursive: true });
    writeFileSync(join(checkout, FIXTURE_FILE.path), FIXTURE_FILE.text);
    return (await recordToolPack(options)).tree;
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

test("the recorder writes the pinned tree for a fixture tool-pack that exercises every file it writes", async () => {
  // Ruff: per-rule records; a corpus answer keyed by the __init__.py files above its file; findings, silence and an exclusion.
  const ruff = await recordedTree("ruff", ["F401", "SIM105", "X999"], ruffAdapter, [
    place("pkg/__init__.py", "1"), place("pkg/mod.py", "2"), place("pkg/empty.py", "3"), place("pkg/broken.py", "4"), place("top.py", "5"),
  ], `${RUFF_FIXTURES}/pylint/`, (fixture) => RUFF_BYTES[fixture]);
  // PHPStan: case keys carry the recorded rules; corpus answers are keyed by the task's batch and the rules.
  const phpstan = await recordedTree("phpstan", ["array.duplicateKey", "variable.undefined"], phpstanAdapter, [
    place("src/a.php", "6", "t1", "<?php echo $a;\n"), place("src/b.php", "7", "t1", "<?php\n"), place("src/c.php", "8", "t2", "<?php\n"),
  ], null, () => undefined);
  const tree = new Map([...ruff, ...phpstan]);
  assert.equal(tree.size, ruff.size + phpstan.size, "the two fixture tool-packs share no path");
  const paths = [...tree.keys()];
  const text = (pattern: RegExp) => paths.filter((path) => pattern.test(path)).map((path) => tree.get(path)!.toString("utf8"));
  assert.ok(text(/^config\//u).every((config) => config.includes(`"sources": "${"0".repeat(64)}"`)), "the adapter sources digest is not the one this test pins it to");
  const actual = packTree(tree).sha256;
  assert.equal(actual, PINNED_TREE, "the recorder now writes a different tree for the same upstream output: if that is intended, bump RECORDING_FORMAT in "
    + `conformance/port-readiness/recording/archive.ts (every tool-pack then records again) and update this pin to ${actual}`);

  // Every kind of file the recorder writes is in the fixture, so the pin covers them all.
  for (const [kind, pattern] of [
    ["config", /^config\/[0-9a-f]{64}\.json$/u],
    ["tool facts", /^tool\/[0-9a-f]{64}\/fixtureWords\.json$/u],
    ["fixture texts", /^tool\/[0-9a-f]{64}\/fixtureTexts\.json$/u],
    ["absent rules", /^tool\/[0-9a-f]{64}\/absent\.json$/u],
    ["name sets", /^tool\/[0-9a-f]{64}\/name-sets\/regexes~[0-9a-f]{16}\.json$/u],
    ["shared name sets", /^tool\/[0-9a-f]{64}\/name-sets\/shared-names~[0-9a-f]{16}\.json$/u],
    ["rule.json", /^rules\/[0-9a-f]{64}\/F401\/rule\.json$/u],
    ["case answers", /^rules\/[0-9a-f]{64}\/F401\/cases\/[0-9a-f]{64}\+[0-9a-f]{12}\.json$/u],
    ["texts", /^texts\/[0-9a-f]{64}\.json$/u],
    ["corpus answers", /^answers\/[0-9a-f]{64}\/F401\/demo-task\/pkg\/mod\.py@2{64}\+[0-9a-f]{12}\.json$/u],
    ["batched corpus answers", /^answers\/[0-9a-f]{64}\/variable\.undefined\/t1\/src\/a\.php@6{64}\+[0-9a-f]{12}\.json$/u],
    ["verdicts", /^verdicts\/[0-9a-f]{64}\/[0-9a-f]{64}\.py\.json$/u],
    ["script verdicts", /^verdicts\/[0-9a-f]{64}\/[0-9a-f]{64}\.pyi\.script\.json$/u],
  ] as const) assert.ok(paths.some((path) => pattern.test(path)), `the fixture tree has no ${kind} file`);
  const ruleJson = text(/\/F401\/rule\.json$/u)[0]!;
  for (const field of ["\"parsedAs\": \".pyi\"", "\"script\": true", "\"markup\": [", "\"skipped\": \"not-copied\"", "\"variant\": \"typing\""]) assert.ok(ruleJson.includes(field), `rule.json lacks ${field}`);
  assert.ok(text(/^rules\/.*\/cases\//u).some((answer) => answer.includes("\"skipped\": \"upstream does not lint notebooks here\"")), "no skipped case answer");
  assert.ok(text(/^rules\/.*\/cases\//u).some((answer) => answer.includes("\"expected\": []")), "no empty case answer");
  assert.ok(text(/^texts\//u).includes("\"x = '\\ud800'\\n\"\n"), "no text with a lone surrogate");
  assert.ok(text(/^answers\//u).includes('{\n  "excluded": "upstream\'s parser rejects it"\n}\n'), "no excluded corpus answer");
  assert.ok(text(/^answers\//u).includes('{\n  "findings": []\n}\n'), "no silent corpus answer");
  // The parsers' verdicts on the byte-order-marked and the Latin-1 case sources, keyed by their decoded text.
  for (const source of [BOM, LATIN1]) assert.ok(paths.some((path) => path.includes(`/${sha256(source)}.py.json`)), `no verdict on ${JSON.stringify(source)}`);
}, 60_000);
