# Blade batch 01: notes

Rules: `blade-parser-duplicate-condition-expressions`, `tlint-space-after-blade-directives`. Ground truth: the tools
themselves, run on a fleet box (stillat/blade-parser v2.1.0's `DuplicateConditionExpressionsValidator`, tightenco/tlint
v9.7.0's `SpaceAfterBladeDirectives` linter class, each called alone on every file), and the originals through
`product-run.ts`. Checked with `ptcheck` against the integration branch at 6627b8c851.

| rule | status | follows | corpora vs the tool |
|---|---|---|---|
| blade-parser-duplicate-condition-expressions | same | blade-parser | fixtures and samples-blade: same keys as the validator |
| tlint-space-after-blade-directives | different (10 translation-only, fixtures) | tlint | fixtures and samples-blade: same keys as tlint (40 and 40) |

## blade-parser-duplicate-condition-expressions

The earlier translation read the compiled view (`blade_code`) and reported nothing at all: `run-policies` gives a
`.rerail` rule no compiled-view document. A probe over the fixture (`parsed(r, "compiled-view", p)`, `guest(_, h, _,
g)`, `kind(n, "if_statement")`) has no rows; the view is built into the tape (`blade/tape.rs` `with_compiled_view`)
but no guest relation exposes it. So `lib/blade/syntax.rerail`'s `blade_code` is empty in every run; that is the
engine requirement NOTES-blade.md already lists (3), still open at 6627b8c851.

The rule now reads the Blade tree, as the validator does: a chain is a `directive_block`'s start and its branch
directives (or, in an attribute value, where Forte pairs nothing, an opener and the `@else…` directives after it at
nesting depth 0); a branch's text is its arguments, outer parentheses stripped while it starts with `(` and ends with
`)` (only where `(` and `)` touch the text, since Forte's `php` leaf is trimmed and blade-parser unwraps the untrimmed
text), then trimmed; an empty one (`@else`) is skipped; the first branch repeating an earlier text is reported.

On 30 hand cases the rule reports in the same 21 files as the validator, one finding each, at the directive the
validator names. Where the original differs, the validator sides with the translation:

| case | validator | translation | original (JSON) |
|---|---|---|---|
| `@unless($a) … @elseif($a) … @endunless` | reported | reported | not (`if (!($a))` is the view's condition) |
| `@if(($a)) … @elseif($a) … @endif` | reported | reported | not (`($a)` ≠ `$a`) |
| `@isset($a) … @elseif(isset($a)) … @endisset` | not | not | reported (the view's `isset($a)`) |
| `@unless($a) … @elseif(!($a)) … @endunless` | not | not | reported |
| `@empty($a) … @elseif(empty($a)) … @endempty` | not (`@empty` opens no chain) | not | reported |
| `@if($a /* c */) … @elseif($a) … @endif` | not (texts differ) | not | reported (the comment is not in `$P`) |
| `@php if ($a) {} elseif ($a) {} @endphp`, and the same in `<?php ?>` tags | not (not directives) | not | reported |

None of these occurs in the corpora, so the rule is `same` there. Not followed: blade-parser also pairs a custom
condition `@x … @elsex … @endx` when all three names occur in the document; Forte's registry reads an unknown
directive as text, so the tree has no directive to pair.

## tlint-space-after-blade-directives

Unchanged but for its comment. The 10 translation-only fixtures are control directives Blade leaves unpaired
(`Lead @forelse($arg) Trail`, `@for($args)`, a bare `@while`): tlint reports each, as it matches
`\B@(@?\w+(?:::\w+)?)([ \t]*)…` line by line; the JSON reports none, as the view compiles no half of a structure and
its `host` rule only sees compiled `for`/`foreach`/`while`/`if` statements. Counterexample: `@foreach($users as $u)`
with no `@endforeach` — tlint and the translation report line 1, the original nothing. Right: translation.

Known divergence from tlint, in neither corpus: tlint reads raw lines, so it also reports a control directive inside
`{{-- … --}}`, `@verbatim` or a PHP string (`{{-- @if($a) --}}`); the translation and the original read directives
only.

Harness note: `tlint lint --only=SpaceAfterBladeDirectives` also runs `NoSpaceAfterBladeDirectives` (the filter is a
substring match), so ground truth was taken by calling the linter class alone.

## Library and engine

- No library change.
- Engine (existing request 3 of NOTES-blade.md, confirmed): `parsed(document, "compiled-view", program)` rows in
  `run-policies`. Until then every rule over `blade_code` (the bladestan rules, `complexity-cyclomatic-blade`) finds
  nothing.
- equivalence.json (for the coordinator): `blade-parser-duplicate-condition-expressions` leaves `different`;
  `tlint-space-after-blade-directives` gets the reason and `right: translation` of blade-01.result.json.
