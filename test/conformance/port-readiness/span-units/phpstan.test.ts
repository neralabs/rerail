import assert from "node:assert/strict";
import { test } from "vitest";

import { errorSpan, phpstanSpan, PHPSTAN_SPAN_PARTS } from "../../../../conformance/port-readiness/span-units/phpstan.js";

// A CRLF, a lone CR and an LF line end, a 2-byte and a 4-byte character, then an undefined variable on line 4.
const SOURCE = "<?php\r\nfunction f() {\r  $x = \"é\u{1F600}\";\n\techo $undefined;\r\n}\n";

test("an engine span is a PHPStan position: its line alone, lines broken as PHP's tokenizer breaks them", () => {
  // PHPStan 2.2.16 on PHP 8.5.11 reports variable.undefined on SOURCE at line 4 (its own output, level 0).
  const start = Buffer.byteLength(SOURCE.slice(0, SOURCE.indexOf("$undefined")), "utf8");
  assert.deepEqual(phpstanSpan(SOURCE, start, start + "$undefined".length), errorSpan({ line: 4 }));
  assert.deepEqual(errorSpan({ line: 4 }), { startLine: 3, startCol: null, endLine: null, endCol: null });
  // PHPStan reports no column and no end, so only the start line is compared.
  assert.deepEqual(PHPSTAN_SPAN_PARTS, { endLine: false, endCol: false });
});
