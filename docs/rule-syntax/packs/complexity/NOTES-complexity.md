# complexity: 7 rules, hand-written (lizard 1.24.0's cyclomatic complexity)

The 7 rules have 137 lines, against 402 in the machine translation. The counting is written once and the languages
are tables:

| file | what |
|---|---|
| `lib/complexity/cyclomatic.rr` | CCN = 1 + decision points inside a function, not entering nested functions (`within_stop` on the table's function kinds); the ceiling and the message |
| `lib/complexity/python.rr` | Python's function kind and decisions |
| `lib/complexity/ecma.rr` | TypeScript's and JavaScript's (and Vue scripts'), and how an unnamed function takes its variable's, field's or key's name |
| `lib/complexity/php.rr` | PHP's (and a Blade template's compiled program's, reached through `parsed(document, "compiled-view", program)`: `blade_code`); a closure takes its variable's name |
| `lib/complexity/rust.rr` | Rust's, with each `&&` of a let chain (`cc_decision(n, i)` has an index for these) |

Each rule is the directives, one include and one line: `complexity(f, msg) :- cc_report(f, msg).` Vue adds that
the function is in a `<script>` block's program (`vscript` from `lib/vue/component.rr`); its template's guest
expressions are left out. Blade adds `blade_code(f)` (`lib/blade/syntax.rr`): the compiled PHP is a document of
its own, derived from the template and reached through `parsed(document, "compiled-view", program)`, not the
template's last child as the first version's comment said (review fix; the engine requirement is in
packs/blade/NOTES-blade.md).

## Scores

The recording (`fixtures.expected.json`) is not in either checkout. The sibling checkout
(`Data-layer-rule-expressivity`) has an older corpus, Python, TypeScript and JavaScript only, with lizard's
`lizard-1.24.0.tsv`. A scratch recording was built from that TSV (every function over 6) and scored with
`rrlab score complexity ID --data <scratch>` on each rule's own fixtures (conformance.json's `python/**`, …):

| rule | lines | TP/FP/FN | notes |
|---|---|---|---|
| complexity-cyclomatic-python | 17 | 18/0/2, 2 ex | FN: match_case.py:18, 34 (a) |
| complexity-cyclomatic-typescript | 20 | 11/0/2, 2 ex | FN: branches.ts:123, nesting.ts:72 (a) |
| complexity-cyclomatic-javascript | 20 | 14/0/2, 2 ex | FN: branches.js:102, nesting.js:74 (a) |
| complexity-cyclomatic-php | 20 | no corpus | lizard run on a scratch corpus (b) |
| complexity-cyclomatic-rust | 19 | no corpus | lizard run on a scratch corpus (b) |
| complexity-cyclomatic-blade | 21 | not scorable | rrlab has no Blade grammar (packs/blade/NOTES-blade.md) |
| complexity-cyclomatic-vue | 20 | not scorable | rrlab does not re-parse `<script>` blocks (`parsed` is empty) |

"ex" are `divergences.json` entries (`??` and Python signatures lizard misreads). The scratch recording includes
test files (the TSV scores them), which rrlab reads too since it does not model `.scope`; both agree there.

(a) **The old corpus counts each `case`.** Its fixtures say "each case of a match counts once" and record
`match_over_limit`, `switchOverLimit` and `set level` at 7; the current JSON and README count a switch or match
once ("the lower of lizard's standard count and its -m count"), which gives 2 for each (and 5 for Python's
guarded match: 1 + match + 3 guards).
The rules follow the JSON; the current recording, built from the newer corpus, would agree with them.

(b) lizard 1.24.0 was installed into the scratch directory (pip, no Node) and run beside the rules on a PHP and a
Rust file covering every decision. Every difference is one the pack's README lists under "Deviations": PHP's `and`
/`or` (counted here, not by lizard), nullable types' `?` and a closure inside a function (lizard adds both to the
function), `$a ?: $b` and `??`; Rust's `where`, `?Sized` and `assert!(a && b)` (lizard counts them), and a trait's
required method (lizard scores it). Functions without those agree: plain 1/1, Rust `branches` 13/13, let chain 4/4.

## Deviations

None from the JSON. Where lizard and the pack differ, the pack's README already says so; the rules follow it.

## Library and core changes

None. rrlab: Blade facts (see the blade NOTES) and guest trees for Vue `<script>` blocks (`parsed`) would let the
last two rules be scored.

## Duplicates to merge

None: every relation is `cc_`-prefixed.
