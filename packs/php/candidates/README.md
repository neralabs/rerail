# Rerail rule pack: PHP, candidates

The ports in `rules/` here are still being evaluated: scored against the tools' expected findings and reported, never
gated and never shipped. This file is not published; the pack's [README](../README.md) covers the rules it ships.

## Candidates

The title is what `pack.json` lists for a candidate once it ships; its group is the tool the title names. The reason
is why it stays a candidate.

| Candidate | Title | Why |
|---|---|---|
| `phpstan-variable-undefined` | PHPStan variable.undefined: Undefined variable | 13 files differ; it needs PHPStan's types |
| `phpstan-if-always-true` | PHPStan if.alwaysTrue: If condition always true | type narrowing needs PHPStan's types |
| `psalm-unused-variable` | Psalm UnusedVariable: Unused variable | 6 of Psalm's cases differ (references and `global`, a call the engine cannot resolve, and one where Psalm reports another line), which needs engine work on PHP scope and usage (`scope/usage/php.rs`) |

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `phpstan-if-always-true`

Port of PHPStan 2.2.16 if.alwaysTrue (IfConstantConditionRule): same evaluation and exclusions as if.alwaysFalse (constant evaluation following variables through their one reaching binding; skips comparisons, negations, ternaries, isset, empty, conditions in traits, and ifs preceded by an inline @var doc comment).

### `phpstan-variable-undefined`

Port of PHPStan 2.2.16 variable.undefined (DefinedVariableRule): a variable read that no path defines ("Undefined variable: $x"). PHPStan's "might not be defined" reports (defined on some paths) are not claimed.

### `psalm-unused-variable`

Port of Psalm 6.19.1 UnusedVariable (--find-unused-variables): a variable whose value no read uses, bound by an assignment statement, a compound assignment or increment, a closure's `use` clause, `global`, or a `foreach` key or destructured element. Psalm reports a `foreach` value as UnusedForeachValue and a parameter as UnusedParam (not this rule), and skips names starting with `$_`.
