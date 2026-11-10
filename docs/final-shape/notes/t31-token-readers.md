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

This plan first finds every such test (D). Most ask whether a token sits next to a node. The
grammar answers that from the parent's named children alone: the token is in the gap between
them (the gap test, below). Rules that report the token itself are rewritten to report the named
node that holds it. Afterwards D becomes a compile-time refusal, so no new rule can read a token
that nothing answers.

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

The census found far fewer shapes than rules. Most tests ask one question: "is this token next to
this node?" That is answered from the tree without the token being stored.

### The gap test

A parent's stored children are a word of its grammar: the kind's automaton reads them in order.
Where the automaton must pass a token between two children (the `,` between arguments), the gap
there holds it. `TokenGrammar::gaps` reads that from the parent's named children alone. It
reads no source, keeps no cache and builds no index. On 5.0 million parents in 2,578 files across
six dialects, its tokens equal tree-sitter's.

The compiler adds the test beside a relation whose target is a token regex
(`compile::ast_rule::gap_test`):

```jsonc
// The argument after a comma: the comma is implied in argument_list.
{ "follows": { "stopBy": "neighbor", "regex": "^,$" } }
// compiles to: the relation as before (a stored comma still matches as a node),
// or the gap before the node holds a comma.
```

A regex that also matches a name or a number (`(?s).*`, `^_`) is a test of node text. It is not
lowered: the tape's dropping a token does not change what it means.

### A token and more: the parts walk

Some targets say more about the token than its text:

```jsonc
// eslint-no-eval: the second argument, by the comma after the first one.
{ "follows": { "regex": "^,$", "follows": { "matches": "first-arg" } } }
```

Such a relation also walks the parent's parts: its stored children with the implied tokens
between them (`Tree::parts`). An implied token is tested by what the target says of its text and
of the parts around it (`follows`, `precedes`, `inside`, `not`, `all`, `any`, utilities). Nothing
else holds of a token. `has` past the neighbours descends the subtree in the same way.

At the top level the walk tests only implied tokens; the relation beside it answers for stored
ones. A walk whose target no token can pass (a target with a `kind`) answers false at once.

### Layout

`layout ... to: token` measures line breaks to the token beside a node. An implied token is the
first text that is not space between the node and the next stored leaf, so layout reads that
bounded window. Blank lines are counted only between leaves.

### Rules rewritten

Where the token *is* the node reported, the rule is re-rooted on the named node that holds it,
and reports where the token sits:

| Rule | Before | Now |
|---|---|---|
| E722, F707 | the `except` keyword | an `except_clause` with no value |
| no-sequences | the first `,` of a sequence | the sequence's first expression, at its end |
| require-await, explicit-function-return-type | the `=>` | the parameters before it, at their end |
| COM818 | a `,` ending the line | also the target before an implied `,`, at its end |

The four rules with the deleted `tokens:` key are rewritten the same way.

### The Datalog side

core.rr's `op`, `op_at`, `token` and `prev_token` hold the implied tokens too: their producer
reads each parent's gaps (`facts::vtab::syntax::tree::parts`). The converter writes a gap test
over them: `prev_token` for the token before a node, the parent's `token` rows for those after it.

### Where the census stands

| Category | Rules | Tests |
|---|---:|---:|
| `lowered` (gap test or parts walk) | 303 | 1,379 |
| `layout` (gap window) | 13 | 22 |
| `excluder` (`not` over a token: an implied one is in no set) | 147 | 305 |
| `root` (the token stored there: holes, trailing commas, modifiers) | 6 | 8 |
| `incidental` | 3 | 3 |
| `gap`, `gap?`, `below` | 0 | 0 |

### Checks

Each step compares the original rules on full tapes with the current rules on named-only tapes,
as finding keys:

- JavaScript: 1,229 = 1,229 keys on 100 files.
- TypeScript: 302 = 302 keys on 100 files.
- Python: 885 cases from Ruff's and Bandit's generators (the inline cases in `generate.py`), equal
  in every chunk. Two chunks hit a stale-store error in both modes (`noqa-PTH113.py`).
- The data-layer replay sample: 268 of 268 events give their recorded findings.

### Open

- The guard is a test for now (below), not yet a compile-time refusal.
- `first_token`, `last_token`, `token_before`, `token_after` relate nodes, and an implied token
  has none.
- A gap test on the Metal path is a fact, and facts are still made on the host there
  (`metal_host_guard_tests`' remaining entry).
- A gap's word placed against comments is approximate: before them, with the gap's span.
- A clause the transducer lowering refuses loses its findings in the slice runner rather than
  falling back.
- `fixtures:pull` needs AWS credentials (`s3://nera-test-fixtures`), which this checkout lacks.

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
- `tree.rs` had token helpers that nothing called (`count_tokens_before`, `count_tokens`,
  `Token::is_punctuation`). They are deleted; the others answer implied tokens from the gaps.

## The guard

`no_rule_reads_a_token_the_tape_may_leave_out` runs D over every pack rule. It fails on any
`root`, `below`, `gap` or `gap?` test that is not one of eight listed by rule and path
(`READS_STORED`), each with the reason its token is stored there.

Next, D's classification moves into rule compilation. A token test whose token the grammar implies
where the rule looks, and that neither the gap test nor the parts walk answers, becomes a compile
error naming the rule and the test. A rule whose parent can't be placed (today's `gap?`) has to say its parent. After that, a
rule can't depend on a token the tapes don't hold, and a grammar update that implies a new token
shows up at compile time.

## Checking still to do

The upstream cases with the linters' own findings (`fixtures.expected.json`, `npm run
fixtures:pull` from S3) are not in this checkout. S7 on every backend, the full data-layer checks
and a CPU measurement against a same-window baseline close the work.
