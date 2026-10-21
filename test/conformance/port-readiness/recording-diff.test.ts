/**
 * The per-rule summary of what moved between two generations of a tool-pack's answers (recording/diff.ts): what a
 * pull request shows in place of the answers themselves, which are not in the tree.
 */
import assert from "node:assert/strict";
import { test } from "vitest";

import { diffRecordings, renderRecordingChanges, type RecordedPair } from "../../../conformance/port-readiness/recording/diff.js";
import { gatesCorpusSchema, gatesRecordingSchema, CORPUS_FORMAT, RECORDING_FORMAT, recordingText } from "../../../conformance/port-readiness/recording/schema.js";

const PARSERS = [{ parser: "cpython 3.14.8", grammar: true }];
const PINS = { repository: "https://github.com/PyCQA/bandit", commit: "a".repeat(40), version: "1.9.4", runtime: "bandit-1.9.4" };

function recordingOf(rules: Record<string, unknown>): ReturnType<typeof gatesRecordingSchema.parse> {
  return gatesRecordingSchema.parse({
    format: RECORDING_FORMAT,
    tool: "bandit",
    adapter: "bandit",
    pins: PINS,
    runtimes: { "bandit-1.9.4": { "pyproject.toml": "d".repeat(64) } },
    identity: { bandit: "1.9.4" },
    packChecks: [],
    licenseSha256: "0".repeat(64),
    inScopeSamples: ["src/example.py"],
    corpus: { extensions: [".py"], caseSensitive: true },
    keywords: [],
    commentVocabulary: [],
    fixtureWords: [],
    nameSets: {},
    rules,
    absent: [],
    parsers: PARSERS,
    parserVerdicts: [],
  });
}

const rule = (expected: [number, number | null, number | null, number | null][], tests: string[] = ["examples/pickle.py"], recommended: { config: string; options: unknown[] } | null = null) => ({
  facts: { name: "B301", implementation: ["bandit/blacklists/calls.py"], tests, deprecation: null, recommended, notOnRealCode: null },
  names: { implementation: { sets: [], names: [], regexes: [] }, shared: { sets: [], names: [], regexes: [] } },
  spanParts: { endLine: true, endCol: true },
  cases: [{ fixture: "examples/pickle.py", variant: "default", path: "pickle.py", skipped: null, expected, markup: null, file: "fixtures/pickle.py" }],
});

function corpusOf(findings: Record<string, [number, number | null, number | null, number | null][]>): ReturnType<typeof gatesCorpusSchema.parse> {
  return gatesCorpusSchema.parse({
    format: CORPUS_FORMAT,
    tool: "bandit",
    pins: { tasks: "c".repeat(64), corpora: [] },
    rules: Object.keys(findings).sort(),
    parsers: PARSERS,
    files: [{ task: "demo", path: "app.py", sha256: "b".repeat(64), excluded: null, findings }],
  });
}

test("the per-rule summary names what moved, and says so plainly when nothing did", () => {
  const before: RecordedPair = { recording: recordingOf({ B301: rule([[1, 0, 1, 17]]), B302: rule([[2, 0, 2, 4]]) }), corpus: corpusOf({ B301: [[3, 0, 3, 9]] }) };
  const after: RecordedPair = { recording: recordingOf({ B301: rule([[1, 0, 1, 17], [5, 0, 5, 8]], ["examples/pickle.py", "examples/more.py"]) }), corpus: corpusOf({ B301: [[4, 0, 4, 9]] }) };

  const unchanged = diffRecordings("demo", "bandit", before, before);
  assert.deepEqual(unchanged.rules, []);
  assert.deepEqual(unchanged.pinsChanged, []);
  assert.match(renderRecordingChanges([unchanged]), /regenerated, and upstream answers the same/u);
  assert.match(renderRecordingChanges([unchanged]), /`demo\/bandit`: unchanged/u);
  // The Composer packages a generation resolved are listed, so a transitive drift shows, without making it a change.
  const resolved = renderRecordingChanges([diffRecordings("demo", "bandit", before, before, [{ runtime: "php", packages: ["nikic/php-parser@v5.9.0"] }])]);
  assert.match(resolved, /regenerated, and upstream answers the same/u);
  assert.match(resolved, /^- `runtimes\/php`: `nikic\/php-parser@v5\.9\.0`$/mu);
  assert.doesNotMatch(renderRecordingChanges([unchanged]), /Composer packages/u);

  const changes = diffRecordings("demo", "bandit", before, after);
  assert.deepEqual(changes.rules, [
    { rule: "B301", status: "changed", expectedAdded: 1, expectedRemoved: 0, corpusFilesChanged: 1, factsChanged: ["tests"] },
    { rule: "B302", status: "removed", expectedAdded: 0, expectedRemoved: 0, corpusFilesChanged: 0, factsChanged: [] },
  ]);
  const rendered = renderRecordingChanges([changes]);
  assert.match(rendered, /1 tool-pack moved/u);
  assert.match(rendered, /\| `B301` \| changed \| \+1 \/ −0 \| 1 \| `tests` \|/u);
  assert.match(rendered, /\| `B302` \| removed \| — \| — \| — \|/u);
  // A regeneration that answers fewer files (the generator answers only the files goldens name) is not a change in
  // upstream's answers: only files both answers cover are compared.
  const whole = corpusOf({ B301: [[3, 0, 3, 9]] });
  const wider: RecordedPair = { ...before, corpus: { ...whole, files: [...whole.files, { ...whole.files[0]!, task: "dropped", findings: { B301: [[1, 0, 1, 9]] } }] } };
  assert.deepEqual(diffRecordings("demo", "bandit", wider, before).rules, []);
  // A first generation has nothing to compare with, and says that rather than listing every rule as a change.
  assert.match(renderRecordingChanges([diffRecordings("demo", "bandit", null, after)]), /first generation/u);
  // Main's answers that could not be read are neither: the summary warns that what moved is unknown.
  const uncompared = renderRecordingChanges([diffRecordings("demo", "bandit", { unavailable: "no answer store is configured" }, after)]);
  assert.match(uncompared, /^## Port readiness answers: ⚠️ 1 tool-pack not compared$/mu);
  assert.match(uncompared, /`demo\/bandit`: ⚠️ not compared\n\n.*\(no answer store is configured\)/u);
  assert.doesNotMatch(uncompared, /: first generation|✅/u);
});

test("the per-rule summary calls a fact changed only when its value changed, whatever order its object keys are in", () => {
  // Bandit's shell_injection options, in the order its gen_config builds them, which is how the generator holds fresh
  // answers in memory; the nested object stands for an ESLint rule's options. Stored answers are read back from their
  // key-sorted text, so the same options come back with every key sorted.
  const options = (shell: string[]) => [{ subprocess: ["subprocess.Popen"], shell, no_shell: ["os.execl"] }, { allow: true, ignore: { patterns: ["^_"], names: ["x"] } }];
  const recorded = (shell: string[]): RecordedPair => ({ recording: recordingOf({ B602: rule([[1, 0, 1, 17]], ["examples/subprocess_shell.py"], { config: "default", options: options(shell) }) }), corpus: null });
  const fresh = recorded(["os.system"]);
  const stored: RecordedPair = { recording: gatesRecordingSchema.parse(JSON.parse(recordingText(fresh.recording))), corpus: null };
  // The two hold the same options in different key orders, so the comparison below is not vacuous.
  assert.notEqual(JSON.stringify(stored.recording.rules.B602!.facts.recommended), JSON.stringify(fresh.recording.rules.B602!.facts.recommended));

  // Run 39319650531 listed `recommended` as changed for nine Bandit rules whose recordings were identical.
  assert.deepEqual(diffRecordings("demo", "bandit", stored, fresh).rules, []);
  assert.deepEqual(diffRecordings("demo", "bandit", fresh, stored).rules, []);
  assert.match(renderRecordingChanges([diffRecordings("demo", "bandit", stored, fresh)]), /`demo\/bandit`: unchanged/u);

  // A value that did change still does, and a list is a value whose order counts: only object keys are unordered.
  const changed = [{ rule: "B602", status: "changed", expectedAdded: 0, expectedRemoved: 0, corpusFilesChanged: 0, factsChanged: ["recommended"] }];
  assert.deepEqual(diffRecordings("demo", "bandit", stored, recorded(["os.system", "os.popen"])).rules, changed);
  assert.deepEqual(diffRecordings("demo", "bandit", recorded(["os.popen", "os.system"]), recorded(["os.system", "os.popen"])).rules, changed);
});

test("the per-rule summary calls a fact the older answers lack changed, instead of failing the generation", () => {
  // The schema requires every fact today, but a field added later would be optional for the answers made before it,
  // so a regeneration would compare fresh answers that have it with stored ones that do not.
  const fresh: RecordedPair = { recording: recordingOf({ B301: rule([[1, 0, 1, 17]]) }), corpus: null };
  const older = gatesRecordingSchema.parse(JSON.parse(recordingText(fresh.recording)));
  delete (older.rules.B301!.facts as Record<string, unknown>).notOnRealCode;

  assert.deepEqual(diffRecordings("demo", "bandit", { recording: older, corpus: null }, fresh).rules, [
    { rule: "B301", status: "changed", expectedAdded: 0, expectedRemoved: 0, corpusFilesChanged: 0, factsChanged: ["notOnRealCode"] },
  ]);
});
