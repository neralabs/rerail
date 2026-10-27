# @neralabs/rerail-pack-php

## 0.1.4

### Patch Changes

- 7392aea: README and licence notes: the pack's test data now lives outside the package; no rule changes.
- c8d1afb: PHP pack: set-up no longer recommends the pack for a repository whose only PHP is tests (`test/`, `tests/`, `Test/`, `Tests/`, `spec/`, `*Test.php`, `*TestCase.php`), scripts and tooling (`scripts/`, `script/`, `bin/`, `tools/`, `ci/`, `.github/`), configuration (`config/`, `*.config.php`, dot-prefixed `.php` files such as `.php-cs-fixer.dist.php`, `rector.php`, `ecs.php`) or documentation (`docs/`, `doc/`), the way the Python and JavaScript packs already leave those out. Blade templates stay excluded. The reason set-up shows reads "PHP source files" instead of "*.php files". Installed packs are unaffected: the rules still run on every `.php` file.
- 211e8b5: PHP pack: PHP_CodeSniffer, PHPStan and Psalm are gated by the port readiness gates, and the seven ports that pass every gate and the pack's conformance check ship: `phpcs-generic-php-lower-case-constant`, `phpstan-argument-sscanf` and `psalm-forbidden-code` on, `phpcs-generic-code-analysis-empty-statement`, `phpcs-generic-php-backtick-operator`, `phpcs-squiz-php-eval` and `phpstan-missing-function-return-type` off; the other 11 stay candidates. `phpcs-generic-php-backtick-operator` reports the closing backtick where it is, not after it, and `psalm-forbidden-code` reports Psalm's whole selection, not its start.
- d0dd309: README: internal ticket references removed; no rule changes.

## 0.1.3

### Patch Changes

- e06db22: PHP pack: messages whose rewrite only changed the voice are back to their original wording; in an eval with a coding agent, the original and rewritten messages steered it equally well.
- e06db22: PHP pack: every rule description now says in one plain sentence what the rule wants; the exact matching detail moved to the pack README.
- e06db22: PHP pack: every violation message now says in one sentence what to change in the flagged code, without tool names or rule codes.

## 0.1.2

### Patch Changes

- da9e12b: First release through the release workflow with npm trusted publishing. The rules are unchanged.

## 0.1.1

### Patch Changes

- a0200c4: The package names the repository and directory it is published from, `neralabs/rerail` at `packs/<id>`.
- 007bb47: `psalm-forbidden-code` no longer steers an agent to remove a var_dump() whose output is captured with ob_start() and ob_get_clean()/ob_get_contents(), or that is the fallback inside a dump() or dd() helper.
- 007869f: `phpstan-if-always-false` no longer steers an agent to delete an `if (false) { class X {} }` declaration stub (deprecation aliases Composer's class map and IDEs read, as Symfony and monolog ship), which broke Symfony before.
- 4d3fc12: `phpcs-generic-php-no-silenced-errors` no longer steers an agent to remove the @ from a race-safe `!@mkdir($dir) && !is_dir($dir)` (or the same re-check after `@rmdir`/`@unlink`) or from `@trigger_error(..., E_USER_DEPRECATED)`, and its message now names the alternatives to removing @.
- 2a303bf: PHP pack: ships no rules until its ports pass the V1 rule validation gates. The 18 PHP_CodeSniffer 4.0.4, PHPStan 2.2.16 and Psalm 6.19.1 ports move to `candidates/rules/`, still scored against the tool's own findings on its tests, and the pack is never recommended while it has no rules.
- fbb6104: PHP pack: 18 rules ported from PHP_CodeSniffer 4.0.4, PHPStan 2.2.16 and Psalm 6.19.1, each scored against the tool's own findings on its tests.
- a05143d: `phpcs-squiz-php-disallow-size-functions-in-loops` no longer steers an agent to hoist count(), sizeof() or strlen() out of a loop that changes the value it measures, which broke work queues, buffers and fixed-point loops.
