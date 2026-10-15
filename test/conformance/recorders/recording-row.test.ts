import assert from "node:assert/strict";
import { test } from "vitest";

import { withRecordingRow } from "../../../conformance/recorders/gates/record.js";

const TABLE = [
  "| File | Upstream | Copy |",
  "| --- | --- | --- |",
  "| `upstream/ruff/fixtures/a.py` | `crates/a.py` | verbatim |",
  "| `upstream/ruff/fixtures/b.py` | `crates/b.py` | verbatim |",
  "",
  "Paths are relative to the pack directory.",
].join("\n");

test("a recording that holds upstream text is listed once in UPSTREAM.md, as excerpts, keeping a sorted table sorted", () => {
  const once = withRecordingRow(TABLE, "ruff", "`mod.rs`: test strings");
  assert.match(once, /\| `upstream\/ruff\/gates\.json` \| `mod\.rs`: test strings \| excerpts \|/u);
  const rows = once.split("\n").filter((line) => line.startsWith("| `upstream/"));
  assert.deepEqual(rows, [...rows].sort());
  const twice = withRecordingRow(once, "ruff", "`mod.rs`, `linter.rs`: test strings");
  assert.equal(twice.split("\n").filter((line) => line.includes("gates.json")).length, 1);
  assert.match(twice, /`linter\.rs`/u);
  assert.match(twice, /Paths are relative to the pack directory\.$/u);
});

test("UPSTREAM.md without a file table is an error, not a silent skip", () => {
  assert.throws(() => withRecordingRow("# Ruff\n\nNo table.\n", "ruff", "x"), /no file table/u);
});
