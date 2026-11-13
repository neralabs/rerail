# Rerail rule pack: Vue, candidates

The ports in `rules/` here are still being evaluated: scored against the tools' expected findings and reported, never
gated and never shipped. This file is not published; the pack's [README](../README.md) covers the rules it ships.

The one candidate, `no-dupe-v-else-if`, fails `compact`.

Over all 24 ports, shipped and candidate: every one agrees with eslint-plugin-vue on every fixture case, 1,106 findings
on the 773 fixture files, with no false positive or false negative and no declared divergence.

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `eslint-plugin-vue-no-dupe-v-else-if`

Ports eslint-plugin-vue 10.11.1 `vue/no-dupe-v-else-if` with default options. Reads the script-block program's (`ast.embedded`) `if (...) { } else if (...) { }` chain (v-if / v-else-if elements, nested chains stay separate; the `true` written for a condition-less directive, whose host is its directive name, is no condition): for each else-if test it applies eslint-plugin-vue's or/and set-cover (the test and, when it is a `&&`, each flattened `&&` operand are the conditions; a condition is dead when every `||` operand of it is a superset, as a set of `&&` operands compared by tokens with `||`/`&&` operands in either order, of an `||` operand of some earlier test in the chain) and reports the first condition to become dead walking the chain from the nearest earlier branch, operands in source order and the whole test last, at that condition (parentheses excluded, as ESTree).
