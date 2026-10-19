import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "vitest";

import { readGatesManifest } from "../../../conformance/port-readiness/manifest.js";
import { gatedToolPacks, ROOT } from "../../../conformance/port-readiness/plan.mjs";
import { inlineTextsSource, recordingNote, withRecordingNote } from "../../../conformance/recorders/port-readiness/record.js";

const ROWS = [
  "| `upstream/ruff/fixtures/a.py` | `crates/a.py` | verbatim |",
  "| `upstream/ruff/fixtures/b.py` | `crates/b.py` | verbatim |",
];
const TABLE = ["| File | Upstream | Copy |", "| --- | --- | --- |", ...ROWS, "", "Paths are relative to the pack directory.", ""].join("\n");
const NOTE_START = "> `recording.json` is a port readiness recording";

test("a recording that holds upstream text is listed once, in a note under UPSTREAM.md's file table and never as a row of it", () => {
  const once = withRecordingNote(TABLE, "`mod.rs`: test strings");
  const lines = once.split("\n");
  // Pack check 6 reads every row as a file the pack carries, and the recording is not one.
  assert.deepEqual(lines.filter((line) => line.startsWith("| `upstream/")), ROWS);
  assert.equal(lines.filter((line) => line.startsWith(NOTE_START)).length, 1);
  assert.equal(lines.indexOf(ROWS[1]!) + 2, lines.findIndex((line) => line.startsWith(NOTE_START)));
  assert.match(once, /tests \(`mod\.rs`: test strings\), redistributed/u);
  assert.ok(lines.every((line) => line.length <= 120));
  assert.match(once, /Paths are relative to the pack directory\.\n$/u);

  // Recording again with the same source changes nothing; another source replaces the note.
  assert.equal(withRecordingNote(once, "`mod.rs`: test strings"), once);
  const twice = withRecordingNote(once, "`mod.rs`, `linter.rs`: test strings");
  assert.equal(twice.split("\n").filter((line) => line.startsWith(NOTE_START)).length, 1);
  assert.match(twice, /`linter\.rs`/u);
  assert.doesNotMatch(twice, /tests \(`mod\.rs`: test strings\)/u);
});

test("a committed note that says the same in other line breaks is left as it is", () => {
  const words = recordingNote("`tests/*.js`: the `code` of each RuleTester case").join(" ").replace(/^> /u, "").split(/ (?:> )?/u);
  const rewrapped = [];
  for (let at = 0; at < words.length; at += 9) rewrapped.push(`> ${words.slice(at, at + 9).join(" ")}`);
  const committed = [TABLE.trimEnd(), "", ...rewrapped, ""].join("\n");
  assert.equal(withRecordingNote(committed, "`tests/*.js`: the `code` of each RuleTester case"), committed);
});

test("UPSTREAM.md without a file table is an error, not a silent skip", () => {
  assert.throws(() => withRecordingNote("# Ruff\n\nNo table.\n", "x"), /no file table/u);
});

test("every committed recording note is the recorder's own output, so a re-record leaves it byte for byte", () => {
  let notes = 0;
  for (const { pack, tool } of gatedToolPacks(ROOT)) {
    const manifest = readGatesManifest(ROOT, pack, tool);
    const lines = readFileSync(join(ROOT, "packs", pack, "licenses", manifest.rerail.project, "UPSTREAM.md"), "utf8").split("\n");
    const at = lines.findIndex((line) => line.startsWith(NOTE_START));
    if (at < 0) continue;
    let end = at;
    while (end + 1 < lines.length && lines[end + 1]!.startsWith(">")) end += 1;
    assert.deepEqual(lines.slice(at, end + 1), recordingNote(inlineTextsSource(manifest)), `${pack}/${tool}`);
    notes += 1;
  }
  assert.ok(notes > 0);
});
