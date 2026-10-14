import assert from "node:assert/strict";
import { test } from "vitest";

import { trojansourcePoint } from "../../../conformance/gates/span-units/bandit.js";

// A form feed inside a comment, a CRLF line end, then a right-to-left override after a 2-byte and a 4-byte character.
const SOURCE = "# a\fb\r\nv = \"é\u{1F600}‮\"\n";
// The override on physical line 2 (0-based 1), after 11 UTF-8 bytes: `v = "` (5), `é` (2), the emoji (4).
const POINT = { line: 1, column: 11 };

test("B613's line is counted as the pinned Bandit release counts it", () => {
  // Bandit 1.9.4 counts str.splitlines() lines, so the form feed ends a line: it reports line 3 and the 1-based
  // code-point column 8. (Its own output on SOURCE.)
  assert.deepEqual(trojansourcePoint(SOURCE, 3, 8), POINT);
});

test("B613's lines break on every separator str.splitlines() knows", () => {
  assert.deepEqual(trojansourcePoint("a\vb\x1cc\x85d ‮", 5, 1), { line: 0, column: 11 });
  assert.deepEqual(trojansourcePoint("a\r\n‮", 2, 1), { line: 1, column: 0 });
});
