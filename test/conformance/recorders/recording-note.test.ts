import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "vitest";

import { readGatesManifest } from "../../../conformance/port-readiness/manifest.js";
import { gatedToolPacks, ROOT } from "../../../conformance/port-readiness/plan.mjs";
import { licenseFiles } from "../../../conformance/port-readiness/recorded-adapter.js";
import { inlineTextsSource, recordingNote, withRecordingNote } from "../../../conformance/recorders/port-readiness/record.js";

const ROWS = [
  "| `upstream/ruff/fixtures/a.py` | `crates/a.py` | verbatim |",
  "| `upstream/ruff/fixtures/b.py` | `crates/b.py` | verbatim |",
];
const TABLE = ["| File | Upstream | Copy |", "| --- | --- | --- |", ...ROWS, "", "Paths are relative to the pack directory.", ""].join("\n");
const NOTE_START = "> `recording.json` is a port readiness recording";

test("a recording that holds upstream text is listed once, in a note under UPSTREAM.md's file table and never as a row of it", () => {
  const once = withRecordingNote(TABLE, "`mod.rs`: test strings", ["LICENSE"]);
  const lines = once.split("\n");
  // Pack check 6 reads every row as a file the pack carries, and the recording is not one.
  assert.deepEqual(lines.filter((line) => line.startsWith("| `upstream/")), ROWS);
  assert.equal(lines.filter((line) => line.startsWith(NOTE_START)).length, 1);
  assert.equal(lines.indexOf(ROWS[1]!) + 2, lines.findIndex((line) => line.startsWith(NOTE_START)));
  assert.match(once, /tests \(`mod\.rs`: test strings\), redistributed/u);
  assert.ok(lines.every((line) => line.length <= 120));
  assert.match(once, /Paths are relative to the pack directory\.\n$/u);

  // Recording again with the same source changes nothing; another source replaces the note.
  assert.equal(withRecordingNote(once, "`mod.rs`: test strings", ["LICENSE"]), once);
  const twice = withRecordingNote(once, "`mod.rs`, `linter.rs`: test strings", ["LICENSE"]);
  assert.equal(twice.split("\n").filter((line) => line.startsWith(NOTE_START)).length, 1);
  assert.match(twice, /`linter\.rs`/u);
  assert.doesNotMatch(twice, /tests \(`mod\.rs`: test strings\)/u);
});

test("a committed note that says the same in other line breaks is left as it is", () => {
  const words = recordingNote("`tests/*.js`: the `code` of each RuleTester case", ["LICENSE"]).join(" ").replace(/^> /u, "").split(/ (?:> )?/u);
  const rewrapped = [];
  for (let at = 0; at < words.length; at += 9) rewrapped.push(`> ${words.slice(at, at + 9).join(" ")}`);
  const committed = [TABLE.trimEnd(), "", ...rewrapped, ""].join("\n");
  assert.equal(withRecordingNote(committed, "`tests/*.js`: the `code` of each RuleTester case", ["LICENSE"]), committed);
});

test("the note names the licence files beside UPSTREAM.md by their own names", () => {
  assert.match(recordingNote("x", ["license.md"]).join(" ").replace(/\s*>\s*/gu, " "), /`license\.md` beside this file is this upstream's own/u);
  assert.match(recordingNote("x", ["LICENSE-APACHE", "LICENSE-MIT"]).join(" ").replace(/\s*>\s*/gu, " "), /`LICENSE-APACHE` and `LICENSE-MIT` beside this file are this upstream's own/u);
  assert.throws(() => recordingNote("x", []), /no licence file/u);
});

test("UPSTREAM.md without a file table is an error, not a silent skip", () => {
  assert.throws(() => withRecordingNote("# Ruff\n\nNo table.\n", "x", ["LICENSE"]), /no file table/u);
});

test("every committed recording note is the recorder's own output, so a re-record leaves it byte for byte", () => {
  let notes = 0;
  for (const { pack, tool } of gatedToolPacks(ROOT)) {
    const manifest = readGatesManifest(ROOT, pack, tool);
    const upstreamMd = join(ROOT, "packs", pack, "licenses", manifest.rerail.project, "UPSTREAM.md");
    const lines = readFileSync(upstreamMd, "utf8").split("\n");
    const at = lines.findIndex((line) => line.startsWith(NOTE_START));
    if (at < 0) continue;
    let end = at;
    while (end + 1 < lines.length && lines[end + 1]!.startsWith(">")) end += 1;
    assert.deepEqual(lines.slice(at, end + 1), recordingNote(inlineTextsSource(manifest), licenseFiles(dirname(upstreamMd))), `${pack}/${tool}`);
    notes += 1;
  }
  assert.ok(notes > 0);
});
