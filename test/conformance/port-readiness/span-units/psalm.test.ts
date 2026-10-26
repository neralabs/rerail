import assert from "node:assert/strict";
import { test } from "vitest";

import { psalmSpan, PSALM_SPAN_PARTS } from "../../../../conformance/port-readiness/span-units/psalm.js";
import { CASE_LAYOUTS } from "../../../../conformance/recorders/port-readiness/record.js";

// A tab, a 2-byte and a 4-byte character before a call on line 2, a CRLF line end, then a call at the start of line 3.
const SOURCE = "<?php\n\t$x = \"é\u{1F600}\"; var_dump($x);\r\nshell_exec(\"a\");\n";

test("an engine span is a Psalm selection: lines as PHP breaks them, columns in UTF-8 bytes, start and end", () => {
  // Psalm 6.19.1 on PHP 8.5.11 reports ForbiddenCode on SOURCE at from 22 to 34 (line 2, columns 17 to 29) and from 37 to
  // 52 (line 3, columns 1 to 16): its own output.
  const varDump = Buffer.byteLength(SOURCE.slice(0, SOURCE.indexOf("var_dump")), "utf8");
  const shellExec = Buffer.byteLength(SOURCE.slice(0, SOURCE.indexOf("shell_exec")), "utf8");
  assert.deepEqual([varDump, shellExec], [22, 37]);
  assert.deepEqual(psalmSpan(SOURCE, 22, 34), { startLine: 1, startCol: 16, endLine: 1, endCol: 28 });
  assert.deepEqual(psalmSpan(SOURCE, 37, 52), { startLine: 2, startCol: 0, endLine: 2, endCol: 15 });
  // Psalm reports both ends of every issue, so both are compared.
  assert.deepEqual(PSALM_SPAN_PARTS, { endLine: true, endCol: true });
});

test("a selection on the first line is read from its offsets, not from Psalm's column_from, which is one short there", () => {
  // Psalm 6.19.1 reports `<?php var_dump(1);` from 6 to 17 with column_from 6: there is no `\n` before it to count from.
  assert.deepEqual(psalmSpan("<?php var_dump(1);\n", 6, 17), { startLine: 0, startCol: 6, endLine: 0, endCol: 17 });
});

test("a Psalm case is committed where upstream/psalm/generate.py wrote it", () => {
  const file = (fixture: string) => CASE_LAYOUTS.psalm.file({ fixture, path: "x.php", bytes: Buffer.from("") }, () => []);
  assert.equal(file("tests/UnusedVariableTest.php#invalid/SKIPPED-unusedGlobalVariable"), "fixtures/UnusedVariableTest/invalid/SKIPPED-unusedGlobalVariable.php");
  assert.equal(file("tests/ForbiddenCodeTest.php#valid/execWithSuppression"), "fixtures/ForbiddenCodeTest/valid/execWithSuppression.php");
  // A case name that is no file name, or a test outside tests/, is not copied.
  assert.equal(file("tests/ForbiddenCodeTest.php#valid/a b"), null);
  assert.equal(file("tests/Config/PluginTest.php#valid/x"), null);
  assert.deepEqual(CASE_LAYOUTS.psalm.row?.("tests/ForbiddenCodeTest.php#invalid/exec"), { upstream: "tests/ForbiddenCodeTest.php", copy: "derived" });
});
