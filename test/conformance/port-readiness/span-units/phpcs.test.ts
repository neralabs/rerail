import assert from "node:assert/strict";
import { test } from "vitest";

import { fileTypeOf } from "../../../../conformance/port-readiness/engine.js";
import { messageSpan, phpcsSpan, PHPCS_SPAN_PARTS } from "../../../../conformance/port-readiness/span-units/phpcs.js";
import { phpRegex } from "../../../../conformance/recorders/port-readiness/languages/php.js";

// A tab, a 2-byte and a 4-byte character before an @ on line 2, a CRLF line end, then an @ at the start of line 3.
const SOURCE = "<?php\n\t$x = \"é\u{1F600}\" . @f();\r\n@g();\n";

test("an engine span is a PHP_CodeSniffer position: its line, and its column in characters with a tab width of 0", () => {
  // PHP_CodeSniffer 4.0.4 on PHP 8.5.11 reports Generic.PHP.NoSilencedErrors on SOURCE at 2:14 and 3:1 (its own output).
  const first = Buffer.byteLength(SOURCE.slice(0, SOURCE.indexOf("@")), "utf8");
  const second = Buffer.byteLength(SOURCE.slice(0, SOURCE.lastIndexOf("@")), "utf8");
  assert.deepEqual(phpcsSpan(SOURCE, first, first + 1), messageSpan({ line: 2, column: 14 }));
  assert.deepEqual(phpcsSpan(SOURCE, second, second + 1), messageSpan({ line: 3, column: 1 }));
  assert.deepEqual(messageSpan({ line: 2, column: 14 }), { startLine: 1, startCol: 13, endLine: null, endCol: null });
  // PHP_CodeSniffer reports no end, so none is compared.
  assert.deepEqual(PHPCS_SPAN_PARTS, { endLine: false, endCol: false });
});

test("a PCRE pattern is read as JavaScript only where it means the same", () => {
  assert.deepEqual(phpRegex("/^foo\\d+$/i"), /^foo\d+$/iu);
  assert.deepEqual(phpRegex("#a|b#"), /a|b/u);
  assert.deepEqual(phpRegex("{x}su"), /x/su);
  assert.deepEqual(phpRegex("`\\$this`"), /\$this/u);
  // Modifiers and syntax PCRE has and JavaScript does not: no translation explains anything.
  assert.equal(phpRegex("/a b/x"), undefined);
  assert.equal(phpRegex("/a(?>b)/"), undefined);
  assert.equal(phpRegex("/a++/"), undefined);
  assert.equal(phpRegex("/\\Aabc\\z/"), undefined);
  assert.equal(phpRegex("/[[:alpha:]]/"), undefined);
  assert.equal(phpRegex("abc"), undefined);
});

test("a corpus file's type is its longest dialect suffix, so a Blade template is no PHP file", () => {
  assert.equal(fileTypeOf("resources/views/welcome.blade.php"), ".blade.php");
  assert.equal(fileTypeOf("src/Foo.php"), ".php");
  assert.equal(fileTypeOf("src/Foo.inc"), ".inc");
  assert.equal(fileTypeOf("src/Foo.BLADE.PHP"), ".BLADE.PHP");
  assert.equal(fileTypeOf("tests/FooUnitTest.inc.php"), ".php");
  // A file named only by the suffix routes nowhere, as the engine reads it.
  assert.equal(fileTypeOf("views/.blade.php"), ".php");
});
