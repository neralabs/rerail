import { describe, expect, it } from "vitest";
import {
  artefactUnits,
  branchName,
  closedHandoffKeys,
  codeFingerprints,
  type CommitChange,
  conceptIds,
  consumerHits,
  contractUnits,
  declarations,
  extractUnits,
  type FileChange,
  folderIndex,
  handoffUnits,
  movedCodeUnits,
  namedAnchors,
  parseShow,
  positionPassages,
  type RefHistory,
  retirementUnits,
  sqlTables,
  subsystemFolders,
  touchedAnchors,
  visibleUnits,
} from "../../../src/engine/index.js";

function commit(files: FileChange[], message: { subject?: string; body?: string } = {}): CommitChange {
  return {
    sha: "a".repeat(40),
    parent: "b".repeat(40),
    merge: false,
    author: "Ada",
    authoredAt: "2026-11-01T10:00:00Z",
    committedAt: "2026-11-01T10:00:00Z",
    subject: message.subject ?? "change",
    body: message.body ?? "",
    files,
  };
}

function file(path: string, added: string[], removed: string[] = [], status: FileChange["status"] = "modified", oldPath = path): FileChange {
  return { path, oldPath, status, added: added.length > 0 ? [{ start: 1, lines: added }] : [], removed: removed.length > 0 ? [{ start: 1, lines: removed }] : [] };
}

const known = new Set(["native/src/content_store/schema.rs", "src/engine/store.ts", "docs/open-items.md", "src/api/routes.ts"]);
const resolve = (token: string): string | undefined => {
  if (known.has(token)) return token;
  const matches = [...known].filter(path => path.endsWith(`/${token}`));
  return matches.length === 1 ? matches[0] : undefined;
};
const noTexts = { before: new Map<string, string>(), after: new Map<string, string>() };

describe("declarations", () => {
  it("reads TypeScript exports, export lists and non-exported top-level functions", () => {
    const text = [
      "export async function load(id: string,",
      "  options: Options): Promise<User> {",
      "}",
      "function helper() {}",
      "export interface Options { cache: boolean }",
      "export const LIMIT = 10;",
      "class Store {}",
      "export { Store };",
    ].join("\n");
    expect(declarations("typescript", text).map(entry => [entry.name, entry.kind, entry.exported, entry.signature])).toEqual([
      ["load", "function", true, "export async function load(id: string, options: Options): Promise<User>"],
      ["helper", "function", false, "function helper()"],
      ["Options", "type", true, "export interface Options { cache: boolean }"],
      ["LIMIT", "constant", true, "export const LIMIT = 10;"],
      ["Store", "class", true, "class Store"],
    ]);
  });

  it("qualifies Python methods by class and treats a leading underscore as private", () => {
    const text = ["class Ledger:", "    def add(self, unit):", "        pass", "    def _drop(self):", "        pass", "def build(path):", "    pass", "MAX_UNITS = 3"].join("\n");
    expect(declarations("python", text).map(entry => [entry.name, entry.kind, entry.exported])).toEqual([
      ["Ledger", "class", true],
      ["Ledger.add", "method", true],
      ["Ledger._drop", "method", false],
      ["build", "function", true],
      ["MAX_UNITS", "constant", true],
    ]);
  });

  it("reads Go receivers and capitalised exports", () => {
    const text = ["func (s *Store) Put(key string) error {", "}", "func helper() {}", "type Tape struct {", "}"].join("\n");
    expect(declarations("go", text).map(entry => [entry.name, entry.kind, entry.exported])).toEqual([
      ["Store.Put", "method", true],
      ["helper", "function", false],
      ["Tape", "type", true],
    ]);
  });

  it("reads Rust items with visibility and qualifies impl functions", () => {
    const text = ["pub struct Tape;", "impl Tape {", "    pub fn put_tape(&self, bytes: &[u8]) -> Result<()> {", "    }", "}", "pub(crate) const SCHEMA: &str = \"x\";", "fn private() {}"].join("\n");
    expect(declarations("rust", text).map(entry => [entry.name, entry.exported, entry.signature])).toEqual([
      ["Tape", true, "pub struct Tape"],
      ["Tape.put_tape", true, "pub fn put_tape(&self, bytes: &[u8]) -> Result<()>"],
      ["SCHEMA", true, "pub(crate) const SCHEMA: &str = \"x\";"],
      ["private", false, "fn private()"],
    ]);
  });
});

describe("sqlTables", () => {
  it("names the tables DDL creates, alters and indexes, also inside string literals", () => {
    expect(sqlTables("\"CREATE TABLE IF NOT EXISTS tapes (id)\"\nALTER TABLE memo ADD COLUMN v\nCREATE INDEX idx ON tapes (id)")).toEqual(["tapes", "memo"]);
  });
});

describe("parseShow", () => {
  it("reads the header, renames and numbered added and removed runs", () => {
    const output = [
      `${"c".repeat(40)}\u001f${"d".repeat(40)}\u001fAda\u001f2026-11-01T10:00:00Z\u001f2026-11-01T11:00:00Z\u001fMove store\u001fBody line\u001e`,
      "diff --git a/src/old.ts b/src/new.ts",
      "similarity index 90%",
      "rename from src/old.ts",
      "rename to src/new.ts",
      "--- a/src/old.ts",
      "+++ b/src/new.ts",
      "@@ -3,2 +3 @@ context",
      "-const a = 1;",
      "--- not a header",
      "+const a = 2;",
      "diff --git a/package-lock.json b/package-lock.json",
      "@@ -1 +1 @@",
      "-x",
      "+y",
    ].join("\n");
    const change = parseShow("c".repeat(40), output);
    expect(change).toMatchObject({ parent: "d".repeat(40), merge: false, author: "Ada", subject: "Move store", body: "Body line" });
    expect(change.files).toEqual([{ path: "src/new.ts", oldPath: "src/old.ts", status: "renamed", added: [{ start: 3, lines: ["const a = 2;"] }], removed: [{ start: 3, lines: ["const a = 1;", "-- not a header"] }] }]);
  });
});

describe("namedAnchors", () => {
  it("resolves paths and keeps code-shaped names, not prose, hashes or timestamps", () => {
    const anchors = namedAnchors("See `schema.rs:12` and src/engine/store.ts; put_tape vs `Tape::open` and rankPrompt, not README or bb5073472 or T092043", resolve);
    expect(anchors.files).toEqual(["native/src/content_store/schema.rs", "src/engine/store.ts"]);
    expect(anchors.symbols).toEqual(["Tape.open", "put_tape", "rankPrompt"]);
  });
});

describe("handoffUnits", () => {
  it("keeps open items that name a file or symbol, and code TODOs anchored to their file", () => {
    const change = commit([
      file("docs/open-items.md", ["- O70: who owns `put_tape` busy retries", "- O71: nothing named here", "Plain text"]),
      file("src/engine/store.ts", ["  // TODO: decide the retry budget"]),
    ]);
    const texts = { before: new Map<string, string>(), after: new Map([["docs/open-items.md", "# Open items\n- O70: who owns `put_tape` busy retries\n- O71: nothing named here\nPlain text"]]) };
    const units = handoffUnits(change, texts, resolve);
    expect(units.map(unit => [unit.anchors.files, unit.anchors.symbols])).toEqual([
      [["docs/open-items.md"], ["put_tape"]],
      [["docs/open-items.md"], []],
      [["src/engine/store.ts"], []],
    ]);
    expect(units[0]!.key).toBe("docs/open-items.md\u0000- o70: who owns `put_tape` busy retries");
  });

  it("closes a hand-off when a later commit removes its line", () => {
    const keys = closedHandoffKeys(commit([file("docs/open-items.md", [], ["- O70: who owns `put_tape` busy retries", "prose that was never an item"])]));
    expect(keys).toEqual(["docs/open-items.md\u0000- o70: who owns `put_tape` busy retries"]);
  });
});

describe("retirementUnits", () => {
  it("groups deletions by folder, records moves and removed exports, and ignores a declaration that moved", () => {
    const change = commit([
      file("src/old/a.ts", [], ["export function alpha() {}"], "deleted"),
      file("src/old/b.ts", [], ["x"], "deleted"),
      file("src/api/routes.ts", ["export function beta() {}"], [], "renamed", "src/api/route.ts"),
      file("src/engine/store.ts", [], ["export function removedHelper() {}"]),
    ], { subject: "Retire the old store; replace `openStore` with openLedger" });
    const texts = {
      before: new Map([["src/old/a.ts", "export function alphaLongName() {}"], ["src/engine/store.ts", "export function removedHelper() {}\nexport function kept() {}"], ["src/api/route.ts", "export function beta() {}"]]),
      after: new Map([["src/engine/store.ts", "export function kept() {}"], ["src/api/routes.ts", "export function beta() {}\nexport function alphaLongName() {}"]]),
    };
    const units = retirementUnits(change, texts, resolve);
    expect(units.map(unit => [unit.text.split(":")[0], unit.anchors])).toEqual([
      ["deletes 2 files under src/old/ (a.ts, b.ts)", { files: ["src/old/a.ts", "src/old/b.ts"], symbols: [] }],
      ["moves src/api/route.ts to src/api/routes.ts", { files: ["src/api/route.ts", "src/api/routes.ts"], symbols: [] }],
      ["removes removedHelper from src/engine/store.ts", { files: ["src/engine/store.ts"], symbols: ["removedHelper"] }],
      ["Retire the old store; replace `openStore` with openLedger", { files: [], symbols: ["openStore", "openLedger"] }],
    ]);
  });
});

describe("contractUnits", () => {
  it("reports an exported signature change but not a visibility-only one, and schema tables", () => {
    const change = commit([
      file("src/engine/store.ts", ["export function open(path: string, mode: Mode) {}"], ["export function open(path: string) {}"]),
      file("native/src/content_store/schema.rs", ["    \"CREATE TABLE tapes (id INTEGER)\","]),
    ]);
    const texts = {
      before: new Map([["src/engine/store.ts", "export function open(path: string) {}\nfunction same() {}"], ["native/src/content_store/schema.rs", "pub(super) fn migrate() {}"]]),
      after: new Map([["src/engine/store.ts", "export function open(path: string, mode: Mode) {}\nfunction same() {}"], ["native/src/content_store/schema.rs", "pub(crate) fn migrate() {}"]]),
    };
    const units = contractUnits(change, texts);
    expect(units.map(unit => [unit.anchors.files[0], unit.anchors.symbols])).toEqual([
      ["src/engine/store.ts", ["open"]],
      ["native/src/content_store/schema.rs", ["tapes"]],
    ]);
    expect(units[0]!.text).toContain("`export function open(path: string)` → `export function open(path: string, mode: Mode)`");
  });
});

describe("positionPassages", () => {
  it("offers design-doc runs that state a choice and a substantive message, not changelogs", () => {
    const decision = "We decided that SQLite is the system of record for facts, rather than a columnar engine in RAM.";
    const change = commit([
      file("docs/final-shape/decisions/0030.md", [decision]),
      file("CHANGELOG.md", [decision]),
      file("docs/notes.md", ["Ran the benchmark again; numbers are in the table below this line of text."]),
    ], { subject: "facts: SQLite stays", body: "The facts engine must keep SQLite as its store; the RAM engine is retired instead of ported." });
    const texts = { before: new Map<string, string>(), after: new Map([["docs/final-shape/decisions/0030.md", `# 0030: Facts store\n${decision}`]]) };
    expect(positionPassages(change, texts).map(passage => [passage.id.split(":")[1], passage.path, passage.heading])).toEqual([
      ["0", null, null],
      ["1", "docs/final-shape/decisions/0030.md", "0030: Facts store"],
    ]);
  });
});

describe("codeFingerprints", () => {
  it("hashes windows of normalised added code lines, skipping comments and braces", () => {
    const lines = ["fn copy_one(a: u32) -> u32 {", "    let first = a + 1;", "    // comment", "    let second = first * 2;", "    let third = second - 3;", "    let fourth = third / 4;", "}", "    let fifth = fourth % 5;", "    return fifth + first;"];
    const one = codeFingerprints(commit([file("native/src/a.rs", lines)]));
    const reindented = codeFingerprints(commit([file("native/src/b.rs", lines.map(line => `  ${line}`))]));
    expect(one).toHaveLength(2);
    expect(reindented.map(entry => entry.hash)).toEqual(one.map(entry => entry.hash));
    expect(codeFingerprints(commit([file("docs/a.md", lines)]))).toEqual([]);
  });
});

describe("visibleUnits", () => {
  const unit = { kind: "handoff" as const, text: "O70 open", anchors: { files: ["docs/open-items.md"], symbols: ["put_tape"] }, key: "docs/open-items.md\u0000o70" };
  const histories: RefHistory[] = [
    {
      ref: "refs/remotes/origin/jeremy/final-shape",
      branch: "jeremy/final-shape",
      pr: null,
      commits: [
        { sha: "1".repeat(40), author: "Jeremy", visibleAt: "2026-11-01T00:00:00Z", units: [unit], closes: [] },
        { sha: "2".repeat(40), author: "Jeremy", visibleAt: "2026-11-03T00:00:00Z", units: [], closes: ["docs/open-items.md\u0000o70"] },
      ],
    },
    {
      ref: "refs/pull/7/head",
      branch: branchName("refs/pull/7/head"),
      pr: 7,
      commits: [{ sha: "1".repeat(40), author: "Jeremy", visibleAt: "2026-10-30T00:00:00Z", units: [unit], closes: [] }],
    },
  ];

  it("counts a commit from when it became public, once, under the ref that showed it first", () => {
    expect(visibleUnits(histories, "2026-10-29T00:00:00Z")).toEqual([]);
    const units = visibleUnits(histories, "2026-11-02T00:00:00Z");
    expect(units.map(entry => [entry.id, entry.branch, entry.pr])).toEqual([[`pull/7@${"1".repeat(10)}#handoff-0`, "pull/7", 7]]);
  });

  it("drops a hand-off a later visible commit on the same ref closes", () => {
    const closedOnOwnRef = visibleUnits([histories[0]!], "2026-11-04T00:00:00Z");
    expect(closedOnOwnRef).toEqual([]);
  });

  it("matches a touch by file or by a symbol the changed lines use", () => {
    expect(touchedAnchors(unit.anchors, { files: ["src/x.rs"], symbols: new Set(["put_tape"]) })).toEqual({ files: [], symbols: ["put_tape"] });
    expect(touchedAnchors({ files: ["native/src/"], symbols: [] }, { files: ["native/src/a.rs"], symbols: new Set() })).toEqual({ files: ["native/src/a.rs"], symbols: [] });
  });
});

describe("extractUnits", () => {
  it("yields nothing for a merge commit", () => {
    expect(extractUnits({ ...commit([file("src/engine/store.ts", ["// TODO: x"])]), merge: true }, noTexts, resolve)).toEqual([]);
  });
});

describe("consumerHits", () => {
  it("fires a contract change where a touched file uses its symbol", () => {
    const base = { text: "changes open", key: null, ref: "refs/pull/1/head", branch: "pull/1", pr: 1, author: "Ada", sha: "1".repeat(40), visibleAt: "2026-11-01T00:00:00Z" };
    const units = [
      { ...base, id: "a", kind: "contract" as const, anchors: { files: ["src/engine/store.ts"], symbols: ["Store.openLedger"] } },
      { ...base, id: "b", kind: "contract" as const, anchors: { files: ["src/engine/store.ts"], symbols: ["closeLedger"] } },
    ];
    const texts = new Map([["src/api/routes.ts", "const ledger = store.openLedger(path);"], ["src/api/other.ts", "openLedgerish()"]]);
    expect([...consumerHits(units, texts)]).toEqual([["a", { files: ["src/api/routes.ts"], symbols: ["Store.openLedger"] }]]);
  });
});

describe("link-1 regressions", () => {
  it("needs a hand-off marker where a comment or item starts, or an open-item reference", () => {
    const change = commit([file("src/engine/store.ts", [
      "export function handoffSettings(page: Page) {}",
      "  // the follow-up reads this later",
      "  // FIXME: the retry budget",
      "  // P's relational inputs (test-only, open item O18)",
    ])]);
    expect(handoffUnits(change, noTexts, resolve).map(unit => unit.text.split(" (")[0])).toEqual(["// FIXME: the retry budget", "// P's relational inputs"]);
  });

  it("makes an open-item section one hand-off keyed by its heading, closed when the heading goes", () => {
    const lines = ["## O61. The builtin catalogue", "The closure is a fixed catalogue of `rx_match` builtins."];
    const change = commit([file("docs/final-shape/open-items.md", lines)]);
    const units = handoffUnits(change, { before: new Map(), after: new Map([["docs/final-shape/open-items.md", lines.join("\n")]]) }, resolve);
    expect(units.map(unit => [unit.kind, unit.key, unit.anchors.symbols])).toEqual([["handoff", "docs/final-shape/open-items.md\u0000#o61. the builtin catalogue", ["rx_match"]]]);
    expect(closedHandoffKeys(commit([file("docs/final-shape/open-items.md", [], [lines[0]!])]))).toEqual(["docs/final-shape/open-items.md\u0000#o61. the builtin catalogue"]);
  });

  it("finds code moved between files without a rename", () => {
    const body = ["export function checkRule(rule: Rule): Result {", "  const compiled = compileRule(rule.source);", "  const cases = rule.cases.map(toCase);", "  const failures = cases.filter(item => !item.ok);", "  return { compiled, failures, passed: failures.length === 0 };", "  // done", "  const extra = summarise(failures, cases);", "  log(extra.message);", "}"];
    const change = commit([file("src/cli/rule-draft.ts", [], body), file("src/engine/code-rule/rule-check.ts", body, [], "added")]);
    const texts = { before: new Map([["src/cli/rule-draft.ts", body.join("\n")]]), after: new Map([["src/engine/code-rule/rule-check.ts", body.join("\n")]]) };
    expect(movedCodeUnits(change, texts).map(unit => [unit.anchors.files, unit.anchors.symbols])).toEqual([[["src/cli/rule-draft.ts", "src/engine/code-rule/rule-check.ts"], ["checkRule"]]]);
  });

  it("treats a migrations list and a type's fields as contracts", () => {
    const change = commit([
      file("src/engine/storage/migrations.ts", ["  { id: \"rule-approvals\", up },"]),
      file("native/src/logic/front.rs", ["    Ast { document: String, captures: Vec<String> },"], ["    Ast { document: String },"]),
    ]);
    const texts = {
      before: new Map([["native/src/logic/front.rs", "pub enum Literal {\n    Ast { document: String },\n}"]]),
      after: new Map([["native/src/logic/front.rs", "pub enum Literal {\n    Ast { document: String, captures: Vec<String> },\n}"]]),
    };
    expect(contractUnits(change, texts).map(unit => [unit.anchors.files[0], unit.anchors.symbols])).toEqual([
      ["src/engine/storage/migrations.ts", []],
      ["native/src/logic/front.rs", ["Literal"]],
    ]);
  });

  it("reads a long new decision doc in chunks so a late decision still reaches the judge", () => {
    const filler = Array.from({ length: 40 }, (_unused, index) => `Background line ${index} describing the spike setup in some detail.`);
    const lines = ["# Facts engine: plan", ...filler, "", "Replacement of his executor is sequenced with him, not done unilaterally."];
    const change = commit([file("docs/facts/PLAN.md", lines, [], "added")]);
    const passages = positionPassages(change, { before: new Map(), after: new Map([["docs/facts/PLAN.md", lines.join("\n")]]) });
    expect(passages.some(passage => passage.text.includes("sequenced with him"))).toBe(true);
  });
});

describe("artefactUnits", () => {
  it("records a new harness or tool with its doc comment, and leaves other new modules, tests and new exports out", () => {
    const harness = ["/** `run --engine slice`: the first slice over the whole worktree. */", "export function runSlice(dir: string): void {}"];
    const module = ["//! The setfix evaluator: runs recursive rules to a fixed point over sets.", "pub fn run_setfix(program: &Program) -> Sets {", "}"];
    const change = commit([
      file("benchmark/workspaces/slice.ts", harness, [], "added"),
      file("benchmark/workspaces/slice.test.ts", ["it('runs', () => {});"], [], "added"),
      file("native/src/logic/native/setfix.rs", module, [], "added"),
      file("src/engine/store.ts", ["export function compileOnce(table: Table) {}"]),
    ], { subject: "bench: slice" });
    const texts = {
      before: new Map([["src/engine/store.ts", "export function open() {}"]]),
      after: new Map([
        ["benchmark/workspaces/slice.ts", harness.join("\n")],
        ["benchmark/workspaces/slice.test.ts", "it('runs', () => {});"],
        ["native/src/logic/native/setfix.rs", module.join("\n")],
        ["src/engine/store.ts", "export function open() {}\nexport function compileOnce(table: Table) {}"],
      ]),
    };
    expect(artefactUnits(change, texts).map(unit => [unit.text, unit.anchors])).toEqual([
      ["adds benchmark/workspaces/slice.ts: `run --engine slice`: the first slice over the whole worktree. (runSlice)", { files: ["benchmark/workspaces/slice.ts"], symbols: ["runSlice"] }],
    ]);
  });
});

describe("link-1 precision regressions", () => {
  it("takes no hand-off from a table separator or a row whose status is closed", () => {
    const lines = ["## Open items", "| id | item | status |", "| --- | ---: | ---: |", "| O22 | P's text prefilter in `src/engine/store.ts` | closed (2026-10-29): option (a) |", "| O41 | Producer labels in `src/engine/store.ts` | open |"];
    const change = commit([file("docs/notes.md", lines)]);
    const texts = { before: new Map(), after: new Map([["docs/notes.md", lines.join("\n")]]) };
    expect(handoffUnits(change, texts, resolve).map(unit => unit.text)).toEqual(["| O41 | Producer labels in `src/engine/store.ts` | open | (docs/notes.md)"]);
  });

  it("reads no contract from tests, benchmarks or a placeholder table name", () => {
    const vtab = ['const SCHEMA: &CStr = c"CREATE TABLE x(file, id, parent, kind)";'];
    const change = commit([
      file("native/src/sql/nodes.rs", vtab),
      file("native/src/sql/lower_tests.rs", ["conn.execute(\"CREATE TABLE points(a, b)\")"]),
      file("benchmark/candidate-pruning.sql", ["SELECT 1;"]),
      file("native/src/content_store/schema.rs", ["CREATE TABLE facts_results (content BLOB)"]),
    ], { subject: "sql" });
    expect(contractUnits(change, noTexts).map(unit => unit.text)).toEqual(["changes the schema of facts_results in native/src/content_store/schema.rs (sql)"]);
  });

  it("states a migration step appended to a registry list with the list's rule (sf-12)", () => {
    const after = [
      "/** This build's steps, oldest first. Append only. */",
      "export const RERAIL_MIGRATIONS: readonly Migration[] = [",
      "  {",
      '    id: "repositories-follow-gitignore",',
      "    up(database) {},",
      "  },",
      "  {",
      '    id: "rule-approvals",',
      "    up(database) {},",
      "  },",
      "];",
    ];
    const migrations: FileChange = { path: "src/engine/storage/migrations.ts", oldPath: "src/engine/storage/migrations.ts", status: "modified", added: [{ start: 7, lines: after.slice(6, 10) }], removed: [] };
    const change = commit([migrations], { subject: "Packs: rule approvals" });
    const texts = { before: new Map(), after: new Map([[migrations.path, after.join("\n")]]) };
    expect(contractUnits(change, texts).map(unit => [unit.text, unit.anchors.symbols])).toEqual([
      ["changes the format in src/engine/storage/migrations.ts (Packs: rule approvals)", []],
      ['appends rule-approvals to RERAIL_MIGRATIONS in src/engine/storage/migrations.ts ("This build\'s steps, oldest first. Append only.") (Packs: rule approvals)', ["RERAIL_MIGRATIONS"]],
    ]);
  });
});

describe("concept anchors", () => {
  const folders = folderIndex([
    "native/src/content_store/schema.rs",
    "native/src/code_rule_dsl/sql/exec.rs",
    "native/src/code_rule_dsl/matcher/anchors/sql/lower.rs",
    "src/engine/merge-assist/candidate.ts",
    "src/runtime/merge-assist/capture.ts",
    "a/sql/x.rs",
    "b/sql/x.rs",
    "c/sql/x.rs",
    "d/sql/x.rs",
  ]);

  it("names a subsystem's home folders, shallowest first, and none when that is ambiguous", () => {
    expect(subsystemFolders("content store", folders)).toEqual(["native/src/content_store/"]);
    expect(subsystemFolders("Merge Assist engine", folders)).toEqual(["src/engine/merge-assist/", "src/runtime/merge-assist/"]);
    expect(subsystemFolders("SQL engine", folders)).toEqual([]);
    expect(subsystemFolders("", folders)).toEqual([]);
  });

  it("keeps the decision and open-item ids a passage writes, canonical", () => {
    expect(conceptIds(["0035", "decision 8", "O41", "o7", "O99", "t18", "35"], "## 0035: bounded rounds\nAs decision 0008 says, O41 and O7 stay open.")).toEqual(["0035", "0008", "O41", "O7"]);
  });
});
