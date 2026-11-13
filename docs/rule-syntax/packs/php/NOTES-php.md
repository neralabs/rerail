# php: 15 rules, hand-written (PHP_CodeSniffer 4.0.4, PHPStan 2.2.16, Psalm 6.19.1)

The 15 rules have 428 lines (plus a 75-line generated table), against 587 in the machine translation. They share
472 lines of library in five new files:

| file | what |
|---|---|
| `lib/php/names.rr` | which global function a call names (as written, or with `use function` followed), positional and named arguments, `$this`, `&&`/`and` logicals |
| `lib/php/phpdoc.rr` | the `/** */` comment before a declaration or statement, and its `@return` / `@var` types (and the `@phpstan-` / `@psalm-` forms) |
| `lib/php/values.rr` | constant values without types: literals, `self::C`, a variable with one write that runs first; truthiness, `===`; the `if (c)` conditions PHPStan's if.always* rules report |
| `lib/php/arrays.rr` | the key of each element of an array literal, as PHP casts and numbers them, literal spreads unpacked |
| `lib/php/format.rr` | printf and scanf formats as PHP 8 parses them: values needed, variables assigned, invalid formats |

## Scores

`rrlab score php ID --all` (every fixture of the tool's recording), matched on (path, line, code); the phpcs and
psalm rules also match on columns, all exact. "ex" counts findings that `divergences.json` already explains
(`rerail-unknown`: PHPStan knows them from types).

| rule | lines | TP/FP/FN | deviations |
|---|---|---|---|
| phpcs-generic-code-analysis-empty-statement | 34 | 42/0/0 | |
| phpcs-generic-php-backtick-operator | 16 | 2/0/0 | |
| phpcs-generic-php-deprecated-functions | 20 | 1/0/0 | table in `.facts.rr` (gen/check scripts) |
| phpcs-generic-php-forbidden-functions | 19 | 11/0/0 | |
| phpcs-generic-php-lower-case-constant | 18 | 41/0/0 | |
| phpcs-generic-php-no-silenced-errors | 48 | 4/0/0 | keeps the pack's two exceptions (a) |
| phpcs-squiz-php-disallow-size-functions-in-loops | 55 | 15/0/0 | keeps the pack's two exceptions (a) |
| phpcs-squiz-php-eval | 16 | 2/0/0 | |
| phpstan-argument-sprintf | 31 | 21/0/0, 2 ex | (b) |
| phpstan-argument-sscanf | 29 | 3/0/0 | (b) |
| phpstan-array-duplicate-key | 27 | 17/0/0, 10 ex | numbers implicit keys and spreads (c) |
| phpstan-if-always-false | 30 | 7/0/0, 2 ex | keeps the pack's stub exception (a); (d) |
| phpstan-missing-function-return-type | 22 | 10/0/0 | |
| phpstan-random-int-parameters | 31 | 9/0/0, 1 ex | compares ranges of nested random_int (e) |
| psalm-forbidden-code | 32 | 5/0/0 | keeps the pack's two var_dump exceptions (a) |

All 15 are perfect on their fixtures, the explained points aside. PHP itself is not installed here (only the
pinned 8.5.11 the recipes build), so no counterexample was run on a tool; the ad-hoc cases below were run with
`rrlab run`.

## Review fixes

The review's cases (in the coordinator's scratch, `rev-php-blade/`) found these; each is fixed, and the 15 rules
score as before:
- **Interpolated strings** had a value: `"x $y"`, `if ("$a")`, and duplicate keys `['$a' => 1, "$a" => 2]`,
  `["a$x" => 1, "a$x" => 2]`. An encapsed string or heredoc with a part other than text or an escape now has none.
- **Single-quoted strings** were unescaped as double-quoted ones (core's `str`): `'a\n'` read as a newline, so
  `['a\n' => 1, "a\n" => 2]` was a duplicate. php/values now unescapes only `\\` and `\'` there.
- **no-silenced-errors** reported `!is_dir($d) && false === @mkdir($d) && !is_dir($d)`; a `==`, `===` or `!==`
  comparison with `false` now passes the climb to the `&&`, as `!` does.
- **size-in-loops** missed writes to `$a['k']['j']` and `[$a, $b] = f()`; `target` follows subscripts down and
  takes list patterns.
- **Array keys after an uncast key**: in `[1e3 => 'a', 'b', 0 => 'c']` 'b' was numbered 0 (PHP: 1001). A key
  with a value php/arrays does not cast now blocks the numbering after it, as an unknown key does; `-0.5` has a
  value (key 0).
- **Argnums above 32**: `sprintf('%40$s', 1)` gave a wrong count; such a format now gives no `printf_needs`.
- Idioms: the duplicate-key message prints `1` or `'a'`, not the internal `i:1`/`s:a`; `php_identical(e, r)` is
  keyed on the `===` expression; format relations are computed only for format arguments (`php_format_arg`: the
  first of sprintf/printf, the second of sscanf), not every string.
- Cheap checks: `new Foo($x)` now counts as a possible write of `$x` (a constructor may take it by reference);
  grouped imports `use function Foo\{a, b}` and `use Foo\{function a, Bar}` are read (php_use_path joins the
  prefix), so `a()` there is not the global a. A nowdoc still has no value (its closing-indent rule is not modelled).

## Deviations

(a) **Exceptions the pack declares.** Five rules keep exceptions the JSON adds and `divergences.json` justifies
on real code as `upstream-wrong` (Symfony and Laravel samples): `@trigger_error(…, E_USER_DEPRECATED)` and
`!@mkdir($d) && !is_dir($d)`; a size call whose operand the loop writes, or `$this->x` in a loop that calls a
method of `$this`; an `if (false)` holding only declarations and class_alias (a stub); var_dump between
`ob_start()` and `ob_get_clean()`, and inside a function named dump or dd. The tools report all of them; the
rules follow the pack, and say so in their opening comments.

(b) **Formats.** Read as PHP 8 parses them, which is also what PHPStan's PrintfHelper does: `%5%` takes a value
(PHP fetches one), `%*d` takes two, `%2$s` needs two. Widths and argnums of ten digits or more are taken as invalid
(PHP's limit is INT_MAX: `%1234567890d` is valid in PHP, invalid here). Argnums are looked for up to 32; a format with a larger one is left unknown. scanf's
XPG positional `%1$d` counts as one assignment, not as its number. The 2 explained FNs (printf.php:52, 54) are a
format variable with two possible values, which PHPStan tracks as a union.

(c) **Array keys.** The JSON compares explicit keys' text only; PHPStan numbers keyless elements and unpacks
spreads. `lib/php/arrays.rr` does both (PHP 8.3 numbering, after negative keys too), and php/values follows
`self::EQ` and `$key2 = 'key'`. That turns 11 of the 21 lines `divergences.json` lists as `rerail-unknown` into TPs
(duplicate-keys.php:32, 63, 67, 69, 76, 105; bug-15244.php:49; bug-15247.php:19, 34; bug-15247-php81.php:14, 19).
Still unknown: keys past nine digits (PHP_INT_MAX numbering, bug-15244.php:38, bug-15247.php:66), `$idx++` keys
and PHPDoc unions (`@param 'foo'|'bar' $key`). The JSON's `layout.span.max 256` (arrays up to 256 elements) is an
engine budget, not PHPStan's; it is left out.

(d) **Constant conditions.** No types: a variable's value is followed only to its one write, in an enclosing
block before the use; any call argument counts as a write unless the callee is a builtin known to take values
(`php_byval_fn`, a list), since PHPStan knows by-reference parameters (`preg_match($p, $s, $m)`) and this does not.
The 2 explained FNs are a `self` parameter and narrowing from an enclosing `if (!$mixed)`.

(e) **random_int.** The JSON compares constants; PHPStan compares integer ranges, and random_int(a, b) has type
int<a, b>. The rule does the same for nested calls, which turns 7 of the 8 `rerail-unknown` lines into TPs
(random-int.php:11, 12, 15, 16, 19, 20, 31). The one left (bug-14468.php:57) needs PHPDoc `int<5, max>`.

Also: phpcs reads tokens, so the phpcs rules match calls as written (`sizeof(…)`, `\sizeof(…)`), not through
`use function` aliases; the PHPStan and Psalm rules resolve them (`php_call`). An unqualified call in a namespace
that defines a function of that name would call the namespace's; that is not followed.

## Library and core changes

None to existing files. Harness notes for rrlab, none blocking:
1. rrlab's `value` reads a legacy octal `017` as 17 (PHP: 15); core's value should normalise it. `0o17` is right.
   Core's `str` also unescapes PHP single-quoted strings as double-quoted ones and leaves octal `"\101"` as `101`;
   php/values works around the first.
2. rrlab maps `(?i)` but not a scoped `(?i:…)` group; the rules avoid it (`@lower` and a set instead).
3. `match_all` is empty in rrlab, so `lib/php/format.rr` reduces a format with `@replace_all` and counts with
   `@count_matches` instead; with match_all a format could be tokenised directly, one row per placeholder.

## Duplicates to merge

None: every relation is `php_`-, `doc_`-, `printf_`- or `scanf_`-prefixed or private to its rule.
