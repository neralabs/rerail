import assert from "node:assert/strict";
import { test } from "vitest";

import { bladestanSpan, templateLineSpan, BLADESTAN_SPAN_PARTS } from "../../../../conformance/port-readiness/span-units/bladestan.js";
import { CONTEXT_FILES } from "../../../../conformance/port-readiness/recording/corpus-answers.js";
import { CASE_LAYOUTS } from "../../../../conformance/recorders/port-readiness/record.js";

// A multibyte character on line 1, a CRLF line end, a lone CR inside line 2, then an `@if` on line 3 of the template.
const SOURCE = "<p>é\u{1F600}</p>\r\n{{ $a }}\r{{ $b }}\n@if (true)\n";

test("an engine span is a Bladestan template line: lines end at \\n only, with no column and no end", () => {
  const second = Buffer.byteLength(SOURCE.slice(0, SOURCE.indexOf("{{ $b")), "utf8");
  const third = Buffer.byteLength(SOURCE.slice(0, SOURCE.indexOf("@if")), "utf8");
  // Bladestan splits a template at PHP_EOL, so the lone \r leaves `{{ $b }}` on line 2.
  assert.deepEqual(bladestanSpan(SOURCE, second, second + 1), templateLineSpan({ templateLine: 2 }));
  assert.deepEqual(bladestanSpan(SOURCE, third, third + 3), templateLineSpan({ templateLine: 3 }));
  assert.deepEqual(templateLineSpan({ templateLine: 3 }), { startLine: 2, startCol: null, endLine: null, endCol: null });
  assert.deepEqual(BLADESTAN_SPAN_PARTS, { endLine: false, endCol: false });
});

test("Bladestan's test templates are committed as fixtures/skeleton/resources/<path>, as generate.py copied them", () => {
  const place = (fixture: string) => CASE_LAYOUTS.bladestan.file({ fixture, path: fixture, bytes: Buffer.from("") }, () => []);
  assert.equal(place("tests/skeleton/resources/views/foo.blade.php"), "fixtures/skeleton/resources/views/foo.blade.php");
  assert.equal(place("tests/skeleton/resources/namespace/Test/namespacedView.blade.php"), "fixtures/skeleton/resources/namespace/Test/namespacedView.blade.php");
  assert.equal(place("tests/Rules/Fixture/once.php"), null);
  assert.deepEqual(CONTEXT_FILES.bladestan("resources/views/a.blade.php"), []);
});
