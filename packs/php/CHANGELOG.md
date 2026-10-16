# @neralabs/rerail-pack-php

## 0.1.1

### Patch Changes

- a0200c4: The package names the repository and directory it is published from, `neralabs/rerail` at `packs/<id>`.
- 007bb47: `psalm-forbidden-code` no longer steers an agent to remove a var_dump() whose output is captured with ob_start() and ob_get_clean()/ob_get_contents(), or that is the fallback inside a dump() or dd() helper (NER-1809).
- 007869f: `phpstan-if-always-false` no longer steers an agent to delete an `if (false) { class X {} }` declaration stub (deprecation aliases Composer's class map and IDEs read, as Symfony and monolog ship), which broke Symfony before (NER-1810).
- 4d3fc12: `phpcs-generic-php-no-silenced-errors` no longer steers an agent to remove the @ from a race-safe `!@mkdir($dir) && !is_dir($dir)` (or the same re-check after `@rmdir`/`@unlink`) or from `@trigger_error(..., E_USER_DEPRECATED)`, and its message now names the alternatives to removing @ (NER-1811).
- 2a303bf: PHP pack: ships no rules until its ports pass the V1 rule validation gates. The 18 PHP_CodeSniffer 4.0.4, PHPStan 2.2.16 and Psalm 6.19.1 ports move to `candidates/rules/`, still scored against the tool's own findings on its tests, and the pack is never recommended while it has no rules.
- fbb6104: PHP pack: 18 rules ported from PHP_CodeSniffer 4.0.4, PHPStan 2.2.16 and Psalm 6.19.1, each scored against the tool's own findings on its tests.
- a05143d: `phpcs-squiz-php-disallow-size-functions-in-loops` no longer steers an agent to hoist count(), sizeof() or strlen() out of a loop that changes the value it measures, which broke work queues, buffers and fixed-point loops (NER-1812).
