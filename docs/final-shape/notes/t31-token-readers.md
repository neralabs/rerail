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

### A token beside other kinds

A token test often carries a `not` that keeps out a named node with the same text:

```jsonc
// typescript-eslint: the `accessor` keyword, not a property named accessor.
{ "has": { "regex": "^accessor$", "not": { "kind": "property_identifier" } } }
```

An implied token is never one of those kinds, so the `not` holds of it. The compiler gives such a
target the plain gap test. Where the `not` named a utility that only lists kinds (`name-token`,
`r37`), the utility is inlined in the rule.

### A token and more: rewritten

A target that says more about the token than its text is rewritten into the question it asks.
Most count arguments by their commas:

```jsonc
// eslint-no-eval, before: the argument after the comma after the first argument.
{ "follows": { "regex": "^,$", "follows": { "matches": "a0" } } }
// now: the second argument, comments not counted.
{ "nthChild": { "position": 2, "ofRule": { "not": { "kind": "comment" } } } }
```

`has` past the neighbours to a token (`stopBy: end`, or a stop rule) splits in two. The token is
in the node's own gaps, or in the gaps of a node the descent passes through:

```jsonc
// before
{ "has": { "stopBy": { "not": { "kind": "type_parameter" } }, "regex": "^,$" } }
// now
{ "any": [
  { "has": { "regex": "^,$" } },
  { "has": { "stopBy": { "not": { "kind": "type_parameter" } }, "kind": "type_parameter",
             "has": { "regex": "^,$" } } } ] }
```

The other rewrites:

| Rule | Before | Now |
|---|---|---|
| no-eval, no-invalid-this, accessor-pairs, RUF061, SIM905, SIM910, SIM911, AIR303 | the nth argument, by commas | `nthChild` among the non-comment children |
| no-self-assign, n-no-unsupported (`declare`) | a utility as the relation's target | the utility inlined: a plain gap test |
| curly | a break before or after the block's `}` | a break after its last child, or after the block |
| no-inline-comments | the layout of a `jsx_expression`'s braces | the layout of the `jsx_expression` |
| RUF007 | a slice's step, after two colons | also: after a colon, with an earlier child after one |
| FAST001 | a lone trailing comma | also: a one-element tuple |
| UP015 | a mode after a comma after `\` | a mode after `\` |
| UP034 | a `,` or `yield` that stops the scan | the same, asked of the gaps of its parent |

### Read as written

Some token tests read a token the tape still stores where the rule looks, so they hold as written.
A gap's tokens are left out only when they are exactly the grammar's implied word. Optional and
alternative tokens stay:

- a slice's leading colons (`x[:3]`, `x[::2]`): the implied word to a first child is empty;
- trailing commas (`[a,]`, `f(a,)`), a hole's comma, a for-in head's `const` or `var`;
- a method's `static`, `get` or `set`, and optional modifiers.

Each of these tests says so beside its regex, with the reason:

```json
{ "regex": "^,$", "stored": "a trailing comma is optional, so the tape keeps it" }
```

The compiler accepts `stored` only beside a regex, and refuses any other test it can't
answer (see [The guard](#the-guard)).

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
| `lowered` (gap test) | 222 | 1,164 |
| `layout` (gap window) | 14 | 22 |
| `excluder` (`not` over a token: an implied one is in no set) | 220 | 487 |
| `incidental` (a regex over node text, out of the gap test's reach) | 12 | 13 |
| `declared` (`stored`: read as written, above) | 4 | 20 |
| `root`, `gap`, `gap?`, `below` (refused by the compiler) | 0 | 0 |

The 18 `stored` declarations cover 20 tests: two sit in utilities that two clauses use.

Rules are counted by their worst category, so a rule moves when its worst test does.

### Regexes that match any node

The gap test once skipped a regex that also matches names (`(?s).`, `[\s\S]`, `^`), taking it
for a test of a node's text. Most of these are `not: {follows: {regex: "(?s)."}}`, a first
child, or its `precedes` twin, a last child. A node after an implied `(` was never a first
child, and on a named-only tape it would be one. That changed 17 tests in 13 rules: an ASI check
that now passed through `(`, a TODO link found past a `)`, `(a!) == b` flagged by the rule whose
message recommends it, and others. So the gap test now takes any regex that matches an implied
token, and a `not` of an extra (a comment) beside it. A `has` in a field takes it only where some
implied word holds a matching token in that field: `has: {field: type, regex: "(?s).*"}` asks
for a `type` child, and no grammar implies one. Each of these tests answers as the full tree
did, with no rule rewritten.

The 13 tests left are `inside`, `has` to the end and `stopBy` forms that the gap test does not
take. Each was read against the grammars, and they answer as before, with two exceptions. In
react-hooks-refs, a bare `return;` now counts as returning no node; the `return` keyword used to
match `[^;]`, so that test never held. In react-hooks-rules-of-hooks, only an empty tagged
template, useEffect``, differs.

String annotations parse into trees of their own. Those drop implied tokens too, as a tape does,
so a rule reads the same structure inside an annotation as outside it.

### Checks

Each step compares finding keys with a baseline recorded before the step:

- JavaScript: 1,229 = 1,229 keys on 100 files.
- TypeScript: 302 = 302 keys on 100 files.
- Python: the whole pack (641 rules) on the runtime path, against a baseline from before the
  branch, on 16 chunks of probe and noqa files. That is 13,403 keys before and 13,391 after.
  The difference is the B017 comment probe (the intended fix, two keys) and ten keys that the
  baseline held twice. The key sets differ only at B017. (An earlier version of this note
  compared the slice, which runs only the wave's rules; see below.)
- The slice on pprc-ac and pprc-af, which hit the stale-store error: 69 and 104 keys, no error.
- The data-layer replay sample: 268 of 268 events give their recorded findings.
- Rules that are off in the packs, and the rewritten ones: their crafted and upstream cases
  against the engine with the parts walk, on the rules before the rewrite. JS and TS are equal.
  Python is equal except three UP015 findings that Ruff reports and the old rule missed.

### Open

- `first_token`, `last_token`, `token_before`, `token_after` relate a node to stored tokens
  only (see [Token relations in `.rr` rules](#token-relations-in-rr-rules)).
- Ten keys the baseline held twice are now held once: PTH106 to PTH111 on line 3 of the
  `string-annotation-calls` probes, where `os.rmdir(path=p)` and `'os.rmdir(p)'` (a string in a
  string annotation) share a line. One of the two findings is no longer made. The key sets are
  equal, so the checks don't see it.
- Compiling the rules costs more than before the branch. See
  [CPU on the runtime path](#cpu-on-the-runtime-path).
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
  dropped silently. They are rewritten like the rest; no pack rule uses the key now, and an
  unknown rule key is a compile error.
- No rule carries `a124ac96a`'s edits any more: of the subtrees it added to 24 rules, only two
  generic ones (`{kind: comment}` and a `raise` test in E722) remain, put there on purpose by the
  later rewrite.
- `compare` with `text: "tokens"` (`matcher::compare::tokens`) already reads the text between
  leaves, so it sees implied tokens without any tree change. It stays.
- `tree.rs` had token helpers that nothing called (`count_tokens_before`, `count_tokens`,
  `Token::is_punctuation`). They are deleted; the others answer implied tokens from the gaps.

## Python checks: the slice runs half the pack

Rules that a pack turns off (`"on": false` in `pack.json`) are still loaded and run by
the key checks. But the Python checks ran the slice (`rer131_pack_keys`), and the slice
evaluates only the clauses that lower to its wave. On the Python pack, 342 of 641 rules
have no such clause, so they find nothing there:

| | in the wave | not in the wave |
|---|---|---|
| on in `pack.json` | 203 | 219 |
| off in `pack.json` | 96 | 123 |

So the findings that seemed lost with the off rules were never computed. It is not a
cross-file effect either: B017 and SIM910 alone in a pack find nothing on the slice.
The runtime check (`rer131_runtime_keys`) evaluates every rule. On probe chunk aa it
gives 467 keys from 641 rules, against the slice's 248, at 16.0 s CPU against 10–12 s.
The Python checks use the runtime now. The slice oracle names the rules it leaves out.

The runtime baseline from before the branch is equal on all 16 chunks except the B017 comment
probe (above, under Checks).

With the whole pack on the runtime, the probe and noqa files for AIR303, B017, FAST001,
FBT001, RUF061, SIM910 and UP015 all get their findings.

## B017: a comment before the string

`pytest.raises(Exception,  # c` followed by `"op()")` is the legacy form, which B017
exempts. The rule asked for the string as the first argument's next neighbour, and found
the comment instead. It now steps past comments. The comment probe gets no B017 with the
whole pack; the noqa and continuation probes still do.

## CPU on the Python probe chunks

The 20–27% rise compared a run at load 25–112 with a baseline taken at a different load. On
this machine, the same binary on the same chunk varies with load alone:

| chunk aa, one binary | load | CPU |
|---|---|---|
| | 112 | 26.0 s, 26.7 s |
| | about 100 | 12.4 s |
| | 28 | 10.1 s, 10.6 s |

At load 15, chunk ad takes 39.3 s (baseline 42.4 s) and pprc-ae takes 19.3 s (baseline
18.9 s). Under contention, nice-19 work spends more CPU time per unit of work: it runs on
the efficiency cores at lower clocks, with shared caches. CPU time counts the cycles the
process held, not the work done.

On the slice, two checks show that the work did not grow. On the runtime path, compiling
the rules did grow; see [CPU on the runtime path](#cpu-on-the-runtime-path).

- **The rule rewrites.** The 14 rules as they were before the rewrites and the same rules
  now, same binary, same chunk, back to back: 10.0 / 9.8 s against 10.6 / 10.1 s, with
  the same 248 keys.
- **A profile.** `sample` over 25 s of chunk ad: the gap lookups (`matcher::gap`) take 9
  of the 3,358 samples in engine code (0.3%). The parser's transducer walk takes 1,349,
  store writes 1,617, and SQLite 711.

## CPU on the runtime path

The runtime check compiles every rule, then evaluates it over the chunk. Instructions retired
(`/usr/bin/time -l`) don't vary with load, so they compare the two binaries directly. On
pprc-aa, run back to back:

| | before the branch | now | change |
|---|---:|---:|---:|
| compiling the 641 rules (no files) | 71.7 G | 94.4 G | +32% |
| compiling, then evaluating pprc-aa | 113.5 G | 137.8 G | +21% |
| evaluating alone (the difference) | 41.8 G | 43.3 G | +4% |

The work per file grew by 4%. Compiling grew by a third. The part of that the profile names:

- the compile-time token check (`token_tests::refuse`), about 7% of the run. It rebuilt the
  grammar's token tables for each rule document until `704809fec`, which builds them once per
  dialect;
- the gap test's automaton walk when a relation is compiled (`ast_rule::gap_test`), 4%;
- pattern parsing on the tape (`language::rule_pattern`, `tape::pattern_doc`), 4% now against
  1% before.

The baseline is from before the integration merge as well, so part of the rise may be the
merge's. A rule is compiled once per catalog, not once per file, so it is the oracle that
spends most of its time compiling. This is still open.

## The guard

Rule compilation refuses a token test the tapes may not answer. `compile_document` runs D's
classification (`compile/token_tests.rs`) on every rule document. A `root`, `below`, `gap` or
`gap?` test is an error that names the test's path and regex:

```text
a token test reads a token the tape may leave out, which nothing answers:
/utils/first-arg/follows (`^\($`, gap) ... or say why the token is stored there (`stored`)
```

A test that reads a token the tape keeps says so with `stored` (above); it counts as `declared`.
A grammar update that implies a new token shows up when the rules are compiled.

A neighbour walk (`stopBy: {not: X}`) is answered by the gap test where X is made of kinds,
utilities and token regexes. The walk passes the stored children that X matches, and the
implied tokens that its regexes match. That answers F631, RUF030, RUF040, RUF048 and RUF061
(their stop rule is the `trivia` utility) and clippy's partial-pub-fields as written.

`no_rule_reads_a_token_the_tape_may_leave_out` still runs D over every pack rule, including the
candidates the catalog doesn't compile.

## Stale store on pprc-ac and pprc-af

Named-only tapes made files of one shape share tape bytes. `os.path.exists("file")` and
`os.path.isfile("file")` store the same tape, and the names are in the source. Keyed by the
sha256 of its bytes, the second file's tape was found as the first's, and reading it against the
other source failed as stale. A tape is now keyed by its bytes and its source's hash
(`HashKey::tape_identity`), in the store, the working set, families and residency.
`files_of_one_tape_shape_keep_their_own_sources` checks it.

## Token relations in `.rr` rules

core.rr has two kinds of token relation:

- `token`, `prev_token`, `op` and `op_at` hold the implied tokens too. Their producer reads
  each parent's gaps, as the gap test does.
- `first_token`, `last_token`, `token_before` and `token_after` relate a node to a token
  entity: a stored anonymous child (`Facts::tokens`). An implied token is not an entity, so
  these relations skip it. `token_before(n, t), text(t, "(")` doesn't hold where the `(` is
  implied.

No program the engine runs uses the second kind: not the data layer, typing, golden, or gate
programs. They are used by 48 of the copied `docs/rule-syntax/packs` rules (34 TypeScript,
7 Python, 4 Vue, 2 Rust, 1 PHP), 122 times. In 45 of those uses the token's text is fixed:
`;`, `(`, `)`, `,` and `[` most often.

Proposed: the four relations mean the stored tokens, as now. A rule that asks for a token the
grammar may imply uses `token`/`prev_token`, which the gap test answers. The `.rr` compiler
refuses a use whose fixed text may be an implied token, as the JSON compiler does. Giving
implied tokens entity ids instead would rebuild the nodes the tapes leave out.

Doing this means rewriting the copies that ask for implied tokens. The copies are kept as
copied for now, so this waits on that decision.

## Checking still to do

The upstream cases with the linters' own findings (`fixtures.expected.json`, `npm run
fixtures:pull` from S3) are not in this checkout. S7 on every backend and the full data-layer
checks close the work. The Python key checks run on the runtime path against a baseline from before the branch.
