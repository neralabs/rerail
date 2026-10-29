# Rerail rule pack: PHP, candidates

The ports in `rules/` here are still being evaluated: scored against the tools' expected findings and reported, never
gated and never shipped. This file is not published; the pack's [README](../README.md) covers the rules it ships.

## Candidates

The title is what `pack.json` lists for a candidate once it ships; its group is the tool the title names. The reason
is why it stays a candidate.

| Candidate | Title | Why |
|---|---|---|
| `phpcs-generic-php-deprecated-functions` | PHP_CodeSniffer Generic.PHP.DeprecatedFunctions: Deprecated functions | the engine reports `source-incomplete` on LowerCaseConstant's deliberately broken test files (`.1.inc`, `.2.inc`), which every ported sniff is judged on |
| `phpcs-generic-php-forbidden-functions` | PHP_CodeSniffer Generic.PHP.ForbiddenFunctions: Forbidden functions | as DeprecatedFunctions |
| `phpcs-generic-php-no-silenced-errors` | PHP_CodeSniffer Generic.PHP.NoSilencedErrors: No silenced errors | its regex names `file_exists` (Rerail's own exemption), a word in upstream's tests but not its code: fitted to the tests, and no rename variant tests it |
| `phpcs-squiz-php-disallow-size-functions-in-loops` | PHP_CodeSniffer Squiz.PHP.DisallowSizeFunctionsInLoops: Size functions in loops | it names `array_push`, from Rerail's own exemption, a word in upstream's tests but not its code: fitted to the tests, and no rename variant tests it |
| `phpstan-variable-undefined` | PHPStan variable.undefined: Undefined variable | 13 files differ; it needs PHPStan's types |
| `phpstan-argument-sprintf` | PHPStan argument.sprintf: sprintf() argument count | a format held in a variable with two possible values needs PHPStan's types |
| `phpstan-array-duplicate-key` | PHPStan array.duplicateKey: Duplicate array key | implicit keys, class constants and PHPDoc-typed keys need PHPStan's types |
| `phpstan-random-int-parameters` | PHPStan argument.type: random_int() parameters | integer ranges need PHPStan's types; it fires where PHPStan does not |
| `phpstan-if-always-true` | PHPStan if.alwaysTrue: If condition always true | type narrowing needs PHPStan's types |
| `phpstan-if-always-false` | PHPStan if.alwaysFalse: If condition always false | as if.alwaysTrue |
| `psalm-unused-variable` | Psalm UnusedVariable: Unused variable | 6 of Psalm's cases differ (references and `global`, a call the engine cannot resolve, and one where Psalm reports another line), which needs engine work on PHP scope and usage (`scope/usage/php.rs`) |

Two candidates carried an example pair while they shipped. `test/engine/catalog/candidate-examples.test.ts` still
checks each pair below through the engine, and the pairs go back into `pack.json` with their rules:

### `phpcs-squiz-php-disallow-size-functions-in-loops`

Caught (`app/print.php`):

```php
<?php

function printAll(array $items): void
{
    for ($i = 0; $i < count($items); $i++) {
        echo $items[$i];
    }
}
```

Passes (`app/drain.php`):

```php
<?php

function drain(array $queue): void
{
    while (count($queue) > 0) {
        run(array_shift($queue));
    }
}
```

### `phpstan-argument-sprintf`

Caught (`app/greet.php`):

```php
<?php

function greet(string $name): string
{
    return sprintf('Hello %s, you have %d messages', $name);
}
```

Passes (`app/greet.php`):

```php
<?php

function greet(string $name, int $count): string
{
    return sprintf('Hello %s, you have %d messages', $name, $count);
}
```

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `phpcs-generic-php-deprecated-functions`

Port of PHP_CodeSniffer 4.0.4 Generic.PHP.DeprecatedFunctions run on PHP 8.5.11: a call PHP resolves to one of the 67 internal functions ReflectionFunction::isDeprecated() reports for the extensions PHP_CodeSniffer's PHP loads. rerail/packs/php/upstream/phpcs records the list.

### `phpcs-generic-php-forbidden-functions`

Port of PHP_CodeSniffer 4.0.4 Generic.PHP.ForbiddenFunctions with its default list (sizeof, delete): a call PHP resolves to either function.

### `phpcs-generic-php-no-silenced-errors`

Port of PHP_CodeSniffer 4.0.4 Generic.PHP.NoSilencedErrors (every @ operator). Divergence: not reported where the @ is part of an idiom that needs it, because removing it changes behaviour:

- a silenced mkdir(), rmdir() or unlink() of a path that the right operand of a `&&`, `||`, `and` or `or` re-checks with is_dir(), is_file() or file_exists(), where the @ call is that operator's left operand or reaches it only through `!`, parentheses and earlier operands of the same chain (the race-safe `!@mkdir($dir) && !is_dir($dir)`, where the re-check decides whether a warning was a failure);
- a silenced trigger_error() or user_error() whose second argument, or argument named error_level, is E_USER_DEPRECATED (Symfony's deprecation convention, silenced so only error handlers that opt in log it).

### `phpcs-squiz-php-disallow-size-functions-in-loops`

Port of PHP_CodeSniffer 4.0.4 Squiz.PHP.DisallowSizeFunctionsInLoops: count(), sizeof() and strlen() in a for, while or do-while condition. Divergence: not reported when the loop changes the value it measures, because hoisting then changes what the loop does. The loop changes it when it assigns or appends to it (=, []=, +=, .=), unsets one of its elements, passes it to array_push, array_unshift, array_pop, array_shift or array_splice, or, for a $this property, calls a method on $this.

### `phpstan-argument-sprintf`

Port of PHPStan 2.2.16 argument.sprintf (PrintfParametersRule): a sprintf() call whose format is known statically and asks for more or fewer values than passed, or a format PHP refuses.

### `phpstan-array-duplicate-key`

Port of PHPStan 2.2.16 array.duplicateKey (DuplicateKeysInLiteralArraysRule). Reported, as PHPStan does, at the first explicitly keyed element whose key PHP coerces to the same key as a later explicit key. Arrays spanning more than 256 lines are not checked: comparing every pair of keys of a larger literal (generated data tables) exceeds the per-file AST work budget, which would fail every policy on the file.

### `phpstan-if-always-false`

Port of PHPStan 2.2.16 if.alwaysFalse (IfConstantConditionRule): an if condition PHP's constant evaluation (variables followed through their one reaching binding) finds always falsy. Not reported: comparisons, negations, ternaries, isset and empty (PHPStan leaves those to other rules), a condition in a trait, or an if preceded by an inline @var doc comment (PHPStan reads the variable as the PHPDoc type). Divergence: an if whose block holds only class, interface, trait, enum or function declarations and class_alias calls (with comments), such as Symfony's `if (false) { class Router {} }` deprecation stubs, which Composer's class map and IDEs read and which must not be deleted, is not reported.

### `phpstan-if-always-true`

Port of PHPStan 2.2.16 if.alwaysTrue (IfConstantConditionRule): same evaluation and exclusions as if.alwaysFalse (constant evaluation following variables through their one reaching binding; skips comparisons, negations, ternaries, isset, empty, conditions in traits, and ifs preceded by an inline @var doc comment).

### `phpstan-random-int-parameters`

Port of PHPStan 2.2.16 RandomIntParametersRule (identifier argument.type): only fires when $min and $max, bound as PHP binds them (named arguments included), are constants with $min > $max.

### `phpstan-variable-undefined`

Port of PHPStan 2.2.16 variable.undefined (DefinedVariableRule): a variable read that no path defines ("Undefined variable: $x"). PHPStan's "might not be defined" reports (defined on some paths) are not claimed.

### `psalm-unused-variable`

Port of Psalm 6.19.1 UnusedVariable (--find-unused-variables): a variable whose value no read uses, bound by an assignment statement, a compound assignment or increment, a closure's `use` clause, `global`, or a `foreach` key or destructured element. Psalm reports a `foreach` value as UnusedForeachValue and a parameter as UnusedParam (not this rule), and skips names starting with `$_`.
