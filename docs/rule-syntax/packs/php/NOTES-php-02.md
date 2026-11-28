# php-02: five PHPStan and Psalm rules checked against their originals

Rules: phpstan-array-duplicate-key, phpstan-if-always-false, phpstan-missing-function-return-type,
phpstan-random-int-parameters, psalm-forbidden-code. Each was run alone with `translations:check` (ptcheck), and
every differing key was looked up in the tool's own recording (`recorded/php/expected.json`: PHPStan 2.2.16 and
Psalm 6.19.1 on their test data) or, for a key the recording does not cover, in the tool's source.

| rule | status | right | what differs |
|---|---|---|---|
| phpstan-array-duplicate-key | different | translation | fixtures: 11 keys PHPStan reports, only the translation |
| phpstan-if-always-false | same | | |
| phpstan-missing-function-return-type | same | | |
| phpstan-random-int-parameters | different | translation | fixtures: 7 keys PHPStan reports, only the translation |
| psalm-forbidden-code | different | neither | samples-php: 15 `@psalm-suppress`ed keys (original right); fixtures: 1 key in a file with syntax errors (translation right) |

## New library: `lib/php/php-02.rerail`

`php_parse_error(d)`: document d holds a tree-sitter ERROR node. The four PHPStan rules share it. PHPStan parses
a file before any rule runs, and a file its parser rejects gets only `phpstan.parse` (FileAnalyser catches
ParserErrorsException and runs no rule), so none of its rules reports in it. The product's path skips the same
files ("source-incomplete: file has syntax errors"). Counterexample, fixtures
`phpcs/Generic/PHP/LowerCaseConstantUnitTest.1.inc.php` (`class True extends Null`, `namespace False;` after code):
the translation reported missingType.return at 4, 7, 10, 20, 23, 26, 36, 39, 42 and 81, and PHPStan reports none of
them. With the guard, missing-function-return-type is `same`.

## phpstan-array-duplicate-key: the translation follows PHPStan

The 11 translation-only keys are all in PHPStan's recording: bug-15244.php:49, bug-15247-php81.php:14 and 19,
bug-15247.php:19 and 34, duplicate-keys.php:32, 63, 67, 69, 76 and 105. The original compares keys as text and
does not number implicit keys or follow spreads and constants; php/arrays does, e.g. `['x', 0 => 'y']` (key 0
twice). Ten keys PHPStan reports are missed by both sides (bug-15244.php:38, bug-15247.php:66, duplicate-keys.php:55,
102, 103, 128, 139, 151, 152, 171): keys from PHPDoc types and `$i++`, which need types; not a difference.

## phpstan-if-always-false: now the same

The original reported `if ($e)` at composer's `src/Composer/Installer/InstallationManager.php:251` (samples-php) and
the translation did not: `$e = null;` at 211, then `try { … } catch (\Exception $e) { throw $e; }`, then `if ($e)`.
PHPStan reports it: the catch always throws, so `$e` is null at 251. php/values' `php_follow` needs a variable with
one write, and the catch binds `$e` a second time. The rule now also follows `x = e;` when every other write between
that statement and the if is in a catch whose block ends in throw or return (and the if is not in that catch), and
x is never a reference, global or static:

    $e = null; try { f(); } catch (\Exception $e) { throw $e; }  if ($e) {}     reported
    $e = null; try { f(); } catch (\Exception $e) { log($e); }   if ($e) {}     not reported
    $e = 1; $e = 0; if ($e) {}                                                   reported

Both sides miss if-condition.php:291 and TooWideTypehints/bug-13384b.php:23 (types).

Library change proposed: this "value that reaches" belongs in php/values as `php_follow`, so the other value rules
(random_int, duplicate keys) get it; the rule reads `php_occ`, `php_write`, `php_referenced` and `php_dynamic`, which
are php/values' internals, not its interface.

## phpstan-random-int-parameters: the translation follows PHPStan

The 7 translation-only keys, random-int.php:11, 12, 15, 16, 19, 20 and 31, are in PHPStan's recording (together
with 8 and 9, which both sides report). The original compares the two arguments' values only, so a nested
`random_int(a, b)` (PHPStan's int<a, b>) is unknown to it: `random_int(0, random_int(-10, 10))` is reported by
PHPStan, because $max may be -10.

## psalm-forbidden-code: neither side alone

- samples-php, 15 translation-only keys: every one is under a `@psalm-suppress ForbiddenCode` that Psalm honours (on
  the method's or function's docblock, as aphiria UnixLikeDriver.php:34 and 37 and phpstreamserver functions.php:115
  to 131, or on the statement, as cluster functions.php:38). The rule declares `suppress psalm "ForbiddenCode"` as
  the JSON does, but `rerail-engine run-policies` returns its diagnostics with no suppression applied, so the suite
  counts them (the python pack's bandit translations show the same for `# nosec`). Original right; an engine gap.
- fixtures, 1 translation-only key: LowerCaseConstantUnitTest.1.inc.php:79, `var_dump(MyClass::TRUE);`, in the file
  with syntax errors above. The product skips the file; Psalm does not: StatementsProvider::parseStatements reports
  each parser error as ParseError and returns the statements PHP-Parser recovered, which are analysed. PHP-Parser's
  errors here (a reserved class name, a late namespace) leave the tree whole, so Psalm reports the var_dump
  [INFERENCE: read from Psalm's source; Psalm is not installed on the boxes]. Translation right, so the PHPStan
  guard is not applied here.

## Engine gaps

1. `run-policies` applies no `suppress` line (psalm here, nosec in the python pack): it should apply or mark them
   as the product does.
2. `run-policies` reports in files with syntax errors, which the product skips. PHPStan stops at a parse error and
   Psalm does not, so the skip stays a guard in the PHPStan rules (`php_parse_error`), not one for every rule.
