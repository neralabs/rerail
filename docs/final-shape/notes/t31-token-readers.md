# T31: rules that read tokens a tape no longer stores (RER-131)

## Intro

A named-only tape (t27 §3, RER-131) leaves out every token the grammar implies: the `(` and `,`
of a call, the `:` of a pair, the `except` of an `except_clause`. The tape's writer drops them as it
walks tree-sitter's tree. Nothing rebuilds them.

Some pack rules test those tokens by text. One example is "the child that precedes `:`" for a
pair's key:

```jsonc
// eslint-sort-keys: a pair's key, found by the `:` after it. The `:` is implied in `pair`,
// so the test can no longer hold.
{ "kind": "pair", "has": { "stopBy": "neighbor", "precedes": { "regex": "^:$" } } }
// The same question about named structure:
{ "kind": "pair", "has": { "stopBy": "neighbor", "field": "key" } }
```

Such a test now fails silently. The rule loses findings, or under a `not` it gains false ones.
A crafted Python file loses `ruff-e722`, and the JavaScript pack loses 117 of 906 keys on 15 files.

This plan finds every such test before changing anything (D). Each one is then rewritten to ask
the question it was really asking, about named nodes, and each pack is checked against its
upstream cases. Afterwards D becomes a compile-time refusal, so no new rule can read an implied
token.

Neither the engine nor the tapes put the tokens back. An earlier attempt (`a124ac96a`, removed in
`1b9c360dc`) kept the old meaning by recognising tokens from the source whenever a rule asked. That
was reconstruction on read, and with its caches and indexes it cost 2.6× the CPU and 2.7× the
memory on the 9,010-file repository.

## D: the census

`tape::token_readers::token_readers` (ignored; `RERAIL_TOKEN_READERS=<tsv>`) reads every rule of
every pack. It finds each *token test*: a sub-rule with a `regex` and no `kind` or `pattern`, whose
regex matches a token of the dialect's grammar. It uses the grammar's automata
(`TokenGrammar::implied_tokens`, `parents`) to say whether that token can be implied where the
rule looks. The token's parent comes from the rule: the node's `kind`, or its neighbour `inside`'s
kind. Where the rule doesn't give it, D uses every kind the grammar has the node under.

| Category | What it is | Rules | Tests |
|---|---|---:|---:|
| `gap` | `has` (neighbour) or `follows`/`precedes`: the token sits in a gap between named siblings, under a known parent that implies it | 377 | 1,323 |
| `gap?` | the same, but the rule does not say the parent; implied in some kind, perhaps not this one | 26 | 372 |
| `root` | the node matched is the token itself (E722's `except`, `no-sequences`' `,`) | 61 | 125 |
| `below` | `has` with `stopBy: end`: the token anywhere under the node | 7 | 98 |
| `layout` | `layout` before or after, `to: token`: line breaks to the adjacent token | 12 | 19 |
| `incidental` | a regex about node text that also reached tokens (`.`, `^[rub]*f`) | 22 | 169 |
| `stored` | the token is never implied there (operators, `async`, holes, trailing commas) | 112 | 1,442 |
| `never` | `inside` a token, which never held | 1 | 198 |

The Rules column counts each rule once, under its worst category. That leaves 483 rules in
`root`, `below`, `layout`, `gap` and `gap?`.

The `gap` tests cluster in a few parents: `argument_list` (549 mentions), `array`, `subscript`,
`arguments`, `tuple`, `tuple_pattern`, `keyword_argument`, `attribute`. The shapes that recur:

| Shape | What it asks | Rewrite |
|---|---|---|
| `follows ^\.$` in `attribute` | the attribute name | `field: attribute` |
| `follows ^=$` in `keyword_argument` | the value | `field: value` |
| `precedes ^:$` / `follows ^:$` in `pair` | the key / the value | `field: key` / `field: value` |
| `follows ^\[$` in `subscript` | the index | `field: subscript` |
| `follows ^\($` in `argument_list` | the first argument | `nthChild: 1` (named) |
| `has ^,$` in `tuple`, `subscript`, `expression_statement` | two or more elements | a count of named children, `min: 2` |
| `not has ^[(),]$` in `argument_list` | nothing but these arguments | a count, or `not has` over the named children |
| `has ^:$` in Rust `field_pattern` | not shorthand | `has: { field: pattern }` |
| commas counted with `follows ^,$` chains (ESLint ports) | an element's position | `nthChild` |

Each of these is a question about named structure, already in the rule DSL and run on every
backend (interpreter, Cranelift, Metal). None needs a new primitive.

## How each category is addressed

1. **`gap`: codemod into the structural forms above.** A one-off script rewrites each recurring
   shape, idiom by idiom, under the parent D names. Shapes the script doesn't know are rewritten
   by hand. The script is not shipped. If a gap test is left that no structural form can express,
   it gets one small engine primitive: whether a given token is in the gap before or after a node,
   answered from the parent's automaton state, which is a table lookup over the parent's named
   children with no source read. It is added only if such a test turns up, and then on every
   backend.
2. **`gap?`: settle the parent first.** Most are tests on a named node's text that D couldn't
   place: `^global$` is the identifier in `global.Array`, not TypeScript's `declare global`. D
   learns to follow kinds through utils. What is left is reviewed by hand.
3. **`root`: re-root on the named node.** The rule matches the parent (`except_clause` with no
   type; `else_clause`; `sequence_expression`; `variable_declaration` of `var`) and reports there.
   A finding key holds the line, and the token is on its parent's first line in these rules. A few
   are D misreads (kinds given in a util: `ytt101`'s `^\*…`, Rust's digit regexes) and drop out
   with item 2's fix.
4. **`below`: a descendant kind.** The token implies its parent's kind, so "a `:` anywhere below"
   becomes "a descendant of the kind that holds it". Rewritten by hand.
5. **`layout ... to: token`: the one engine change.** It measures line breaks from a node to the
   token beside it. Where that token is implied, it lies in the gap before the next stored node,
   and its position is the first non-space, non-comment byte of that gap. Layout reads that
   bounded window, with word operations rather than a per-byte loop. Nothing is cached, and the
   change applies wherever layout runs.
6. **`incidental`: review.** `regex: "."` on a sibling used to mean "any sibling, token or named";
   now it means a named one. Each such rule is checked for whether that changes its answer.
7. **`stored` and `never`: nothing to change.**

## Other readers found on the way

- Four rules came in with the integration merge using the `tokens:` syntax that `1b9c360dc`
  deleted: `eslint-max-params`, `typescript-eslint-max-params`, `eslint-max-statements` and
  `eslint-no-unexpected-multiline`. The compiler ignores the unknown key, so those tests are
  dropped silently. They are rewritten like the rest, and an unknown rule key becomes a compile
  error.
- Ten candidate rules still carry `a124ac96a`'s edits. They are restored, then rewritten like the
  rest.
- `compare` with `text: "tokens"` (`matcher::compare::tokens`) already reads the text between
  leaves, so it sees implied tokens without any tree change. It stays.
- `tree.rs` still has token helpers that nothing calls (`count_tokens_before`, `count_tokens`,
  `token_after`, `is_punctuation`). They are deleted.

## The guard

Once the rules are rewritten, D's classification moves into rule compilation. A token test whose
token the grammar implies where the rule looks becomes a compile error naming the rule and the
test. A rule whose parent can't be placed (today's `gap?`) has to say its parent. After that, a
rule can't depend on a token the tapes don't hold, and a grammar update that implies a new token
shows up at compile time.

## Checking

The oracle is each pack's upstream cases with the linter's own findings (`fixtures.expected.json`,
`npm run fixtures:pull` from S3). They are not in any checkout here. Each pack's rewrite is
checked against them, and against key sets on the corpus samples. S7, the data-layer checks and a
CPU measurement against a same-window baseline close the work.

## Order

1. D follows kinds through utils; `gap?` and the root misreads shrink.
2. Pull the upstream cases.
3. `gap` codemod, pack by pack (Python, JavaScript, TypeScript, then the rest), each checked.
4. `root`, `below`, `incidental` and the leftover rules, by hand.
5. Layout's gap window.
6. The guard, and unknown rule keys refused.
7. S7 on all backends, the data-layer checks, the CPU measurement.
