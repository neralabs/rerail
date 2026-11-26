# php-01: six php rules against their JSON originals

Checked with `translations:check` (through `ptcheck`) on the pack's examples, the recorded tool fixtures and
`samples-php`; ground truth from the fixtures' recorded findings (`recorded/php/expected.json`: PHP_CodeSniffer
4.0.4, PHPStan 2.2.16) and the tools' sources and tests. PHP, phpcs and PHPStan are not on the boxes, so no
counterexample was run on a tool; cases were run through `rerail-engine run-policies` on a box.

## phpcs-squiz-php-eval: same

The baseline's 8 translation-only findings on `samples-php` were `eval` under a `// phpcs:ignore` (bare, by sniff
`Squiz.PHP.Eval` and by code `Squiz.PHP.Eval.Discouraged`): the box's engine of the baseline did not honour the
translation's `suppress phpcs`. The engine installed since does, and the rule is the same unchanged.

## phpcs-generic-code-analysis-empty-statement: same

As eval: the 5 translation-only findings were blocks under `// phpcs:ignore Generic.CodeAnalysis.EmptyStatement.Detected…`,
honoured by the current engine. Unchanged.

## phpcs-generic-php-lower-case-constant: follows PHP_CodeSniffer

`LowerCaseConstantUnitTest.1.inc.php:83`, `['type' => Null::TYPE_STRING]`: tree-sitter-php reads `Null` as the
keyword `null` inside an ERROR node with the `::`, and the translation reported it. PHP_CodeSniffer tokenizes a
class name there and reports nothing (the line is not in its recorded findings); the original drops it because it
skips findings in the file's error regions. The translation now leaves a `null`/`boolean` alone when its parent is
an ERROR holding `::`.

## phpcs-generic-php-no-silenced-errors: translation bug fixed

`fs_call(s, v)` did not require `s` to be an `@` expression, so the second clause ("recheck the outcome") reported
every statement whose child is a call to `mkdir`, `rmdir` or `unlink`, `@` or not: `unlink($tempFile);` in
`system/View/View.php:338` and 75 more. `fs_call` now requires `error_suppression_expression`, as the JSON's
`kind`.

## phpstan-argument-sprintf: follows PHPStan

- **Engine gap**: php/format's `printf_needs` computes the largest argnum with `top = max n : { printf_numbered(s,
  n) }`; the engine evaluates that aggregate to 4611686018427387904 (2^62) for every format with an argnum
  (`sprintf('%1$%', $i)` gives "more placeholders (4611686018427387904) than values given (1)"). Every numbered
  format whose values matched was reported (fixtures `printf-format-parser.php:8,13,14,15`, `printf.php:12`;
  samples `ConstantFunctionProviderTest.php:162`). The rule computes the need itself (`sprintf_needs`: the largest
  argnum with no larger one, no aggregate). Proposed: fix the aggregate in the engine, or take the same rewrite
  into php/format.
- **Formats the translation did not know** (original-only on samples): a nowdoc format,
  `FailedMessagesShowCommandTest.php:346` (`\sprintf(<<<'EOF' … "%s" … %%d … EOF, __FILE__, $line)`: 1 placeholder,
  2 values), and a variable written twice, `DebuggerTest.php:770,772` (`$expectedText = <<<EXPECTED …` a second
  time, then `sprintf($expectedText, $a, $b)` with no placeholder). PHPStan reads both constant strings at the call.
  The new `lib/php/php-01.rerail` gives `php_fmt_string`: heredocs and nowdocs (core's `value`; php/values'
  `php_value` reads core's `str`, which has no row for them), parentheses, `.` concatenations of strings and
  ints, and a variable at its last write when that write dominates the use (the JSON's `follow.php` cases).

## phpstan-argument-sscanf: follows PHPStan

`bug-14567.php:8,11,20` (`sscanf($s, "%d\0%d", $a)`): scanf.c reads the format as a C string, so placeholders
after a NUL assign nothing, and PHPStan's fixture expects no error. The rule cuts the format at its first NUL
before counting.

**Engine gap** met on the way: core's `str`/`value` decodes a PHP octal escape wrongly. `"\033"` gives NUL "33"
(PHP: ESC), `"\0003"` NUL "003" (PHP: NUL "3"), `"\101"` "101" (PHP: "A"). Cutting at every NUL reported
`sscanf($code, "\033[%d;%dR", $row, $col)` (symfony `Console/Cursor.php:200`, 3 copies in samples-php) as having
no placeholders. The cut is at the first NUL whose following digits complete a zero escape (`\0`, `\00`, `\000`),
which is right for every octal escape; `"\x005"` (NUL "5") is then not cut, a case not in any corpus. Proposed:
decode octal escapes in core (`\[0-7]{1,3}`, as PHP's zend_language_scanner).

## Library

New: `lib/php/php-01.rerail`, `php_fmt_string(n, s)`, used by both phpstan rules. It also adds rows to
php/format's `php_format` so the format relations hold for those strings. Its recursion is in two layers, down
the tree (concatenation, parentheses) and back in source (a variable's last write, `e < u`): one cycle moving both
ways is refused by the engine as an unbounded value cycle.

Proposed changes to existing libraries:
- php/values: `php_value(n, "string", v)` for heredoc and nowdoc should read core's `value` (decoded, closing
  indentation removed, no row when interpolated); `str` has no row for them, so today no heredoc has a value.
- php/format: `printf_needs` without the `max` aggregate (as above), until the engine's aggregate is fixed.
- php/format: scanf formats end at their first NUL (`scanf_assigns`, `scanf_invalid`), as the sscanf rule does
  now.
