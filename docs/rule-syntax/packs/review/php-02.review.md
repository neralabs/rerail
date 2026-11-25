# Review: php batch 02

Branch reviewed: `tbelanger/pack-translations-php-02` (4772a0e1a3). The tools are the pinned releases: PHPStan 2.2.16
(the packagist phar, built from phpstan-src 08e3736efd) and Psalm 6.19.1, run on PHP 8.5.5 (static-php-cli; the pack's
runtime is 8.5.11). PHPStan ran at level max, the generator's setting, without bleedingEdge unless a finding says so;
Psalm ran with `psalm --init`'s settings (errorLevel 1, findUnusedCode). The translation side is
`rerail-engine run-policies` with the branch's `docs/rule-syntax/lib`. Everything ran on pt-w01, in `/tmp/pr02` (case
files in `/tmp/pr02/c/*`, runner `/tmp/pr02/run.sh`), with all five rules run on every case file on both sides. Each
finding is `file:line code`. "Tool" means the PHPStan or Psalm output; "translation" means the `.rerail`.

`ptcheck --ref tbelanger/pack-translations-php-02 -- --pack php --rules <the five>` (exit 0): if-always-false and
missing-function-return-type `same`; duplicate-key, random-int-parameters and forbidden-code `different`, all
translation-only (11, 7, and 1 + 15), as `php-02.result.json` records. Its `problems` lists if-always-false and
missing-function-return-type as "reports the same findings as its original now; remove it from different": both are
still in `docs/rule-syntax/packs/php/equivalence.json` as "recorded by the suite, not yet explained", and the
coordinator should remove them there. The corpora barely exercise four of the rules (6, 7, 2 and 16 fixture
findings). So the review rests on 31 case files of its own, and the upstream fixtures named below. Snippets below
are condensed. Their `file:line` comments are the lines in the case file, where each statement is on its own line.

| rule | verdict |
|---|---|
| phpstan-array-duplicate-key | defects 1, 2, 3 (spreads are bleedingEdge-only; line of a spread key), 4 (anonymous class `self::`) |
| phpstan-if-always-false | defects 1, 2, 5 (catch left by `continue`, or `finally`), 4 |
| phpstan-missing-function-return-type | defect 2; misses (doc comment choice, invalid `@return`) |
| phpstan-random-int-parameters | defects 1, 2, 7 (named arguments) |
| psalm-forbidden-code | defect 1; minor translation-only cases |
| result file | duplicate-key's `right` holds only with bleedingEdge; forbidden-code's fixture key is not confirmed (original right) |

The first two problems affect four rules each, so they come first.

## Shared problems: unreachable code (four rules) and files PHPStan rejects (the four PHPStan rules)

### 1. Statements PHPStan and Psalm never analyse are reported

PHPStan does not run rules on statements after a statement that always ends (return, throw, exit/die, continue, a
call to a `never` function, an infinite loop, an if whose two branches both return). Psalm did not report ForbiddenCode
in any of these places either. The translations have no notion of reachability, so array.duplicateKey,
if.alwaysFalse, argument.type and ForbiddenCode all report there. missingType.return is not affected: PHPStan still
reports functions declared after a `return` (ur.php:33).

```php
function ur1($x): void { throw new \Exception(); var_dump($x); }   // ur.php:4  ForbiddenCode: translation only
function ur2($x): void { exit(1); var_dump($x); random_int(5, 1); } // :8 ForbiddenCode, :9 argument.type: translation only
function stop(): never { exit(1); }
function ur3($x): array {
    stop();
    if (false) {}                    // ur.php:14 if.alwaysFalse: translation only
    return ['a' => 1, 'a' => 2];     // ur.php:15 array.duplicateKey: translation only
}
function ur4(array $xs): void { foreach ($xs as $x) { continue; if (0) {} } }   // :20 translation only
function ur5($x): void { if ($x) { return; } else { return; } random_int(5, 1); } // :25 translation only
function ur6($x): void { while (true) {} if (false) {} }                          // :29 translation only
function ur8($x): void { die; `ls`; }                                             // :37 ForbiddenCode: translation only
```

PHPStan reports none of these, and Psalm reports neither ForbiddenCode. The same happens for `return; if (false) {}`
(if1.php:41), `return [...]; $z = ['b' => 1, 'b' => 2];` (dk2.php:9) and `return; var_dump($x);` (fc1.php:26). No
divergence records it. The most likely place for it in real code is debugging code left after a `return`.

### 2. php/php-02's `php_parse_error` misses files that PHPStan rejects

`php_parse_error(d)` is an `ERROR` node. tree-sitter-php recovers from some syntax errors with a zero-width MISSING
token and no ERROR node, and it accepts what PHP-Parser rejects at parse time. PHPStan reports `phpstan.parse` for all
of these files and nothing else. The four PHPStan translations report them as usual:

```php
<?php                                  // s-pe1/pe1.php
$a = 1
function pe1() {}                      // :3 missingType.return: translation only (PHPStan: "Syntax error, unexpected T_FUNCTION")
```
```php
<?php                                  // s-pe6/pe6.php
class Pe6 { public public $x; }        // PHPStan: "Multiple access type modifiers are not allowed"
function pe6() {}                      // :3 missingType.return: translation only
```
```php
<?php                                  // s-pe7/pe7.php
abstract final class Pe7 {}            // PHPStan: "Cannot use the final modifier on an abstract class"
function pe7() {}                      // :3 missingType.return: translation only
```

The other three report too. With `$a = 1` (no `;`) before `function pe9(): array { if (false) {} random_int(5, 1);
return ['a' => 1, 'a' => 2]; }` (s-pe9/pe9.php), the translation reports if.alwaysFalse at 4, argument.type at 5 and
array.duplicateKey at 6. PHPStan reports only `phpstan.parse` at 3.

The missing `;` is the case that matters, because an agent's file is often in the middle of an edit. A file with a
real ERROR node (`$a = 1` before `var_dump($a);`, pp1.php) is skipped correctly. Valid PHP 8.4/8.5 syntax does not
produce a false ERROR: property hooks, `new X()->m()`, asymmetric visibility and `|>` all parse, and the files are
reported as PHPStan reports them (pe2–pe5).

Not a translation defect, but worth knowing: when any analysed file has a parse error, PHPStan 2.2.16 reports only the
parse errors in that whole run, and none of the other files' findings (seen with the c4 directory before
pe1/pe6/pe7 were moved out). Per-file translations cannot reproduce that.

## phpstan-array-duplicate-key: defects

### 3. Spread duplicates are bleedingEdge findings; without bleedingEdge PHPStan reports none

PHPStan 2.2.16 counts the keys of an unpacked constant array only when bleedingEdge is on (observed). The likely
mechanism, from reading the source: without bleedingEdge, `ArrayUnpackingHelper::getKeyTypes` gets a constant array
that is not sealed (`isUnsealed()` is not `no`) and returns null, so every key after a spread is unknown [INFERENCE].
Upstream's `RuleTestCase` always includes `conf/bleedingEdge.neon`
(src/Testing/RuleTestCase.php:344). So the test expectations that the pack's fixtures record (bug-15247.php:19/34,
bug-15247-php81.php:14/19) are bleedingEdge results. The pack's `level0` configuration, and every level without
bleedingEdge, reports none of them:

```php
function sp1(): array { return [...['a' => 1], 'a' => 2]; }     // sp.php:2: translation; PHPStan only with bleedingEdge
function sp2(): array { return ['a' => 1, ...['a' => 2]]; }     // sp.php:3: same (this is bug-15247-php81.php:14's code)
```

`phpstan analyse -l 0` (or `-l max`) on upstream's own `bug-15247.php` reports only line 51 (two literal `'a'` keys).
With bleedingEdge it reports 19, 34, 51 and 66, as the test expects. The translation follows the test expectations,
so with the pack's configuration it reports every spread-derived duplicate falsely. Either the translation stops
counting spread keys, or the pack records that its PHPStan reference is bleedingEdge.

Even under bleedingEdge, two cases differ:
- **Line**: PHPStan reports a key that comes from a spread at the line of the `...` item (`$item->getStartLine()`).
  `php_at` gives the inner element's line instead. With bleedingEdge, PHPStan reports sp.php:6 and the translation
  sp.php:7 (sp.php:23 against :25, dk1.php:4 against :5):
  ```php
  return [
      ...[             // line 6: PHPStan (bleedingEdge)
          'a' => 1,    // line 7: translation
      ],
      'a' => 2,
  ];
  ```
- **A spread of a variable** holding a literal: `$x = ['a' => 1]; $y = [...$x, 'a' => 2];` (dk2.php:4). PHPStan reports
  it with bleedingEdge; the translation does not (`php_spread_item` takes only literals).

### 4. `self::X` inside an anonymous class reads the outer class's constant

`php_class_const` (php/values) takes `nearest(n, k, "class_declaration")`. An anonymous class is not a
`class_declaration`, so its `self::A` resolves to the enclosing named class's `A`:

```php
class AnonOuter {
    const A = 'a';
    public function f(): object {
        return new class {
            const A = 'b';
            public function g(): array { return [self::A => 1, 'a' => 2]; }  // fl.php:14: translation only
        };
    }
}
```

PHPStan reads `self::A` as `'b'` and reports nothing. The same lookup gives if.alwaysFalse a false finding: in
`class IfDbg { const DEBUG = false; … new class { function h() { if (self::DEBUG) {} } } }` (fl.php:23), PHPStan reports
an undefined constant, not if.alwaysFalse.

### Misses (PHPStan reports, the translation does not)

None of these is a false finding. Only the first is documented, and only in php/arrays, not in the rule's header:
- `[1e3 => 'x', 1000 => 'y']` (dk1.php:15): exponent floats are not cast.
- global constants: `const GK = 'g'; ['g' => 1, GK => 2]` (dk2.php:25).
- a key with two possible values: `$k = $c ? 'a' : 'b'; ['a' => 1, 'b' => 2, $k => 3]` reports both 'a' and 'b'
  (dk3.php:22).
- an enum's constants: `enum En: string { const K = 'k'; … [self::K => 1, 'k' => 2] }` (fl.php:29).

Checked and agreeing: octal `010`/`0o17`, hex, binary and `1_000` keys (the php/values header's "017 reads as 17" gap
is stale: `[010 => 'x', 10 => 'y']` is reported by neither), escapes in double-quoted keys, `null`/`''`, `true`/`1.9`,
negative keys with implicit ones after them, class constants (`self::`, `static::` in a final class, `static::` in a
non-final one not followed), the one-write variable, a parameter default, a file-scope variable read in a function, a
write after the array, keys in class constants, property defaults, parameter defaults, `static` initialisers,
attribute arguments and global `const`. Destructuring (`['a' => $x, 'a' => $y] = $arr`, `list(…)`,
`foreach (… as [...])`) is reported by neither.

## phpstan-if-always-false: defects

### 5. `reaching` treats a write in a catch that ends by throwing as dead, but it reaches the if

`dead_write` drops a write that is in a catch whose last statement is `throw` or `return`. The catch can still be left
by another path, and a `finally` runs after the throw:

```php
function if1(array $xs): void {
    $e = null;
    foreach ($xs as $x) {
        try { g($x); }
        catch (\Exception $e) {
            if ($x) { continue; }   // leaves the catch with $e set
            throw $e;
        }
    }
    if ($e) { echo 'failed'; }      // if1.php:14: translation only; PHPStan: $e is Exception|null
}
```
```php
function fin1(): void {
    $e = null;
    try { fin2(); }
    catch (\Exception $e) { throw $e; }
    finally {
        if ($e) { echo 'failed'; }  // fin.php:9: translation only
    }
}
```

The catch must end without a `continue`/`break`/`goto` that leaves it, and the if must not be in that try's `finally`.

### 6. `self::DEBUG` in an anonymous class (see 4)

### Misses

- **A trait used by a class**: `trait IfT { function t() { if (false) {} } } class IfC { use IfT; }` (if2.php:4).
  PHPStan reports it, because the condition is false in every class that uses the trait. An unused trait is reported
  by neither (if2.php:12). divergences.json records the trait exclusion only for phpstan-if-always-true
  (if-condition-in-trait.php:19, `not-comparable`), with nothing for if-always-false. Its reason, "the truth can
  differ per using class", does not hold when the truth is the same in all of them.
- `and`, `or` and `xor` written as keywords: `if (false and g(1))`, `if (false or 0)`, `if (false xor false)`
  (if1.php:20–22). PHPStan reports them; only `&&`/`||` are other rules'.
- `if (1 - 1)`, `if (+0)`, `if (null ?? false)`, `if ($x = 0)` (if1.php:23, 25, 28; fin.php:16).
- A `/** @var … */` above the if hides it even when the condition is not the variable it names:
  `/** @var int $y */ if (false) {}` and `/** @var int */ if (0) {}` (if1.php:35, 37) are reported by PHPStan.
- A closure's `use ($z)`: `$z = 0; function () use ($z) { if ($z) {} }` (if2.php:17).
- `$y = 0; $f = fn() => $y = 1; if ($y) {}` (fin.php:20): an assignment in an arrow function does not write the
  outer `$y`, but it is counted as a write.

Agreeing: `false`, `0`, `''`, `[]`, `array()`, `"\x30"`, `0.0`, `'0.0'`, constants such as `PHP_EOL`, `M_PI` and
`PHP_INT_SIZE === 0` (none reported), ifs nested in dead branches and in `else` after `elseif`, alternative syntax
`if (…):`, a write in a loop after the if, two writes in sequence (`$w = 1; $w = 0; if ($w)`), the
composer InstallationManager.php:251 case that the header's try/catch example comes from (reported by both).

## phpstan-missing-function-return-type: misses only

No false findings beyond 2. PHP-Parser keeps the last doc comment before a function. PHPStan also ignores an `@return`
that is in the middle of prose or whose type does not parse (observed below). `returns_doc` instead takes any `/** */`
in the run of comments before the function, and any `@return word` anywhere in it:

```php
/** @return int */
/** Another doc */
function mr1() { return 1; }        // mr1.php:4: PHPStan only (the second doc wins)
/** Never forget the @return tag. */
function mr2() { return 1; }        // mr1.php:6: PHPStan only (@return in prose)
/** @return {int} */
function mr3() { return 1; }        // mr1.php:8: PHPStan only (invalid type)
/** @return int| */
function mr14() { return 1; }       // mr1.php:38: PHPStan only (invalid type)
```

Agreeing: `@return mixed`, `@phpstan-return`, `@psalm-return`, an empty `@return`, a `//` comment between the doc and
the function, `/* @return */` (not a doc), a doc before or after an attribute, `@returns`, by-reference functions,
functions nested in methods, functions declared in an `if (!function_exists(…))`, namespaced functions, a function
after a top-level `return`, closures (neither), `function f(): mixed`, generics in `@return array<int, string>`.

## phpstan-random-int-parameters: defects

### 7. Named arguments: PHPStan reads them by position

`RandomIntParametersRule` takes `$node->getArgs()[0]` and `[1]`, in the order they are written. The translation maps
named arguments to `min`/`max`, so the two disagree both ways:

```php
random_int(max: 1, min: 5);   // ri1.php:11: translation only (PHPStan reads min=1, max=5)
random_int(max: 5, min: 1);   // ri1.php:12: PHPStan only ("$min (5) … $max (1)")
random_int(min: 5, max: 1);   // ri1.php:21: both
```

PHPStan is the one that is wrong here: line 11 throws at run time and line 12 does not. So line 11 should not be
dropped. The divergence should be recorded (upstream-wrong), which also covers line 12.

### Misses

The header documents ranges from types (`int<5, max>`, a parameter's `int`). These are not documented. PHPStan converts
each argument with `toInteger()` and folds constant expressions:
- `random_int('10', 5)`, `random_int(true, 0)`, `random_int(2.5, 1)`, `random_int((int) '9', 1)`,
  `random_int(intval('9'), 1)` (ri1.php:5–7, rr2.php:16–17).
- `random_int(10 - 1, 5)`, `random_int(+5, 1)` (ri1.php:8, 13).
- a variable holding a `random_int` range: `$b = random_int($a, 10); random_int($b, 4);` with `$a = 5`, and
  `$c = random_int(1, 3); random_int(2, $c);` (rr2.php:5, 7).
- `random_int(count($a), -1)` (ri1.php:10): `count()` is `int<0, max>`, certainly above -1. This one is from types.

A namespace's own `random_int` is reported as the global one (`namespace Ri; function random_int(…) {…} random_int(5, 1);`,
rr.php:5: translation only). php/names documents this ("not followed: rare").

Also note that the translation reports "maybe" overlaps (ri1.php:16–17). PHPStan reports these only with
reportMaybes, which is on at level 7 and above and in the test. At level 5, where the rule is registered, PHPStan does
not report the same code in random-int.php (12 and 31, below). The generator reads level max, so the translation is
consistent with it, but it is a configuration choice.

Agreeing: `010` (8, not 10: no finding), hex and `1_000`, `-0`, negative pairs, equal constants, a third argument,
`\random_int`, a one-write variable, nested calls in both arguments (upstream's random-int.php cases), spreads.

## psalm-forbidden-code: no defect of its own (1 applies)

Agreeing on lines: `var_dump`, `\VAR_DUMP`, `use function var_dump as vd; vd()`, `shell_exec`, backticks, `var_dump()`
with no arguments, `var_dump(...[$x])`, and calls in `if (false)`, `?:`, `??`, `echo`, arrow functions and closures.
`run-policies` prints no column. The translation anchors at the call's or backtick's node start, which is Psalm's
column in every case here (`\VAR_DUMP` at the backslash, `` $o = `ls -la` `` at 10), and the suite's line+column
match agrees on all 52 + 80 shared keys. Reported by neither: first-class callables (`var_dump(...)`, `shell_exec(...)`),
`call_user_func('var_dump', …)`, `call_user_func_array('shell_exec', …)`, `$h = 'var_dump'; $h($x)`, `exec`, `system`,
`passthru`, `eval` and `print_r`.

The two kept idioms are Psalm-only, as intended: `ob_start(); var_dump($x); $s = ob_get_clean();` (fc1.php:30), and
`var_dump` in the body of a `dump`/`dd` method or function (fc2.php:13, 16). divergences.json records the ob_start
idiom (7 entries, Doctrine's `Debug::dump()` among them, which is also an ob_start capture). It has no entry for the
`dump`/`dd` body rule that the header attributes to it. That exclusion matches the JSON original, but its
upstream-wrong reason is not recorded.

Smaller differences, all translation-only:
- a trait that no class uses: `trait UnusedTraitDump { function t($x) { var_dump($x); } }` (tr.php:3). Psalm reports
  nothing there.
- a file whose syntax error makes Psalm drop the function: `function pp2($x) { var_dump($x); … } function pp2b( {`
  (pp2.php:3). Psalm reports only ParseError. Its recovery is its own. php/php-02 notes that Psalm analyses what its
  parser recovers, but it does not always recover a function; see also the fixture key under "Result file" below.
- a namespace's own `var_dump`: `namespace Nv; function var_dump(mixed $x): void {} … var_dump(1);` (nv.php:5).
  php/names documents this as a gap, and it is the same for random_int.

## Result file (`php-02.result.json`): the `right` sides against the tools

Each `different` entry's keys were run through the tool alone (pt-w01, `/tmp/pr02/fx2`, `fx3`).

- **phpstan-array-duplicate-key, right = translation (11 keys).** This holds only for PHPStan with bleedingEdge, the
  configuration of upstream's tests, which the recording reproduces. With bleedingEdge, PHPStan reports all 11.
  Without it (`-l 0`, `-l max`), 7 of them: bug-15244.php:49 and duplicate-keys.php:32, 63, 67, 69, 76 and 105. It
  does not report the four spread keys (bug-15247-php81.php:14 and 19, bug-15247.php:19 and 34). See 3.
- **phpstan-random-int-parameters, right = translation (7 keys).** Confirmed at level max: PHPStan reports
  random-int.php:11, 12, 15, 16, 19, 20 and 31, as well as 8 and 9. At level 5, the rule's own level (reportMaybes
  off), it reports only 8, 9, 11 and 15.
- **psalm-forbidden-code, right = neither.**
  - The 15 samples-php keys (original right): confirmed on aphiria `UnixLikeDriver.php` (34 and 37, under the method's
    `@psalm-suppress ForbiddenCode` at 25) and cluster `functions.php` (38 and 72, statement-level suppressions at 37
    and 71). Psalm reports no ForbiddenCode in either file, and `run-policies` applies no suppression (the known gap).
  - The fixtures key `phpcs/Generic/PHP/LowerCaseConstantUnitTest.1.inc.php:79` (translation right): **not
    confirmed. The original is right.** The NOTES mark it `[INFERENCE: … Psalm is not installed on the boxes]`. Psalm
    6.19.1 on that file alone reports ParseError at 56 ("Namespace declaration statement has to be the very first
    statement") and class-level issues (DuplicateClass at 73, UnusedClass, …), but no ForbiddenCode at 79. It reports
    no issue at all from the file's top-level statements after the namespace either: there is no
    UndefinedGlobalVariable for `$f` at 70. A reduced file (`$x = 1; namespace Foo; var_dump(1);`, s-ns/n.php) is
    reported, so Psalm does sometimes analyse what it recovers, but not here.
- if-always-false and missing-function-return-type are `same` in the result file. As said at the top, equivalence.json
  still lists them as unexplained differences.

The NOTES describe the `reaching` rule as following `x = e;` "when every other write between that statement and the
if is in a catch whose block ends in throw or return". Finding 5 is where that does not hold. The NOTES' composer
InstallationManager.php:251 case is still reported by both sides.
