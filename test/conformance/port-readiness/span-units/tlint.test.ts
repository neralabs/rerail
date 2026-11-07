import assert from "node:assert/strict";
import { test } from "vitest";

import { lintSpan, tlintSpan, TLINT_SPAN_PARTS } from "../../../../conformance/port-readiness/span-units/tlint.js";

// A CRLF line end, a 2-byte character, a lone CR, then a badly spaced echo on line 3 and a @dd on line 4.
const SOURCE = "{{ $a }}\r\n<p>é</p>\r{{$b}}\n@dd($c)\n";

test("an engine span is a tlint position: its line alone, lines split at \\r\\n, \\r and \\n", () => {
  // tlint 9.7.0 on PHP 8.5.11 reports SpacesAroundBladeRenderContent on SOURCE at line 3 and NoDumpDirectives at line 4 (its own output).
  const echo = Buffer.byteLength(SOURCE.slice(0, SOURCE.indexOf("{{$b}}")), "utf8");
  const dd = Buffer.byteLength(SOURCE.slice(0, SOURCE.indexOf("@dd")), "utf8");
  assert.deepEqual(tlintSpan(SOURCE, echo, echo + 6), lintSpan({ line: 3 }));
  assert.deepEqual(tlintSpan(SOURCE, dd, dd + 7), lintSpan({ line: 4 }));
  assert.deepEqual(lintSpan({ line: 3 }), { startLine: 2, startCol: null, endLine: null, endCol: null });
  // An offset between the `\r` and `\n` of line 1's CRLF is still on line 1.
  const crlf = Buffer.byteLength(SOURCE.slice(0, SOURCE.indexOf("\r\n") + 1), "utf8");
  assert.deepEqual(tlintSpan(SOURCE, crlf, crlf), lintSpan({ line: 1 }));
  // tlint reports no column and no end, so none is compared.
  assert.deepEqual(TLINT_SPAN_PARTS, { endLine: false, endCol: false });
});
