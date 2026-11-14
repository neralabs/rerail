# Notes: the plan group

This group ports four checks from the data-layer repository's Python linters to `.rr`. The specification is
each linter at commit 2b1504adc. A score counts the recorded linter calls (events) whose finding keys the rule
reproduces exactly, using plain `tools/rrlab/rrlab dl` with `--jobs 2`. The last column says whether the engine's
`.rr` reader (`rr-census`) lowers every clause.

| check | Python lines | .rr lines | score (exact/events) | reader lowers it |
|---|---|---|---|---|
| plan_composition_architecture | 354 | 120 | 12/13 | yes |
| plan_helpers_allowlist | 607 | 166 + 35 generated | 27/82 (75/82 with message codes) | yes |
| plan_optimizer_v2 | 1,032 | 242 | 159/166 | yes |
| eager_resolve | 2,666 | 551 + 115 generated | 72/77 | yes |
| total | 4,659 | 1,079 + 150 generated | 270/338 (318/338) | |

Files:
- the rules: `plan-composition-architecture.rr`, `plan-helpers-allowlist.rr`, `plan-optimizer-v2.rr` and
  `eager-resolve.rr`;
- the generated tables: `plan-helpers-allowlist.tables.rr` and `eager-resolve.tables.rr`;
- their generators: `gen-plan-helpers-allowlist.py` and `gen-eager-resolve.py`. Both read the linter with
  `git show`, and `--check` prints ok when the committed table matches.

No library file was added: none of the vocabulary is shared by two checks.

## The misses

**plan_composition_architecture, 1 miss.** `test_live_composition_positive` has no files. Its finding comes from
the run artifact `plan_composition_probes`. rrlab does not model `artifact=` inputs, so the event cannot run. I
checked the probe logic by reading the same observations through a `filename=` copy of the input.

**plan_helpers_allowlist, 55 misses: 48 recording-shape misses and 7 scope misses.**
- Every finding is PHA001, as the plugin reports it: `.violation PHA001(at: node, c: symbol) "{c}"`, with the
  message saying which case it is. The direct-call recordings (`scan_file`, `scan_repo`) carry the message as
  their code ("old or private H() call is forbidden"). That is a recording shape, not a code the plugin reports.
  `--codeless` does not apply, because those records do have a code. The 48 events that differ only in that way
  are recording-shape misses.
- A scratch copy of the rule that differs only in `code {c}` scores 75/82, so those 48 events are otherwise
  exact.
- The other 7 misses are `scan_repo` walk-skip events. They put a finding-shaped file under a directory the linter
  does not walk (`tests`, `conform`, `docs`, `.venv`, `.git`, the terminals and `data_layer_testing`) and expect
  nothing. The rule's `.scope dirs` and `.scope tests false` skip those files in production, but rrlab ignores
  `.scope` on direct calls.

**plan_optimizer_v2, 7 misses.** All seven are missing-document events with no files. They call `find_violations`,
which runs the doc sync even when there is no v2 file and reports PV2-050 for the missing doc. The plugin, which is
what production runs, syncs only after it has seen the verifier's facts. The rule follows the plugin: the
missing-doc PV2-050 needs `impl/common/l2065_plan_core/verifier.py`. These are recording-shape misses: the recorded
entry and the plugin differ.

**eager_resolve, 5 misses.**
- `test_signed_external_io_boundary_suppresses_owned_eager_db_terminals` monkeypatches the external-I/O allowlist
  to contain the test's symbol. The rule reads the real allowlist file, which a fixture cannot supply with a valid
  signature.
- The four `_scan_legacy_boundary_definition_ast` events call an entry that always returns `[]` ("legacy-boundary
  definitions are inventory"). No event of that entry has a finding, so dl compares it on every code, and the
  whole-linter rule reports transitive sites and EAGER-ALLOWLIST there. The rule is right; the comparison is not.

The BIG events are 11 real repository files, from 126 to 14,248 lines, and each expects `[]`. All of them pass. A
direct `rrlab run` over the 11 files, with the repository's real allowlist files added, gives no finding: the 34
external-I/O entries and the burn-down ceiling verify. A fabricated entry is reported.

## Reader status

All four rules lower with no refused clause.

plan_helpers_allowlist no longer uses `ordered by earlier`. That form was not needed. Whether a write puts a helper
in the linter's map is monotone, because entries are never removed (`pha_stores`). Only "which qualifying write is
the latest" needs an order, and that is a `max` over a lower stratum (`pha_latest`). This plain form, taken from
the review, scores the same and takes 1 Soufflé round instead of 4.

## Review fixes

**plan_helpers_allowlist:** the PHA001 code and the plain `pha_stores` / `pha_latest` form, both above.

**eager_resolve:**
- A nested def's decorators are no longer part of the outer body (`er_stop("decorator")`).
- `fn: object = publish` is not a legacy-boundary alias.
- A receiver chain no longer follows `await`.
- A parenthesized bound method, `register((x.resolve))`, is found.
- The phase scan reads only the last def of a symbol, as the linter's map keeps it.
- Lazy and SQL locals are collected in statement order: `y = x` counts only after x is lazy. The patterns test
  against the whole set, as the linter's precomputed sets do.
- "Contains an SQL-derived expression" looks through the linter's expression kinds only: calls, keyword values,
  attributes' objects, operators, subscripts and displays. It does not look into comprehensions, lambdas or
  `*rows`.
- Annotation mentions look through subscripts, `|`, tuples and type parameters only, so not into
  `Callable[[LazyResult], int]`.
- The grandfathered fragments' errors are reported as EAGER-ALLOWLIST: blank lines, the field count, unnormalized
  source paths, a source that does not match its fragment, and duplicates.
- The comment-as-argument case (`getattr(  # c`) now passes with the library's `posarg` fix.

**plan_optimizer_v2:**
- The design doc is in `.scope include`, so PV2-051 and the missing-doc PV2-050 stay in production.
- `DEFAULT_V2_PASSES = X = (…)` counts.
- `Protocol[T]` is not a protocol base.
- Parenthesized literals count, in four places: the first argument of `split(".")` and of
  `ColumnSchema("x")`, `code=("scope.c")`, and the phase value.
- PV2-044 counts `\r\n`, lone `\r` and `\n` line ends, as universal newlines read them.
- `attr_of` replaces the repeated attribute pattern.
- The missing-doc PV2-050 has its own violation (`PV2_050_DOC`), with the linter's message.

**Aggregate variables.** I checked every aggregate in the four rules for a variable shared with the clause outside
it. Each shared variable is a group key on purpose (for example, `x` and `e` in `pha_latest`, and `d, c, s` in
`ER`'s first line). None is a leak.

## Choices worth knowing

- **plan_helpers_allowlist: PHA000** is a gate. When the signed allowlist does not verify, it is the only finding.
  rrlab now has a real `@ed25519_ok` when libsodium is installed, and the allowlist verifies.
- **eager_resolve: message text.** For AST sites the linter's message ends with `ast.unparse(node)`. For transitive
  sites it ends with the first line of `ast.unparse(def)`. The rule uses the stripped source line in both cases.
  Only the keys are compared (the loose findings contract).
- **eager_resolve: approximations,** each outside every recorded event:
  - The second wrapper shape (`h = x_lazy(…)` then `return run_in_legacy_execution_boundary(h.resolve)`) only
    discards a `_resolve_legacy_result_*` pattern that such a body cannot contain, so it is not written.
  - A local written in a statement that reads it (`x = cast(T, x)`) does not count as earlier.
  - When a payload has more grandfathered entries than sites, the extra entries are not listed as stale.
- **eager_resolve: the grandfathered directory** is read with `IO=lines, dir=…`. The engine reader lowers it, but
  rrlab does not model `dir=` and warns that the input is missing. The directory is empty at the pinned commit, and
  the empty list's signature verifies.

## Harness gaps (tools/rrlab)

`dl.py` now reads `code` or `rule`, compares `compare_fields` events on their keys and reads every recorded shape.
The remaining gaps:
- events with no files cannot be given a verifier file;
- `.scope` is ignored on direct calls;
- `artifact=` and `dir=` inputs are not modelled;
- monkeypatched module globals (eager_resolve's allowlist) cannot be expressed;
- an entry that never reports is compared on every code;
- direct calls whose recorded code is the message cannot be mapped to the plugin's code.

## Change requests

1. **RUN-FACTS.md, section 7 (plan_composition_probes).** It says the producer prints violations. In fact the
   linter's probe prints observations, and the rule decides. Replace the section's description of the output with:

   > The producer runs the live composition probe and writes one JSON object:
   > `{"fingerprints": [...], "first_refs": [...], "second_refs": [...]}`. `fingerprints` is the plan fingerprint
   > of each composed run, and `first_refs` and `second_refs` are the ref ids each fragment contributed. The rule
   > reports SPC-012 when two fingerprints differ and SPC-005 when a ref id is in both lists.

2. **tools/rrlab/dl.py:** a way to score direct-call records whose code is the message against a one-code rule.
   For example, `--code-from-message CODE` would treat any recorded code that is not a declared code of the rule as
   CODE.

No change to `lib/core.rr` or to an existing library is needed.

## Not expressible

Nothing in these four checks. The remaining misses are harness limits or recording shapes, not language limits.
