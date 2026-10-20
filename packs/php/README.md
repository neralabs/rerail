# Rerail rule pack: PHP

18 ports of PHP rules: PHP_CodeSniffer 4.0.4 (8 sniffs), PHPStan 2.2.16 (8 rules) and Psalm 6.19.1 (2 issues). A port
starts in `candidates/rules/` and moves to `rules/` and `pack.json` once it passes the V1 rule validation gates
([nera-rule-validation](https://github.com/neralabs/nera-rule-validation)). Every port is a candidate today, so
`pack.json` lists no rules: the pack ships none and is never recommended. Every port is scored against the tool's own
findings on the tool's tests, through the engine's production workspace path, with every
disagreement declared in `divergences.json`. The conformance check (`--check`, run in CI) scores the tests: it gates
the ports in `rules/` and reports the candidates without gating them.

It follows the [pack structure and validation contract](../../docs/pack-format.md): it is checked by
`npx vitest run test/engine/catalog/packs.test.ts -t "pack php"`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): its match definition, and no rules until a port passes |
| `candidates/rules/` | The candidates' code-rule documents (`rerail.policy/v1`), scored but never shipped |
| `conformance.json` | Every port (shipped under `rules/`, or a candidate under `candidates/rules/`): its upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tools and the pinned corpora (laravel, symfony) |
| `divergences.json` | Declared disagreements with the tools |
| `upstream/<tool>/` | The tool's copied tests (`fixtures/`), its recordings (`fixtures.expected.json`, `corpus-<name>.expected.json`) and the recorder (`generate.py`, sharing `upstream/common.py`); Psalm's corpus dependencies are pinned by `upstream/psalm/composer-<name>.lock` |
| `licenses/<tool>/` | The tool's licence and `UPSTREAM.md`, listing every copied file |

## Candidates

The title is what `pack.json` lists for a candidate once it ships; its group is the tool the title names.

| Candidate | Title |
|---|---|
| `phpcs-generic-code-analysis-empty-statement` | PHP_CodeSniffer Generic.CodeAnalysis.EmptyStatement: Empty statement |
| `phpcs-generic-php-backtick-operator` | PHP_CodeSniffer Generic.PHP.BacktickOperator: Backtick operator |
| `phpcs-generic-php-deprecated-functions` | PHP_CodeSniffer Generic.PHP.DeprecatedFunctions: Deprecated functions |
| `phpcs-generic-php-forbidden-functions` | PHP_CodeSniffer Generic.PHP.ForbiddenFunctions: Forbidden functions |
| `phpcs-generic-php-lower-case-constant` | PHP_CodeSniffer Generic.PHP.LowerCaseConstant: Lower-case constant |
| `phpcs-generic-php-no-silenced-errors` | PHP_CodeSniffer Generic.PHP.NoSilencedErrors: No silenced errors |
| `phpcs-squiz-php-disallow-size-functions-in-loops` | PHP_CodeSniffer Squiz.PHP.DisallowSizeFunctionsInLoops: Size functions in loops |
| `phpcs-squiz-php-eval` | PHP_CodeSniffer Squiz.PHP.Eval: Eval |
| `phpstan-variable-undefined` | PHPStan variable.undefined: Undefined variable |
| `phpstan-argument-sprintf` | PHPStan argument.sprintf: sprintf() argument count |
| `phpstan-argument-sscanf` | PHPStan argument.sscanf: sscanf() argument count |
| `phpstan-array-duplicate-key` | PHPStan array.duplicateKey: Duplicate array key |
| `phpstan-random-int-parameters` | PHPStan argument.type: random_int() parameters |
| `phpstan-missing-function-return-type` | PHPStan missingType.return: Missing function return type |
| `phpstan-if-always-true` | PHPStan if.alwaysTrue: If condition always true |
| `phpstan-if-always-false` | PHPStan if.alwaysFalse: If condition always false |
| `psalm-unused-variable` | Psalm UnusedVariable: Unused variable |
| `psalm-forbidden-code` | Psalm ForbiddenCode: Forbidden code |

Three candidates carried an example pair while they shipped. `test/engine/catalog/candidate-examples.test.ts` still
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

### `phpcs-squiz-php-eval`

Caught (`app/render.php`):

```php
<?php

function render(string $template): void
{
    eval($template);
}
```

Passes (`app/render.php`):

```php
<?php

function render(string $template): void
{
    echo htmlspecialchars($template);
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

Re-recording runs offline against the pinned tool versions, with PHP 8.5.11:

```sh
python3 upstream/phpcs/generate.py fixtures <PHP_CodeSniffer checkout at 4.0.4> <composer project with squizlabs/php_codesniffer 4.0.4>
python3 upstream/phpstan/generate.py fixtures <phpstan-src checkout at 2.2.16> <composer project with phpstan/phpstan 2.2.16>
python3 upstream/psalm/generate.py fixtures <Psalm checkout at 6.19.1> <composer project with vimeo/psalm 6.19.1>
python3 upstream/<tool>/generate.py corpus <composer project> laravel=<laravel checkout> symfony=<symfony checkout>
```

The Psalm recorder reads which code Psalm leaves unanalysed with the usage-corpus generator it shares with the engine's
scope oracle, `fixtures/php/scope/php/generate_usage_corpus.py`.

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack php --rule psalm-unused-variable --fixtures
```

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `phpcs-generic-code-analysis-empty-statement`

Port of PHP_CodeSniffer 4.0.4 Generic.CodeAnalysis.EmptyStatement. Covers if, elseif, else, for, foreach, while, do, switch, try, catch, finally and match whose braced body holds only comments and white space.

### `phpcs-generic-php-backtick-operator`

Port of PHP_CodeSniffer 4.0.4 Generic.PHP.BacktickOperator. Like PHP_CodeSniffer, each shell command expression is reported twice: at the opening and at the closing backtick.

### `phpcs-generic-php-deprecated-functions`

Port of PHP_CodeSniffer 4.0.4 Generic.PHP.DeprecatedFunctions run on PHP 8.5.11: a call PHP resolves to one of the 67 internal functions ReflectionFunction::isDeprecated() reports for the extensions PHP_CodeSniffer's PHP loads. rerail/packs/php/upstream/phpcs records the list.

### `phpcs-generic-php-forbidden-functions`

Port of PHP_CodeSniffer 4.0.4 Generic.PHP.ForbiddenFunctions with its default list (sizeof, delete): a call PHP resolves to either function.

### `phpcs-generic-php-lower-case-constant`

Port of PHP_CodeSniffer 4.0.4 Generic.PHP.LowerCaseConstant. Catches true, false or null with any upper-case letter, also the fully qualified forms (\TRUE), but not when used as a type.

### `phpcs-generic-php-no-silenced-errors`

Port of PHP_CodeSniffer 4.0.4 Generic.PHP.NoSilencedErrors (every @ operator). Divergence (NER-1811): not reported where the @ is part of an idiom that needs it, because removing it changes behaviour:

- a silenced mkdir(), rmdir() or unlink() of a path that the right operand of a `&&`, `||`, `and` or `or` re-checks with is_dir(), is_file() or file_exists(), where the @ call is that operator's left operand or reaches it only through `!`, parentheses and earlier operands of the same chain (the race-safe `!@mkdir($dir) && !is_dir($dir)`, where the re-check decides whether a warning was a failure);
- a silenced trigger_error() or user_error() whose second argument, or argument named error_level, is E_USER_DEPRECATED (Symfony's deprecation convention, silenced so only error handlers that opt in log it).

### `phpcs-squiz-php-disallow-size-functions-in-loops`

Port of PHP_CodeSniffer 4.0.4 Squiz.PHP.DisallowSizeFunctionsInLoops: count(), sizeof() and strlen() in a for, while or do-while condition. Divergence (NER-1812): not reported when the loop changes the value it measures, because hoisting then changes what the loop does. The loop changes it when it assigns or appends to it (=, []=, +=, .=), unsets one of its elements, passes it to array_push, array_unshift, array_pop, array_shift or array_splice, or, for a $this property, calls a method on $this.

### `phpcs-squiz-php-eval`

Port of PHP_CodeSniffer 4.0.4 Squiz.PHP.Eval: every eval construct.

### `phpstan-argument-sprintf`

Port of PHPStan 2.2.16 argument.sprintf (PrintfParametersRule): a sprintf() call whose format is known statically and asks for more or fewer values than passed, or a format PHP refuses.

### `phpstan-argument-sscanf`

Port of PHPStan 2.2.16 argument.sscanf (PrintfParametersRule): only sscanf() calls that pass output variables (three or more arguments) are checked; reports a known format assigning more or fewer variables than passed, or a format PHP refuses.

### `phpstan-array-duplicate-key`

Port of PHPStan 2.2.16 array.duplicateKey (DuplicateKeysInLiteralArraysRule). Reported, as PHPStan does, at the first explicitly keyed element whose key PHP coerces to the same key as a later explicit key. Arrays spanning more than 256 lines are not checked: comparing every pair of keys of a larger literal (generated data tables) exceeds the per-file AST work budget, which would fail every policy on the file.

### `phpstan-if-always-false`

Port of PHPStan 2.2.16 if.alwaysFalse (IfConstantConditionRule): an if condition PHP's constant evaluation (variables followed through their one reaching binding) finds always falsy. Not reported: comparisons, negations, ternaries, isset and empty (PHPStan leaves those to other rules), a condition in a trait, or an if preceded by an inline @var doc comment (PHPStan reads the variable as the PHPDoc type). Divergence (NER-1810): an if whose block holds only class, interface, trait, enum or function declarations and class_alias calls (with comments), such as Symfony's `if (false) { class Router {} }` deprecation stubs, which Composer's class map and IDEs read and which must not be deleted, is not reported.

### `phpstan-if-always-true`

Port of PHPStan 2.2.16 if.alwaysTrue (IfConstantConditionRule): same evaluation and exclusions as if.alwaysFalse (constant evaluation following variables through their one reaching binding; skips comparisons, negations, ternaries, isset, empty, conditions in traits, and ifs preceded by an inline @var doc comment).

### `phpstan-missing-function-return-type`

Port of PHPStan 2.2.16 missingType.return as MissingFunctionReturnTypehintRule reports it: a function declaration (not a method or closure) with neither a native return type nor a @return, @phpstan-return or @psalm-return tag in its doc comment.

### `phpstan-random-int-parameters`

Port of PHPStan 2.2.16 RandomIntParametersRule (identifier argument.type): only fires when $min and $max, bound as PHP binds them (named arguments included), are constants with $min > $max.

### `phpstan-variable-undefined`

Port of PHPStan 2.2.16 variable.undefined (DefinedVariableRule): a variable read that no path defines ("Undefined variable: $x"). PHPStan's "might not be defined" reports (defined on some paths) are not claimed.

### `psalm-forbidden-code`

Port of Psalm 6.19.1 ForbiddenCode with no forbiddenFunctions configured: a call PHP resolves to var_dump or shell_exec (not a first-class callable), and a backtick shell command, which Psalm reads as a shell_exec call. Divergence (NER-1809): a var_dump call is not reported when its output is the point, because removing it changes behaviour: when it is a statement that an earlier statement in the same block starts buffering with ob_start() and a later statement reads the buffer with ob_get_clean() or ob_get_contents(), or when it is inside a function or method named dump or dd, where it is the fallback dumper.

### `psalm-unused-variable`

Port of Psalm 6.19.1 UnusedVariable (--find-unused-variables): a variable whose value no read uses, bound by an assignment statement, a compound assignment or increment, a closure's `use` clause, `global`, or a `foreach` key or destructured element. Psalm reports a `foreach` value as UnusedForeachValue and a parameter as UnusedParam (not this rule), and skips names starting with `$_`.
