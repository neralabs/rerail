import assert from "node:assert/strict";
import { test } from "vitest";

import { bladeParserSpan, BLADE_PARSER_SPAN_PARTS, validationSpan } from "../../../../conformance/port-readiness/span-units/blade-parser.js";

/**
 * Templates with one `@dd(1)` each, and where stillat/blade-parser 2.1.0 on PHP 8.5.11 reports DebugDirectiveValidator
 * on it (its own output: the BladeError's 1-based start line and column).
 */
const REPORTED: [template: string, line: number, column: number][] = [
  ["ab\n  @dd(1)\n", 2, 3],
  ["\tx @dd(1)", 1, 4],
  ["a\nbb\n   @dd(1)", 3, 4],
  // \r\n and \r are line ends.
  ["a\r\n@dd(1)", 2, 1],
  ["a\r@dd(1)", 2, 1],
  // A node's character offset is looked up among the line ends' byte offsets: after multibyte characters the line and
  // column are where the character count lands.
  ["éééé\n  @dd(1)\n", 1, 8],
  ["é\né\né\né\n@dd(1)", 3, 3],
  ["😀 @dd(1)", 1, 3],
  // A line end at offset 0 is read as no previous line end, so the next line's columns count from the template's start.
  ["\n  @dd(1)\n", 2, 4],
  ["\nx\n  @dd(1)", 3, 3],
];

test("an engine span is a stillat/blade-parser position: the start line and column it numbers a node by", () => {
  for (const [template, line, column] of REPORTED) {
    const start = Buffer.byteLength(template.slice(0, template.indexOf("@")), "utf8");
    assert.deepEqual(bladeParserSpan(template, start, start + 3), validationSpan({ line, column }), JSON.stringify(template));
  }
  // An offset between the `\r` and `\n` of a CRLF is where `\r\n`'s normalization leaves it: the end of its line, as
  // the offset before the `\r` is.
  assert.deepEqual(bladeParserSpan("a\r\n@dd(1)", 2, 2), bladeParserSpan("a\r\n@dd(1)", 1, 1));
  assert.deepEqual(bladeParserSpan("a\r\n@dd(1)", 2, 2), validationSpan({ line: 1, column: 2 }));
  assert.deepEqual(validationSpan({ line: 2, column: 3 }), { startLine: 1, startCol: 2, endLine: null, endCol: null });
  // A finding is compared by its start: the rules anchor a point.
  assert.deepEqual(BLADE_PARSER_SPAN_PARTS, { endLine: false, endCol: false });
});
