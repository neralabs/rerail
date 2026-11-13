# T31: rules that read tokens a tape no longer stores (RER-131)

## Intro

A tape stores a file's named nodes and leaves out the tokens its grammar implies. The writer
decides that from the grammar alone, never from the rules. Nothing rebuilds a token it left out:
no reconstruction, no cache, no side table.

A rule that asked for such a token by its text, or used it for its place, would now be wrong
without saying so. So each rule asks about the named structure, or about the source around a
node, and a check refuses any rule that still asks for a token the tape may leave out.

```text
f(a, b)        stored: call, identifier f, argument_list, a, b
               left out: ( , )          the argument list implies them
def f(): pass  stored: def, :           both can be other words in their gap (async def, a return type)
```

## What the tape stores

The writer reads each kind's grammar as an automaton over its children. Between two named
children (a gap), the grammar may admit several token words. The shortest one, where it is
unique, is the implied word. Two classes of token follow from that:

- **Delimiters** (`( ) [ ] { } < > , ; |`, quotes, `${`, `</`, `/>` and the like) are left out
  wherever the gap holds exactly the implied word.
- **Every other token** (keywords, operators) is left out only where its kind implies it in every
  context. A token that some gap holds in a word other than the implied one is stored
  everywhere, even in gaps where it is implied.

So `return` in a return statement is left out (always there), but Python's `def` is stored
(`async def` is another word of that gap), and so are `except` and a slice's `:`. An operator
that is a choice (`+` or `-`) is stored. A gap that holds a hole (`[a, , b]`) is not the implied
word, so all its tokens are stored.

Per label (token, field) the class is computed once per grammar (`tape/tokens.rs`: `kept`,
`Automaton::dropped`). The tape format is 15.

## The numbers

`drop_census` (ignored; `RERAIL_FILES`, `RERAIL_DROP_CENSUS`) counts what the writer leaves out
over real files. Anonymous tokens left out, before (all implied words) and now:

| Dialect | Files | Named nodes | Left out before | Left out now | Records |
|---|---:|---:|---:|---:|---:|
| Python | 13,088 | 22.3 M | 10.69 M | 10.25 M | +1.9% |
| TypeScript | 615 | 1.12 M | 676 K | 659 K | +1.2% |
| JavaScript | 248 | 380 K | 251 K | 245 K | +1.3% |
| Rust | 1,206 | 4.00 M | 2.84 M | 2.76 M | +1.6% |
| PHP | 94 | 57 K | 35 K | 33 K | +3.2% |

No keyword or operator is stored in some places and left out in others any more.

## Reading around a node

Two engine relations in `core.rr` answer layout questions from the source:

- `structural_before(n, s)`: the source between n's start and the end of the stored child just
  before it (or its parent's start);
- `structural_after(n, s)`: the source between n's end and the start of the next stored child (or
  its parent's end).

```text
f(a, b)    structural_before(a, "(")   structural_after(a, ", ")   structural_after(b, ")")
```

They are the source, not tokens: a rule that asks which token stands there asks the kind, the
children or the fields. `lib/layout.rr` builds on them for the questions ESLint asks of its token
store: `next_code` / `prev_code` (the code after or before a node, past comments and up through
ancestors that end or start with it), `after_line` / `before_line`, `touch_before` /
`touch_after`, and `in_parens` (isParenthesised). A rule names the nodes it asks about with
`layout_at(x)`.

## The check

Both rule languages are checked when they load.

- **JSON rules**: the guard in `compile/token_tests.rs` refuses a token test the tape may not
  answer, naming the test's path and regex. A test that reads a token the tape keeps says so with
  `stored` and a reason.
- **`.rr` rules**: `logic/rr/token_reads.rs` refuses a clause that reads a token by its text
  where the tape may leave it out: `op`, `op_at`, `token`, `prev_token`, and the four leaf
  relations `first_token`, `last_token`, `token_before`, `token_after`. A leaf relation read for
  its place (a line, an edit, a chain, with no test on the token's text) is refused the same way.
  A relation whose reads are exact by construction says so: `.stored rel "why"`.

Both use the same classes as the writer (`Automaton::dropped`), so a grammar update shows up
when rules load.

check-rr over the 1,007 rule files of `docs/rule-syntax`: no clause reads a token the tape
leaves out.

## How the rules were rewritten

Locations: a finding is keyed by its rule, code, document and line, so a rule that reported at a
token reports at the node that starts there, at a capture in the node's source, or at the source
line the token is on. Edits need the token's exact span: they capture it in the node's source
(`match("(?s)(?<t>var)\\b.*", d) => replace(t, `let`)`).

| Question | Before | Now |
|---|---|---|
| a keyword the kind implies (`var`, `type`, `interface`, `import`, `print`, `module`) | the first token | the kind, or the first word of the node's source |
| `export = v`, `declare global {}` | `op(e, "=")`, `op(a, "global")` | the source before v; the ambient declaration's first child is a block |
| `as const`, `<const>x` | `op(n, "const")` | no type after `as`; empty type arguments |
| ASI hazards, `;` needed, glued tokens | `token_after`, `token_before` | `next_code`, `prev_code`, `touch_before` |
| a body's lines (curly, no-lonely-if) | the line of the token before | `before_line`, `after_line` |
| array slots and holes | commas walked as tokens | commas in the source between children, and the stored commas of a hole's gap |
| ESLint's token comparison (`token_text`) | tokens joined | stored leaves joined with the source between stored children |
| Python slices, UP034, UP003 | the parent's text, substrings | stored `:`, `structural_before` / `structural_after` |

Engine code that read tokens: `scope/ecma.rs` takes `module`'s keyword from its kind (an
internal_module stores its `namespace`), and typescript-eslint's declaration-file marks read
`export =`, `export *` and `export default` from the statement's source.

## Checks

- check-rr: 886 of 1,007 files pass; the rest fail for reasons that are not tokens (below).
- `rule_syntax_files_pass_check_rr` (declarations and strata): passes.
- Probes with run-rr on small TypeScript files: no-sparse-arrays, no-sequences,
  no-unexpected-multiline, no-lonely-if, require-await, arrow-body-style,
  no-confusing-non-null-assertion, prefer-namespace-keyword, no-extra-non-null-assertion and
  consistent-type-definitions report the lines ESLint does. dot-notation, no-array-constructor,
  no-self-compare, yoda and no-inferrable-types could not be probed: they stop on the load
  failures below.
- Targeted tests: tape, tape tokens, tree, the syntax facts, `compile::binds`, the convert replay
  sample, and `tokens_are_the_stored_neighbours`.

## Open

- **Rules that fail to load for other reasons**, hidden before behind the token refusals:
  unbounded value cycles (`norm`/`sci` in 20 files, `unescape` in 10), `$k is a Name and a Kind`
  (4), message text read as a placeholder (`eslint-curly`, `no-template-curly-in-string`,
  `no-object-constructor`), and `ordered`, `match_all` over a computed node in perf403.
- `pattern_literals_match_and_bind_their_metavariables` fails; it is a separate item.
- The JSON gap test (`matcher/gap.rs`) stays as it is (decided).

Fixed in the engine since:

- **`match_all` over a node.** `match_all("re", n, m)` over a node a sibling binds to a kind (`kind(n, "arguments")`)
  lowers to the batch's `rx_span` over that kind's nodes, so it gives each match as a span. Before, it read the
  `matchall_` relations, which are made over `source`, `src_line` and comment spans only, and gave no rows.
  A derived relation's column still reads `matchall_` (it may hold source spans, as the data layer's do).
- **Counting rows under `_`.** An aggregate's helper relation kept only the body's named variables, so rows that
  differed in a `_` were one: `count : { match_all(",", n, _) }` gave 1, `sum strlen(v) : { kv(_, v) }` summed
  distinct values. For `count`, `sum` and `product` each `_` is now a column, as Soufflé counts tuples and as the
  engine's one-atom aggregate already did.
- **Undeclared relations.** check-rr and a run refuse them; `rule_syntax_files_pass_check_rr` did not. It does now
  (`check::undeclared`), which found `lib/python/imports.rr` and `misc-seen.rr` reading `names.rr` without
  including it, and `nested-routers.b1.rr` reading `ref_site` without `tsx/names.rr`.
- **Languages of several dialects.** `.language typescript` covers `.ts`, `.tsx` and `.js` files. run-rr runs the
  policy once per dialect with files in scope (a program is lowered for one grammar), the first into `out`, the
  others into `out/<dialect>`, and reports the findings together. Before, it read the `.ts` files alone.

Found while checking those fixes:

- `simplify::merge_keyed` renamed a merged variable in the head's bare terms only, so `rn(n, @lower(t))` lost `t`
  when its `text(n, t)` merged with an inlined one (`$t is unbound in SQL`). It renames inside head terms now.
- `@to_precision` had no SQL or native function; it is in `strings::functions`, as JavaScript's toPrecision.
- `token_text` of a leaf (`1`, `x`) was empty, so no-self-compare took any two leaves for equal.
- Value cycles the engine refuses, rewritten as one step: `stored` reads the literal's digits by string
  operations (not `sci`), `unescape` and `natural_key` decode pieces split by `match_all` and joined by ordinal.
  Rust's `hex_place` reads a table of powers of 16, and Python's `b005_char` numbers characters by `match_all`.
- Pattern literals (``n ~ `…` ``) matched nothing: patterns are tapes with no implied token (`tape::pattern_doc`),
  and `ast_match` ran them over tree-sitter's full tree. They run over the document's tape now (`ast_match` is in
  `TAPED`).
- Rules that read a kind or a span as text, refused at plan time: Rust's `lit_body` and `raw_str` matched groups on
  the node (spans) and compared them with `""`; `prefix("jsx_", k)` and `lit_val(n, k, k) :- kind(n, k)` read a
  kind as text. They match the node's text, list the JSX kinds, and name each keyword's kind.

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

## Stale store on pprc-ac and pprc-af

Named-only tapes made files of one shape share tape bytes. `os.path.exists("file")` and
`os.path.isfile("file")` store the same tape, and the names are in the source. Keyed by the
sha256 of its bytes, the second file's tape was found as the first's, and reading it against the
other source failed as stale. A tape is now keyed by its bytes and its source's hash
(`HashKey::tape_identity`), in the store, the working set, families and residency.
`files_of_one_tape_shape_keep_their_own_sources` checks it.
