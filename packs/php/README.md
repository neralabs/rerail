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
