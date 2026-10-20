// conformance/port-readiness/recording/record-plan.mjs: the tool-packs a record-port-readiness label records, the ones
// whose recorder inputs the pull request changes, and port-readiness-record.yml's authorize job, which runs it.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

import { afterAll, describe, it } from "vitest";
import { parse } from "yaml";

import { gatedToolPacks } from "../../../conformance/port-readiness/plan.mjs";
import { commentBody, dispatchPlan, labelPlan, NOT_COPIED, notCopiedOf, parseToolPacks, ROOT, writeOutputs, type RecordPlan } from "../../../conformance/port-readiness/recording/record-plan.mjs";
import { notCopiedPrefixes } from "../../../conformance/recorders/port-readiness/record.js";

const temporary: string[] = [];
afterAll(() => {
  for (const directory of temporary.splice(0)) rmSync(directory, { recursive: true, force: true });
});

function tempRoot(): string {
  const root = mkdtempSync(join(tmpdir(), "rerail-record-plan-"));
  temporary.push(root);
  return root;
}

function write(root: string, path: string, text = "\n"): void {
  mkdirSync(join(root, dirname(path)), { recursive: true });
  writeFileSync(join(root, path), text);
}

const TOOL_PACKS = [{ pack: "python", tool: "ruff" }, { pack: "python", tool: "bandit" }, { pack: "javascript", tool: "eslint" }];
const RUFF = "python/ruff";
const BANDIT = "python/bandit";
const ESLINT = "javascript/eslint";
const names = (planned: Pick<RecordPlan, "legs">): string[] => planned.legs.map(({ pack, tool }) => `${pack}/${tool}`);

/**
 * A Markdown text's code spans as CommonMark pairs backticks (a run opens one, the next run exactly as long closes it,
 * and one space comes off each side of a text that starts and ends with one and is not all spaces), and the text
 * outside them.
 */
function codeSpans(markdown: string): { spans: string[]; outside: string } {
  const spans: string[] = [];
  let outside = "";
  let from = 0;
  const runs = /`+/gu;
  for (let open = runs.exec(markdown); open !== null; open = runs.exec(markdown)) {
    let close = runs.exec(markdown);
    while (close !== null && close[0].length !== open[0].length) close = runs.exec(markdown);
    if (close === null) {
      runs.lastIndex = open.index + open[0].length;
      continue;
    }
    const text = markdown.slice(open.index + open[0].length, close.index);
    spans.push(text.startsWith(" ") && text.endsWith(" ") && /[^ ]/u.test(text) ? text.slice(1, -1) : text);
    outside += markdown.slice(from, open.index);
    from = close.index + close[0].length;
  }
  return { spans, outside: outside + markdown.slice(from) };
}

const UPSTREAM_MD = [
  "# Ruff",
  "",
  "- Not copied: `crates/ruff_linter/resources/test/fixtures/pylint` — derived from pylint, GPL-2.0-or-later",
  "",
  "| File | Upstream path | Copy |",
  "|---|---|---|",
  "| `upstream/ruff/fixtures/pyflakes/F401_0.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F401_0.py` | verbatim |",
  "",
].join("\n");

const CORPUS = { name: "django", repository: "https://github.com/django/django", commit: "a".repeat(40), paths: ["django/db", "django/conf"], exclude: [] };
const manifest = (rules: { id: string; tool: string; rule: string; policy?: string }[], corpora?: unknown[]): string =>
  `${JSON.stringify({ ...(corpora === undefined ? {} : { corpora }), rules: rules.map(({ id, tool, rule, policy }) => ({ id, upstream: { tool, rule }, policy: policy ?? `rules/${id}.json` })) }, null, 2)}\n`;
const PACK_RULES = [{ id: "ruff-f401", tool: "ruff", rule: "F401" }, { id: "bandit-b101", tool: "bandit", rule: "B101" }];
const CANDIDATE_RULES = [{ id: "ruff-e501", tool: "ruff", rule: "E501" }];

// A checkout of its own: two gated packs with what the recorder reads, then main moves on after the pull requests
// below branch off, so a selection that diffs main's tip instead of the merge base would select javascript/eslint too.
const root = tempRoot();
const git = (...args: string[]): string =>
  execFileSync("git", ["-c", "user.name=gates", "-c", "user.email=gates@example.test", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", ...args], { cwd: root, encoding: "utf8" }).trim();
git("init", "-q", "-b", "main");
for (const tool of ["ruff", "bandit"]) write(root, `packs/python/upstream/${tool}/gates.yaml`, `adapter: ${tool}\n`);
write(root, "packs/python/upstream/ruff/goldens.tsv", "task\tpath\n");
write(root, "packs/python/upstream/ruff/fixtures/pyflakes/F401_0.py", "import os\n");
write(root, "packs/python/upstream/ruff/generate.py", "print()\n");
write(root, "packs/python/conformance.json", manifest(PACK_RULES, [CORPUS]));
write(root, "packs/python/candidates/conformance.json", manifest(CANDIDATE_RULES));
write(root, "packs/python/licenses/ruff/UPSTREAM.md", UPSTREAM_MD);
write(root, "packs/python/licenses/bandit/UPSTREAM.md", "# Bandit\n\n| File | Upstream path | Copy |\n|---|---|---|\n");
write(root, "packs/javascript/upstream/eslint/gates.yaml", "adapter: eslint\n");
write(root, "packs/javascript/upstream/eslint/goldens.tsv", "task\tpath\n");
write(root, "conformance/port-readiness/corpus-tasks.json", '{ "tasks": [] }\n');
write(root, "conformance/recorders/port-readiness/runtimes/ruff-0.16.9/pyproject.toml", "[project]\n");
write(root, "conformance/recorders/port-readiness/record.ts", "export {};\n");
mkdirSync(join(root, "conformance", "port-readiness", "recording"), { recursive: true });
copyFileSync(join(ROOT, "conformance", "port-readiness", "recording", "record-plan.mjs"), join(root, "conformance", "port-readiness", "recording", "record-plan.mjs"));
git("add", "-A");
git("commit", "-q", "-m", "base");
const FORK = git("rev-parse", "HEAD");
write(root, "packs/javascript/upstream/eslint/goldens.tsv", "task\tpath\nmain-task\tsrc/a.js\n");
git("commit", "-q", "-am", "main moves on");
const MAIN = git("rev-parse", "HEAD");

let branches = 0;
/** A pull request off the fork point: each path written, or deleted when its text is null; returns its head. */
function pullRequest(files: Record<string, string | null>): string {
  branches += 1;
  git("checkout", "-q", "-b", `pr-${branches}`, FORK);
  for (const [path, text] of Object.entries(files)) {
    if (text === null) rmSync(join(root, path));
    else write(root, path, text);
  }
  git("add", "-A");
  git("commit", "-q", "--allow-empty", "-m", `pull request ${branches}`);
  const head = git("rev-parse", "HEAD");
  git("checkout", "-q", "main");
  return head;
}

const planFor = (files: Record<string, string | null>): RecordPlan => labelPlan(TOOL_PACKS, MAIN, pullRequest(files), root);

describe("the tool-packs a label records", () => {
  it("diffs the head against its merge base with the base, not against the base's tip", () => {
    const planned = planFor({ "packs/python/upstream/ruff/goldens.tsv": "task\tpath\npr-task\tapp.py\n" });
    assert.deepEqual(names(planned), [RUFF]);
    assert.deepEqual(planned.lines, ["python/ruff: `packs/python/upstream/ruff/goldens.tsv` changed"]);
    assert.deepEqual(planned.warnings, []);
  });

  it("a tool-pack's gates.yaml selects it", () => {
    assert.deepEqual(names(planFor({ "packs/python/upstream/bandit/gates.yaml": "adapter: bandit\nknown_gaps: []\n" })), [BANDIT]);
  });

  it("a file under a tool-pack's fixtures/ selects it", () => {
    const planned = planFor({ "packs/python/upstream/ruff/fixtures/pyflakes/F401_1.py": "import sys\n", "packs/python/upstream/ruff/fixtures/pyflakes/F401_2.py": "import re\n" });
    assert.deepEqual(names(planned), [RUFF]);
    assert.deepEqual(planned.lines, ["python/ruff: `packs/python/upstream/ruff/fixtures/pyflakes/F401_1.py` changed (and 1 more)"]);
  });

  it("a tool-pack's other files select nothing: the recorder does not read them", () => {
    const planned = planFor({ "packs/python/upstream/ruff/generate.py": "print(1)\n", "packs/python/upstream/ruff/recording.lock": "{}\n", "packs/python/README.md": "# Python\n" });
    assert.deepEqual(planned.legs, []);
    assert.deepEqual(planned.lines, ["No tool-pack's recorder inputs changed: nothing to record."]);
  });

  it("a changed `- Not copied:` line selects every tool-pack of the pack", () => {
    const planned = planFor({ "packs/python/licenses/bandit/UPSTREAM.md": "# Bandit\n\n- Not copied: `examples/pickle` — GPL-3.0-only\n\n| File | Upstream path | Copy |\n|---|---|---|\n" });
    assert.deepEqual(names(planned), [RUFF, BANDIT]);
    assert.match(planned.lines[0]!, /^python\/ruff: the "Not copied" lines of packs\/python\/licenses\/\*\/UPSTREAM\.md changed/u);
  });

  it("a row the recorder adds to an UPSTREAM.md table, or a reworded Not-copied reason, selects nothing", () => {
    const rows = UPSTREAM_MD.replace("verbatim |\n", "verbatim |\n| `upstream/ruff/fixtures/pyflakes/F401_1.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F401_1.py` | verbatim |\n");
    assert.deepEqual(planFor({ "packs/python/licenses/ruff/UPSTREAM.md": rows.replace("GPL-2.0-or-later", "GPL-2.0-or-later, so read in place") }).legs, []);
  });

  it("an upstream rule of a tool in either conformance manifest selects that tool-pack alone", () => {
    const pack = planFor({ "packs/python/conformance.json": manifest([...PACK_RULES, { id: "ruff-f841", tool: "ruff", rule: "F841" }], [CORPUS]) });
    assert.deepEqual(names(pack), [RUFF]);
    assert.deepEqual(pack.lines, ["python/ruff: the ruff rules packs/python/conformance.json and candidates/conformance.json name changed (added `F841`)"]);
    const candidates = planFor({ "packs/python/candidates/conformance.json": manifest([...CANDIDATE_RULES, { id: "bandit-b301", tool: "bandit", rule: "B301" }]) });
    assert.deepEqual(names(candidates), [BANDIT]);
    const dropped = planFor({ "packs/python/conformance.json": manifest([PACK_RULES[1]!], [CORPUS]) });
    assert.deepEqual(dropped.lines, ["python/ruff: the ruff rules packs/python/conformance.json and candidates/conformance.json name changed (removed `F401`)"]);
  });

  it("a rule promoted from candidates/ to the pack's manifest selects nothing: the recorder reads both", () => {
    assert.deepEqual(planFor({
      "packs/python/conformance.json": manifest([...PACK_RULES, ...CANDIDATE_RULES], [CORPUS]),
      "packs/python/candidates/conformance.json": manifest([]),
    }).legs, []);
  });

  it("a change to the pack's corpora selects every tool-pack of the pack", () => {
    const planned = planFor({ "packs/python/candidates/conformance.json": manifest(CANDIDATE_RULES, [{ ...CORPUS, name: "flask", repository: "https://github.com/pallets/flask" }]) });
    assert.deepEqual(names(planned), [RUFF, BANDIT]);
    assert.match(planned.lines[1]!, /^python\/bandit: the corpora packs\/python\/conformance\.json and candidates\/conformance\.json pin changed$/u);
    // Read as the recorder reads them: paths and exclude sorted, so reordering them is no change.
    assert.deepEqual(planFor({ "packs/python/conformance.json": manifest(PACK_RULES, [{ ...CORPUS, paths: ["django/conf", "django/db"] }]) }).legs, []);
  });

  it("an edit to a field the recorder does not read selects nothing", () => {
    const edited = manifest(PACK_RULES.map((rule) => ({ ...rule, policy: `candidates/rules/${rule.id}.json` })), [CORPUS]).replace('"rules"', '"tools": [{ "name": "ruff", "version": "0.16.9" }],\n  "rules"');
    assert.deepEqual(planFor({ "packs/python/conformance.json": edited }).legs, []);
  });

  it("a manifest that does not parse at the head selects every tool-pack of the pack, with a warning", () => {
    const planned = planFor({ "packs/python/conformance.json": "{ not json\n" });
    assert.deepEqual(names(planned), [RUFF, BANDIT]);
    assert.equal(planned.warnings.length, 1);
    assert.match(planned.warnings[0]!, /^packs\/python\/conformance\.json does not parse at [0-9a-f]{10}, so every tool-pack of python is recorded: `[^`]+`$/u);
  });

  it("corpus-tasks.json selects every tool-pack", () => {
    assert.deepEqual(names(planFor({ "conformance/port-readiness/corpus-tasks.json": '{ "tasks": [1] }\n' })), [RUFF, BANDIT, ESLINT]);
  });

  it("a commit git cannot read selects every tool-pack, with a warning", () => {
    const planned = labelPlan(TOOL_PACKS, MAIN, "f".repeat(40), root);
    assert.deepEqual(names(planned), [RUFF, BANDIT, ESLINT]);
    assert.equal(planned.warnings.length, 1);
    assert.match(planned.warnings[0]!, new RegExp(`^git could not compare \`${MAIN}\` with \`${"f".repeat(40)}\`, so every tool-pack is recorded: \`[^\`]+\`$`, "u"));
    // A commit that is not given at all is still a span: a space, since a span cannot be empty.
    const unnamed = labelPlan(TOOL_PACKS, "", MAIN, root);
    assert.match(unnamed.warnings[0]!, new RegExp(`^git could not compare \` \` with \`${MAIN}\`, so every tool-pack is recorded: \`[^\`]+\`$`, "u"));
  });

  it("a tool-pack whose gates.yaml is gone at the head is not recorded", () => {
    const planned = planFor({ "packs/python/upstream/bandit/gates.yaml": null, "conformance/port-readiness/corpus-tasks.json": '{ "tasks": [2] }\n' });
    assert.deepEqual(names(planned), [RUFF, ESLINT]);
    assert.deepEqual(planned.dropped.map(({ pack, tool, why }) => [`${pack}/${tool}`, why]), [[BANDIT, "packs/python/upstream/bandit/gates.yaml is gone at the head"]]);
    assert.ok(planned.lines.includes("python/bandit: not recorded, packs/python/upstream/bandit/gates.yaml is gone at the head"));
  });

  it("a gates.yaml new at the head and not in TOOL_PACKS records nothing, with a warning: a new tool-pack lands on main first", () => {
    const planned = planFor({ "packs/node/upstream/eslint-plugin-n/gates.yaml": "adapter: eslint\n", "packs/node/upstream/eslint-plugin-n/goldens.tsv": "task\tpath\n" });
    assert.deepEqual(planned.legs, []);
    assert.equal(planned.warnings.length, 1);
    assert.match(planned.warnings[0]!, /^`packs\/node\/upstream\/eslint-plugin-n\/gates\.yaml` is not a tool-pack main's port-readiness-record\.yml records \(TOOL_PACKS\): .*lands on main first/u);
  });

  it("a change to the recorder's runtimes, or to its other code, records nothing here, with a warning", () => {
    const planned = planFor({
      "conformance/recorders/port-readiness/runtimes/ruff-0.16.9/pyproject.toml": "[project]\ndependencies = []\n",
      "conformance/recorders/port-readiness/record.ts": "export const x = 1;\n",
    });
    assert.deepEqual(planned.legs, []);
    assert.equal(planned.warnings.length, 2);
    assert.match(planned.warnings[0]!, /^`conformance\/recorders\/port-readiness\/runtimes\/ruff-0\.16\.9\/pyproject\.toml` changed: a label records with main's runtimes, so a runtime change lands on main first, and main is then recorded$/u);
    assert.match(planned.warnings[1]!, /^`conformance\/recorders\/port-readiness\/record\.ts` changed: a label records with main's recorder/u);
  });

  // A file the pull request names with a newline is a line of its own on stdout, where `::` starts a workflow command.
  it.skipIf(process.platform === "win32")("a path with a control character selects as any other, and is printed escaped", () => {
    const planned = planFor({ "packs/python/upstream/ruff/fixtures/a\n::error::b.py": "x\n", "conformance/recorders/port-readiness/runtimes/x\r\n::error::y": "z\n" });
    assert.deepEqual(names(planned), [RUFF]);
    assert.deepEqual(planned.lines, ["python/ruff: `packs/python/upstream/ruff/fixtures/a\\u000a:\\u003aerror:\\u003ab.py` changed"]);
    assert.deepEqual(planned.warnings.map((warning) => warning.split(" changed:")[0]), ["`conformance/recorders/port-readiness/runtimes/x\\u000d\\u000a:\\u003aerror:\\u003ay`"]);
  });

  // So is an upstream rule name a manifest at the head gives: the reason quotes it.
  it("a rule name with a control character selects as any other, and is printed escaped", () => {
    const planned = planFor({ "packs/python/conformance.json": manifest([...PACK_RULES, { id: "ruff-f841", tool: "ruff", rule: "F841\n::error title=x::y" }], [CORPUS]) });
    assert.deepEqual(names(planned), [RUFF]);
    assert.deepEqual(planned.lines, ["python/ruff: the ruff rules packs/python/conformance.json and candidates/conformance.json name changed (added `F841\\u000a:\\u003aerror title=x:\\u003ay`)"]);
    assert.match(commentBody(planned), /^- `python\/ruff`: .*\(added `F841\\u000a:\\u003aerror title=x:\\u003ay`\)$/mu);
  });

  // The runner takes `::` at a line's start for a workflow command, and the legacy `##[` anywhere in it
  // (`##[set-output name=legs;]x` mid-line sets a step output): neither survives, wherever the pull request puts it.
  it.skipIf(process.platform === "win32")("a path with `##[` or `::` anywhere selects as any other, and is printed with neither", () => {
    const planned = planFor({
      "packs/python/upstream/ruff/fixtures/##[set-output name=legs;]x::warning::y.py": "x\n",
      "packs/##[set-output name=legs;]x/upstream/::warning::y/gates.yaml": "adapter: x\n",
      "conformance/recorders/port-readiness/runtimes/::warning::x/##[set-output name=legs;]y": "z\n",
    });
    assert.deepEqual(names(planned), [RUFF]);
    assert.deepEqual(planned.lines, ["python/ruff: `packs/python/upstream/ruff/fixtures/#\\u0023[set-output name=legs;]x:\\u003awarning:\\u003ay.py` changed"]);
    assert.deepEqual(planned.warnings.map((warning) => warning.split(/ changed:| is not a tool-pack/u)[0]), [
      "`packs/#\\u0023[set-output name=legs;]x/upstream/:\\u003awarning:\\u003ay/gates.yaml`",
      "`conformance/recorders/port-readiness/runtimes/:\\u003awarning:\\u003ax/#\\u0023[set-output name=legs;]y`",
    ]);
    for (const text of [...planned.lines, ...planned.warnings, commentBody(planned)]) assert.doesNotMatch(text, /::|##\[/u);
  });

  it("a rule name with `##[` or `::` at its start, in it or after a newline selects as any other, and is printed with neither", () => {
    const rules = (tool: string, names: string[]) => names.map((rule, index) => ({ id: `${tool}-x${index}`, tool, rule }));
    const planned = planFor({
      "packs/python/conformance.json": manifest([...PACK_RULES, ...rules("bandit", ["##[set-output name=legs;]x", "::warning::x", "B1 ##[set-output name=legs;]x", "B2 ::warning::x"])], [CORPUS]),
      "packs/python/candidates/conformance.json": manifest([...CANDIDATE_RULES, ...rules("ruff", ["\n::warning::x", "\r\n##[set-output name=legs;]x"])]),
    });
    assert.deepEqual(names(planned), [RUFF, BANDIT]);
    assert.deepEqual(planned.lines.map((line) => /\(added (.*)\)$/u.exec(line)?.[1]), [
      "`\\u000a:\\u003awarning:\\u003ax`, `\\u000d\\u000a#\\u0023[set-output name=legs;]x`",
      "`#\\u0023[set-output name=legs;]x`, `:\\u003awarning:\\u003ax`, `B1 #\\u0023[set-output name=legs;]x`, `B2 :\\u003awarning:\\u003ax`",
    ]);
    for (const text of [...planned.lines, commentBody(planned)]) assert.doesNotMatch(text, /::|##\[/u);
    // A manifest that does not parse at the head is quoted in the warning: so is what it holds.
    for (const text of ["##[set-output name=legs;]x ::warning::y\n", "::warning::x ##[set-output name=legs;]y\n"]) {
      const unparsed = planFor({ "packs/python/candidates/conformance.json": text });
      assert.equal(unparsed.warnings.length, 1);
      assert.match(unparsed.warnings[0]!, /#\\u0023\[|:\\u003a/u);
      for (const shown of [...unparsed.warnings, commentBody(unparsed)]) assert.doesNotMatch(shown, /::|##\[/u);
    }
  });

  // The comment goes on the pull request and to the job's summary as Markdown: what the pull request chose is a code
  // span there, so it cannot mention anyone, reference an issue, link, embed an image or open an HTML comment. A path
  // cannot hold `//`, so its links have one slash, which a browser reads as two.
  it.skipIf(process.platform === "win32")("a path or rule name with a mention, a reference, a link, an image or `<!--` is in the comment only as a code span", () => {
    const path = "packs/python/upstream/ruff/fixtures/@someone #12 [x](https:/evil.example) ![i](https:/evil.example/i.png) <!--.py";
    const rule = "@someone #12 [x](https://evil.example) ![i](https://evil.example/i.png) <!--";
    const planned = planFor({ [path]: "x\n", "packs/python/conformance.json": manifest([...PACK_RULES, { id: "bandit-x", tool: "bandit", rule }], [CORPUS]) });
    assert.deepEqual(names(planned), [RUFF, BANDIT]);
    const comment = commentBody(planned);
    const lines = comment.split("\n");
    assert.ok(lines.includes(`- \`python/ruff\`: \`${path}\` changed`), comment);
    assert.ok(lines.includes(`- \`python/bandit\`: the bandit rules packs/python/conformance.json and candidates/conformance.json name changed (added \`${rule}\`)`), comment);
    const { spans, outside } = codeSpans(comment);
    assert.ok(spans.includes(path) && spans.includes(rule), spans.join("\n"));
    for (const token of ["@someone", "#12", "](", "![", "<!--", "evil.example"]) assert.ok(!outside.includes(token), `${token} outside a code span:\n${outside}`);
  });

  // A backtick in what the pull request chose must not close its span early, nor one at either end join the fence.
  it("a path or rule name with backticks is fenced longer than any run of them, and padded where one meets the fence", () => {
    const runtime = "conformance/recorders/port-readiness/runtimes/x``y`";
    const planned = planFor({
      "packs/python/conformance.json": manifest([...PACK_RULES, ...["a`b``c", "`x", " y "].map((rule, index) => ({ id: `bandit-x${index}`, tool: "bandit", rule }))], [CORPUS]),
      [runtime]: "z\n",
    });
    assert.deepEqual(names(planned), [BANDIT]);
    assert.deepEqual(planned.lines, ["python/bandit: the bandit rules packs/python/conformance.json and candidates/conformance.json name changed (added `  y  `, `` `x ``, ```a`b``c```)"]);
    assert.deepEqual(planned.warnings.map((warning) => warning.split(" changed:")[0]), ["``` conformance/recorders/port-readiness/runtimes/x``y` ```"]);
    // Read back as CommonMark reads the comment, each span is the text the pull request chose, whole.
    const { spans } = codeSpans(commentBody(planned));
    for (const text of [" y ", "`x", "a`b``c", runtime]) assert.ok(spans.includes(text), `${text} is no span of the comment:\n${spans.join("\n")}`);
  });

  // git ls-tree prints `<mode> <type> <object>\t<path>`, and the path may hold a tab of its own: the recorder reads
  // every licenses/*/UPSTREAM.md, whatever its directory is named.
  it.skipIf(process.platform === "win32")("a `- Not copied:` line in an UPSTREAM.md whose path holds a tab selects every tool-pack of the pack", () => {
    const planned = planFor({ "packs/python/licenses/ru\tff/UPSTREAM.md": "# Ruff\n\n- Not copied: `crates/x` — GPL-3.0-only\n" });
    assert.deepEqual(names(planned), [RUFF, BANDIT]);
    assert.match(planned.lines[0]!, /^python\/ruff: the "Not copied" lines of packs\/python\/licenses\/\*\/UPSTREAM\.md changed$/u);
  });

  it("the regex is the recorder's, and reads an UPSTREAM.md as the recorder does", () => {
    const literal = /^const NOT_COPIED = (\/.*\/[a-z]*);$/mu.exec(readFileSync(join(ROOT, "conformance", "recorders", "port-readiness", "record.ts"), "utf8"))?.[1];
    assert.equal(literal, String(NOT_COPIED));
    const pack = tempRoot();
    write(pack, "packs/demo/licenses/demo/UPSTREAM.md", `${UPSTREAM_MD}- Not copied: \`b/\` and more\n`);
    assert.deepEqual(notCopiedOf(readFileSync(join(pack, "packs/demo/licenses/demo/UPSTREAM.md"), "utf8")), notCopiedPrefixes(pack, "demo"));
  });
});

describe("the tool-packs a dispatch records", () => {
  it("keeps the dispatch's pack and tool filter, every tool-pack when both are empty", () => {
    assert.deepEqual(names(dispatchPlan(TOOL_PACKS, "python", "")), [RUFF, BANDIT]);
    assert.deepEqual(names(dispatchPlan(TOOL_PACKS, "", "eslint")), [ESLINT]);
    assert.deepEqual(names(dispatchPlan(TOOL_PACKS, "", "")), [RUFF, BANDIT, ESLINT]);
    assert.deepEqual(dispatchPlan(TOOL_PACKS, "rust", "").legs, []);
  });

  it("reads TOOL_PACKS as the workflow writes it, and refuses anything else", () => {
    assert.deepEqual(parseToolPacks('[\n  {"pack": "python", "tool": "ruff"}\n]\n'), [{ pack: "python", tool: "ruff" }]);
    assert.throws(() => parseToolPacks("[{\"pack\": \"python\"}]"), /TOOL_PACKS/u);
    assert.throws(() => parseToolPacks("{"), /TOOL_PACKS/u);
  });
});

describe("the command", () => {
  function run(...args: string[]): { status: number | null; stdout: string; output: string; comment: string } {
    const directory = tempRoot();
    const output = join(directory, "github-output");
    const comment = join(directory, "comment.md");
    writeFileSync(output, "");
    const result = spawnSync(process.execPath, [join(root, "conformance", "port-readiness", "recording", "record-plan.mjs"), "--tool-packs", JSON.stringify(TOOL_PACKS), "--comment", comment, ...args], {
      cwd: root,
      encoding: "utf8",
      env: { ...process.env, GITHUB_OUTPUT: output },
    });
    const read = (path: string) => { try { return readFileSync(path, "utf8"); } catch { return ""; } };
    return { status: result.status, stdout: `${result.stdout}${result.stderr}`, output: read(output), comment: read(comment) };
  }

  it("writes the selected legs in TOOL_PACKS order, one line per tool-pack, and the comment", () => {
    const head = pullRequest({ "packs/python/upstream/ruff/goldens.tsv": "task\tpath\nx\ty.py\n", "packs/python/licenses/ruff/UPSTREAM.md": `${UPSTREAM_MD}- Not copied: \`x/\`\n` });
    const { status, stdout, output, comment } = run("--event", "pull_request_target", "--base", MAIN, "--head", head);
    assert.equal(status, 0, stdout);
    assert.equal(output, 'legs=[{"pack":"python","tool":"ruff"},{"pack":"python","tool":"bandit"}]\n');
    assert.match(stdout, /^python\/ruff: `packs\/python\/upstream\/ruff\/goldens\.tsv` changed \(and 1 more\)$/mu);
    assert.match(stdout, /^python\/bandit: the "Not copied" lines/mu);
    assert.match(comment, /records the 2 tool-packs whose recorder inputs this pull request changes/u);
    assert.match(comment, /^- `python\/ruff`: `packs\/python\/upstream\/ruff\/goldens\.tsv` changed \(and 1 more\)$/mu);
  });

  it("writes `legs=[]` and a comment saying why nothing is recorded, with the warnings", () => {
    const head = pullRequest({ "conformance/recorders/port-readiness/runtimes/ruff-0.16.9/pyproject.toml": "[project]\nname = \"x\"\n" });
    const { status, stdout, output, comment } = run("--event", "pull_request_target", "--base", MAIN, "--head", head);
    assert.equal(status, 0, stdout);
    assert.equal(output, "legs=[]\n");
    assert.match(stdout, /^No tool-pack's recorder inputs changed: nothing to record\.$/mu);
    assert.match(stdout, /^::warning title=Port readiness recording::`conformance\/recorders\/port-readiness\/runtimes\/ruff-0\.16\.9\/pyproject\.toml` changed: /mu);
    assert.match(comment, /nothing to record at `[0-9a-f]{10}`/u);
    assert.match(comment, /`gates\.yaml`, a file under its `fixtures\/`, or its `goldens\.tsv`/u);
    assert.match(comment, /^- `conformance\/recorders\/port-readiness\/runtimes\/ruff-0\.16\.9\/pyproject\.toml` changed: a label records with main's runtimes/mu);
    assert.match(comment, /To record a tool-pack anyway, run `port-readiness-record\.yml` from `main` by hand/u);
  });

  it("starts no line of its stdout or its comment with what the pull request wrote", () => {
    const head = pullRequest({ "packs/python/conformance.json": manifest([...PACK_RULES, { id: "ruff-f841", tool: "ruff", rule: "F841\r\n::error title=x::y" }], [CORPUS]) });
    const { status, stdout, output, comment } = run("--event", "pull_request_target", "--base", MAIN, "--head", head);
    assert.equal(status, 0, stdout);
    assert.equal(output, 'legs=[{"pack":"python","tool":"ruff"}]\n');
    assert.doesNotMatch(stdout, /^::error/mu);
    assert.doesNotMatch(comment, /^::error/mu);
    assert.match(stdout, /^python\/ruff: .*\(added `F841\\u000d\\u000a:\\u003aerror title=x:\\u003ay`\)$/mu);
  });

  // As authorize runs it: its own workflow commands go to --annotations, for the step to print once commands resume.
  it("prints nothing the pull request wrote as a workflow command, anywhere in a line, and writes its own to --annotations", () => {
    const rules = ["##[set-output name=legs;]x", "::warning::x", "B1 ##[set-output name=legs;]x", "B2\n::warning::x"].map((rule, index) => ({ id: `bandit-x${index}`, tool: "bandit", rule }));
    const head = pullRequest({
      "packs/python/conformance.json": manifest([...PACK_RULES, ...rules], [CORPUS]),
      "conformance/recorders/port-readiness/runtimes/##[set-output name=legs;]x": "z\n",
    });
    const annotations = join(tempRoot(), "annotations");
    const { status, stdout, output, comment } = run("--event", "pull_request_target", "--base", MAIN, "--head", head, "--annotations", annotations);
    assert.equal(status, 0, stdout);
    assert.equal(output, 'legs=[{"pack":"python","tool":"bandit"}]\n');
    assert.match(stdout, /^python\/bandit: .*\(added `#\\u0023\[set-output name=legs;\]x`, `:\\u003awarning:\\u003ax`, /mu);
    assert.doesNotMatch(stdout, /::|##\[/u);
    assert.doesNotMatch(comment, /::|##\[/u);
    // The comment, which goes to the job's summary, has the warnings when a tool-pack is selected too.
    assert.match(comment, /^Warnings:\n\n- `conformance\/recorders\/port-readiness\/runtimes\/#\\u0023\[set-output name=legs;\]x` changed: /mu);
    const commands = readFileSync(annotations, "utf8").split("\n").filter((line) => line !== "");
    assert.equal(commands.length, 1);
    const prefix = "::warning title=Port readiness recording::";
    assert.ok(commands[0]!.startsWith(`${prefix}\`conformance/recorders/port-readiness/runtimes/#\\u0023[set-output name=legs;]x\` changed: `), commands[0]!);
    assert.doesNotMatch(commands[0]!.slice(prefix.length), /::|##\[/u);
  });

  it("records every tool-pack when git cannot compare, with a warning", () => {
    const { status, stdout, output } = run("--event", "pull_request_target", "--base", "no-such-commit", "--head", MAIN);
    assert.equal(status, 0, stdout);
    assert.match(stdout, /^::warning title=Port readiness recording::git could not compare `no-such-commit` with `[0-9a-f]{40}`, so every tool-pack is recorded: `[^`]+`$/mu);
    assert.equal(output, `legs=${JSON.stringify(TOOL_PACKS)}\n`);
  });

  it("a dispatch keeps its filter, and fails when it names no tool-pack", () => {
    assert.equal(run("--event", "workflow_dispatch", "--pack", "python", "--tool", "bandit").output, 'legs=[{"pack":"python","tool":"bandit"}]\n');
    assert.equal(run("--event", "workflow_dispatch", "--pack", "", "--tool", "").output, `legs=${JSON.stringify(TOOL_PACKS)}\n`);
    const none = run("--event", "workflow_dispatch", "--pack", "rust", "--tool", "");
    assert.equal(none.status, 1);
    assert.match(none.stdout, /^::error::no gated tool-pack is pack 'rust', tool ''$/mu);
    assert.equal(none.output, "");
    const annotations = join(tempRoot(), "annotations");
    const annotated = run("--event", "workflow_dispatch", "--pack", "rust", "--tool", "", "--annotations", annotations);
    assert.equal(annotated.status, 1);
    assert.equal(readFileSync(annotations, "utf8"), "::error::no gated tool-pack is pack 'rust', tool ''\n");
    assert.doesNotMatch(annotated.stdout, /::/u);
  });

  it("writes exactly a `legs=` line", () => {
    const out = join(tempRoot(), "github-output");
    writeOutputs(out, { legs: [] });
    writeOutputs(out, { legs: [{ pack: "python", tool: "ruff" }] });
    assert.equal(readFileSync(out, "utf8"), 'legs=[]\nlegs=[{"pack":"python","tool":"ruff"}]\n');
    assert.match(commentBody(dispatchPlan(TOOL_PACKS, "python", "ruff")), /records the 1 tool-pack the dispatch names/u);
  });
});

// The authorize job runs main's copy of record-plan.mjs, on the pull request's head as git objects alone.
describe("port-readiness-record.yml", () => {
  type Step = { id?: string; name?: string; if?: string; uses?: string; run?: string; with?: Record<string, unknown>; env?: Record<string, string> };
  const jobs = parse(readFileSync(join(ROOT, ".github", "workflows", "port-readiness-record.yml"), "utf8")).jobs as Record<string, { if?: string; steps: Step[] }>;
  const steps = jobs.authorize!.steps;
  const at = (predicate: (step: Step) => boolean): number => steps.findIndex(predicate);
  const legs = steps[at((step) => step.id === "legs")]!;

  it("chooses the tool-packs with record-plan.mjs on every event, the label's included", () => {
    assert.equal(legs.if, undefined);
    const command = legs.run!.replace(/\\\n\s*/gu, "");
    assert.match(command, /^\s*node conformance\/port-readiness\/recording\/record-plan\.mjs --event "\$EVENT" --base "\$BASE_SHA" --head "\$HEAD_SHA" --pack "\$WANT_PACK" --tool "\$WANT_TOOL" --tool-packs "\$TOOL_PACKS" --comment "[^"]+" --annotations "[^"]+" \|\| status=\$\?$/mu);
    assert.equal(legs.env!.EVENT, "${{ github.event_name }}");
    assert.equal(legs.env!.BASE_SHA, "${{ steps.target.outputs.base-sha }}");
    assert.equal(legs.env!.HEAD_SHA, "${{ steps.target.outputs.head }}");
    assert.doesNotMatch(legs.run!, /\bjq\b/u);
  });

  it("keeps TOOL_PACKS in the legs step: main's list is what a label can record", () => {
    assert.deepEqual(parseToolPacks(legs.env!.TOOL_PACKS!).map(({ pack, tool }) => `${pack}/${tool}`).sort(), gatedToolPacks().map(({ pack, tool }) => `${pack}/${tool}`).sort());
  });

  it("checks out the base commit, and reads the pull request's head as git objects, never checked out", () => {
    const target = at((step) => step.id === "target");
    const checkout = at((step) => step.uses?.startsWith("actions/checkout@") === true);
    const fetch = at((step) => /refs\/pull\/\$PR\/head/u.test(step.run ?? ""));
    assert.ok(target < checkout && checkout < fetch && fetch < at((step) => step.id === "legs"), "target, checkout, fetch, legs, in that order");
    assert.deepEqual(steps[checkout]!.with, { ref: "${{ steps.target.outputs.base-sha }}", "fetch-depth": 0, "persist-credentials": false });
    assert.equal(steps[fetch]!.if, "github.event_name == 'pull_request_target'");
    assert.equal(steps.filter((step) => step.uses?.startsWith("actions/checkout@") === true).length, 1);
    for (const step of steps) assert.doesNotMatch(step.run ?? "", /\bgit (checkout|switch|worktree|reset)\b/u, step.name ?? step.id ?? "a step");
  });

  it("says on the pull request why a label recorded nothing, and runs no record leg", () => {
    const comment = /--comment "([^"]+)"/u.exec(legs.run!)?.[1];
    assert.ok(comment !== undefined, "the legs step writes no comment");
    const post = steps.find((step) => step.if === "github.event_name == 'pull_request_target' && steps.legs.outputs.legs == '[]'");
    assert.ok(post?.run !== undefined, "no step posts the comment when a label selects nothing");
    assert.ok(post.run.includes(`gh api "repos/$REPO/issues/$PR/comments" -F body=@"${comment}"`), post.run);
    assert.equal(jobs.record!.if, "needs.authorize.outputs.legs != '[]'");
  });

  // record-plan.mjs prints paths and rule names the pull request chose, in a job holding a write token whose `legs`
  // output is the record job's matrix: the runner takes nothing it prints for a workflow command.
  it("runs record-plan.mjs with workflow commands stopped, under a token the step makes, and resumes them after", () => {
    const script = legs.run!.replace(/\\\n\s*/gu, "").split("\n").map((line) => line.trim());
    const line = (pattern: RegExp): number => script.findIndex((text) => pattern.test(text));
    const token = line(/^token="\$\((?:uuidgen|.*\/dev\/urandom.*)\)"$/u);
    const stop = line(/^echo "::stop-commands::\$token"$/u);
    const node = line(/^node conformance\/port-readiness\/recording\/record-plan\.mjs /u);
    const resume = line(/^echo "::\$token::"$/u);
    const annotations = /--annotations "([^"]+)"/u.exec(script[node] ?? "")?.[1];
    assert.ok(annotations !== undefined, "record-plan.mjs writes its workflow commands to no file");
    const printed = script.findIndex((text) => text.endsWith(`cat "${annotations}"`));
    assert.ok(token >= 0 && token < stop && stop < node && node < resume && resume < printed, script.join("\n"));
    // A plan that fails resumes commands too, and the step then fails with it.
    assert.match(script[node]!, / \|\| status=\$\?$/u);
    assert.ok(script.slice(resume + 1).some((text) => /(?:^|\|\| )exit "\$status"$/u.test(text)), script.join("\n"));
    // Nothing else runs while commands are stopped, and no expression puts text in the script.
    assert.deepEqual(script.slice(stop + 1, resume), [script[node]]);
    assert.doesNotMatch(legs.run!, /\$\{\{/u);
    // The comment, warnings and all, goes to the job's summary whatever is selected.
    const comment = /--comment "([^"]+)"/u.exec(script[node]!)?.[1];
    assert.ok(script.slice(resume + 1).includes(`cat "${comment}" >> "$GITHUB_STEP_SUMMARY"`), script.join("\n"));
  });
});
