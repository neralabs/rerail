# Rerail rule pack: PHP

15 code rules for PHP, ported from PHP_CodeSniffer 4.0.4 (8 sniffs), PHPStan 2.2.16 (6 rules) and Psalm 6.19.1 (1
issue). 10 start off (`"on": false`): no approved configuration of their tool turns them on, so they are experimental.
Bare `phpcs` runs PSR12, which has `Generic.PHP.LowerCaseConstant` alone of the ported sniffs, and PHPStan's default
level 0 does not register `missingType.return` (a level 6 rule); Psalm's `psalm --init` configuration reports
ForbiddenCode. Each is scored against the tool's own findings on the tool's tests, and against the labelled lines of
the tool's real-code samples ([docs/upstream.md](../../docs/upstream.md#samples)), through the engine's
production workspace path, with every disagreement declared in `divergences.json`. None of the three tools is
ECMAScript, so the tests carry no rename variants (`holds-under-rename`); the real-code samples are the pack's only
evidence that a port is not fitted to its tests.

A port starts in `candidates/rules/` and moves to `rules/` and `pack.json` once it passes the conformance check
([conformance/README.md](../../conformance/README.md#the-gate)); every tool's ports are gated
(`upstream/<tool>/gates.yaml`), and 3 stay candidates ([candidates/README.md](candidates/README.md)). The conformance
check (`--check`, run in CI) scores the tests and the samples: it gates the ports in `rules/` and reports the candidates
without gating them.

It follows the [pack structure and validation contract](../../docs/pack-format.md): it is checked by
`npx vitest run test/engine/catalog/packs.test.ts -t "pack php"`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): its match definition, and each rule's document, title, group and, for a rule that starts off, `on`; three rules carry examples |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `candidates/rules/` | The candidates' code-rule documents (`rerail.policy/v1`), scored but never shipped |
| `candidates/README.md` | What each candidate is waiting on and its rule details; not published |
| `conformance.json` | Every port (shipped under `rules/`, or a candidate under `candidates/rules/`): its upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tools |
| `divergences.json` | Declared disagreements with the tools |
| `upstream/<tool>/` | The tool's copied tests (`fixtures/`), its expected findings (`fixtures.expected.json`) and the generator (`generate.py`, sharing `upstream/common.py`); the lock naming its real-code samples (`samples.lock`, fetched by `npm run fixtures:pull`) with their sources and licences (`SAMPLES.md`); the tool's upstream manifest (`gates.yaml`: its pins, approved configurations, the file types it lints and the facts of every rule the pack ports) |
| `licenses/<tool>/` | The tool's licence and `UPSTREAM.md`, per upstream repository: its source, the patterns of the files taken and how (each file's provenance is `provenance.json` in the tool's test-data archive) |

Taking a new snapshot is a maintainer deriving the pack's test data from its pins and publishing it
([docs/upstream.md](../../docs/upstream.md#take-a-new-snapshot)); a pull request carries it only as a publish request
under `upstream/<tool>/publish/`. The recipe (`scripts/pack-data/recipes/php.ts`) runs each tool's `generate.py` over
its tests, against the pinned tool versions, with the pinned PHP 8.5.11 (`scripts/pack-data/runtimes/php/build.sh`)
and Composer:

```sh
npm run fixtures:derive -- --pack php                 # every tool
npm run fixtures:derive -- --pack php --tool psalm    # one
npm run fixtures:stage -- --pack php --tool psalm     # copy what changed into upstream/psalm/publish/
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack php --rule psalm-forbidden-code --fixtures
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

Port of PHP_CodeSniffer 4.0.4 Generic.PHP.NoSilencedErrors (every @ operator). Divergence: not reported where the @ is part of an idiom that needs it, because removing it changes behaviour:

- a silenced mkdir(), rmdir() or unlink() of a path that the right operand of a `&&`, `||`, `and` or `or` re-checks with is_dir(), is_file() or file_exists(), where the @ call is that operator's left operand or reaches it only through `!`, parentheses and earlier operands of the same chain (the race-safe `!@mkdir($dir) && !is_dir($dir)`, where the re-check decides whether a warning was a failure);
- a silenced trigger_error() or user_error() whose second argument, or argument named error_level, is E_USER_DEPRECATED (Symfony's deprecation convention, silenced so only error handlers that opt in log it).

### `phpcs-squiz-php-disallow-size-functions-in-loops`

Port of PHP_CodeSniffer 4.0.4 Squiz.PHP.DisallowSizeFunctionsInLoops: count(), sizeof() and strlen() in a for, while or do-while condition. Divergence: not reported when the loop changes the value it measures, because hoisting then changes what the loop does. The loop changes it when it assigns or appends to it (=, []=, +=, .=), unsets one of its elements, passes it to array_push, array_unshift, array_pop, array_shift or array_splice, or, for a $this property, calls a method on $this.

### `phpcs-squiz-php-eval`

Port of PHP_CodeSniffer 4.0.4 Squiz.PHP.Eval: every eval construct.

### `phpstan-argument-sprintf`

Port of PHPStan 2.2.16 argument.sprintf (PrintfParametersRule): a sprintf() call whose format is known statically and asks for more or fewer values than passed, or a format PHP refuses.

### `phpstan-argument-sscanf`

Port of PHPStan 2.2.16 argument.sscanf (PrintfParametersRule): only sscanf() calls that pass output variables (three or more arguments) are checked; reports a known format assigning more or fewer variables than passed, or a format PHP refuses.

### `phpstan-array-duplicate-key`

Port of PHPStan 2.2.16 array.duplicateKey (DuplicateKeysInLiteralArraysRule). Reported, as PHPStan does, at the first explicitly keyed element whose key PHP coerces to the same key as a later explicit key. Arrays spanning more than 256 lines are not checked: comparing every pair of keys of a larger literal (generated data tables) exceeds the per-file AST work budget, which would fail every policy on the file.

### `phpstan-if-always-false`

Port of PHPStan 2.2.16 if.alwaysFalse (IfConstantConditionRule): an if condition PHP's constant evaluation (variables followed through their one reaching binding) finds always falsy. Not reported: comparisons, negations, ternaries, isset and empty (PHPStan leaves those to other rules), a condition in a trait, or an if preceded by an inline @var doc comment (PHPStan reads the variable as the PHPDoc type). Divergence: an if whose block holds only class, interface, trait, enum or function declarations and class_alias calls (with comments), such as Symfony's `if (false) { class Router {} }` deprecation stubs, which Composer's class map and IDEs read and which must not be deleted, is not reported.

### `phpstan-missing-function-return-type`

Port of PHPStan 2.2.16 missingType.return as MissingFunctionReturnTypehintRule reports it: a function declaration (not a method or closure) with neither a native return type nor a @return, @phpstan-return or @psalm-return tag in its doc comment.

### `phpstan-random-int-parameters`

Port of PHPStan 2.2.16 RandomIntParametersRule (identifier argument.type): only fires when $min and $max, bound as PHP binds them (named arguments included), are constants with $min > $max.

### `psalm-forbidden-code`

Port of Psalm 6.19.1 ForbiddenCode with no forbiddenFunctions configured: a call PHP resolves to var_dump or shell_exec (not a first-class callable), and a backtick shell command, which Psalm reads as a shell_exec call. Divergence: a var_dump call is not reported when its output is the point, because removing it changes behaviour: when it is a statement that an earlier statement in the same block starts buffering with ob_start() and a later statement reads the buffer with ob_get_clean() or ob_get_contents(), or when it is inside a function or method named dump or dd, where it is the fallback dumper.
