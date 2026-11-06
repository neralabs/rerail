# Notes: the plan group

This group ports four checks from the data-layer repository's Python linters to `.rr`. The specification is
each linter at commit 2b1504adc. A score counts the recorded linter calls (events) whose finding keys the rule
reproduces exactly, using `rrlab dl` with `--jobs 2`. The last column says whether the engine's `.rr` reader
(`rr-census`) lowers every clause.

| check | Python lines | .rr lines | score (exact/events) | reader lowers it |
|---|---|---|---|---|
| plan_composition_architecture | 354 | 120 | 12/13 | yes |
| plan_helpers_allowlist | 607 | 157 + 35 generated | 75/82 | no: `ordered by` (below) |
| plan_optimizer_v2 | 1,032 | 240 | 159/166 | yes |
| eager_resolve | 2,666 | 484 + 115 generated | 72/77 | yes |
| total | 4,659 | 1,001 + 150 generated | 318/338 | |

Files:
- the rules: `plan-composition-architecture.rr`, `plan-helpers-allowlist.rr`, `plan-optimizer-v2.rr` and
  `eager-resolve.rr`;
- the generated tables: `plan-helpers-allowlist.tables.rr` and `eager-resolve.tables.rr`;
- their generators: `gen-plan-helpers-allowlist.py` and `gen-eager-resolve.py`. Both read the linter with
  `git show`, and `--check` prints ok when the committed table matches.

No library file was added: none of the vocabulary is shared by two checks.

The scores were measured with a scratch copy of `tools/rrlab/dl.py` that has three harness fixes, listed under
"Harness gaps". The current `dl.py` reads no events for eager_resolve or plan_helpers_allowlist, and scores
plan_composition_architecture at 3/13.

## The misses

**plan_composition_architecture, 1 miss.** `test_live_composition_positive` has no files. Its finding comes from
the run artifact `plan_composition_probes`. rrlab does not model `artifact=` inputs, so the event cannot run. I
checked the probe logic by reading the same observations through a `filename=` copy of the input.

**plan_helpers_allowlist, 7 misses.** All seven are `scan_repo` walk-skip events. They put a finding-shaped file
under a directory the linter does not walk (`tests`, `conform`, `docs`, `.venv`, `.git`, the terminals and
`data_layer_testing`) and expect nothing. The rule's `.scope dirs` and `.scope tests false` skip those files in
production, but rrlab ignores `.scope` on direct calls.

**plan_optimizer_v2, 7 misses.** All seven are missing-document events. They expect PV2-050 when
`docs/design/PLAN_OPTIMIZER_V2_INVARIANTS.md` is absent, and have no files at all, so the harness cannot run them.

**eager_resolve, 5 misses.**
- `test_signed_external_io_boundary_suppresses_owned_eager_db_terminals` monkeypatches the external-I/O allowlist
  to contain the test's symbol. The rule reads the real allowlist file, which a fixture cannot supply with a valid
  signature.
- The four `_scan_legacy_boundary_definition_ast` events call an entry that always returns `[]` ("legacy-boundary
  definitions are inventory"). No event of that entry has a finding, so dl compares it on every code, and the
  whole-linter rule reports transitive sites and EAGER-ALLOWLIST there. The rule is right; the comparison is not.

The BIG events are 11 real repository files, from 126 to 14,248 lines, and each expects `[]`. All of them pass, and
a direct `rrlab run` over the 11 files gives no finding except the ceiling signature, whose file was not in that
root. With the repository's real allowlist files added, the 34 external-I/O entries and the burn-down ceiling
verify, and a fabricated entry is reported.

## Reader refusals

`plan-helpers-allowlist.rr`, line 90: `not lowered yet: ordered on .decl pha_helper`. The linter keeps one map per
file from names to helpers. It writes the map in visit order, and only for values that are helpers: after
`a = grammar_row_lazy; a = other`, `a` is still a helper. So "what does `a` hold here" means "the latest earlier
write whose value is a helper". That recursion runs through `max` over `earlier` (the `sketches/owner-code.rr`
pattern) and needs `ordered by earlier`. I kept that form because it is the clearest statement of the semantics. A
form without it would need the reader's ordered recursion in another guise, or would give up the
"writes of anything else leave the map alone" rule.

No other clause is refused.

## Choices worth knowing

- **plan_helpers_allowlist: the message is the code.** The direct calls and the JSON port report each finding with
  its message as the code ("old or private H() call is forbidden"), while the plugin reports all of them as
  PHA001. The rule follows the recorded calls: `code {c} "{c}"`. Production would map them to PHA001.
- **plan_helpers_allowlist: PHA000** is a gate. When the signed allowlist does not verify, it is the only finding.
  rrlab now has a real `@ed25519_ok` when libsodium is installed, and the allowlist verifies, so PHA000 no longer
  fires in rrlab.
- **eager_resolve: message text.** For AST sites the linter's message ends with `ast.unparse(node)`. For transitive
  sites it ends with the first line of `ast.unparse(def)`. The rule uses the stripped source line in both cases.
  Only the keys are compared (the loose findings contract).
- **eager_resolve: approximations,** each outside every recorded event:
  - Annotation mentions (`LazyResult`, `Transaction`, `PlanningServerValueShape`) are any identifier in the
    annotation that is not an attribute's object and not inside a string. The linter recurses only through
    names, attributes, subscripts, `|` and tuples, so `Callable[[…], LazyResult]` is a mention here and not
    there.
  - "Contains an SQL-derived expression" looks at every descendant. The linter descends through a fixed list of
    expression kinds.
  - The lower/compile scan reads every def with a symbol. The linter reads only the last one (property getters and
    setters share a symbol).
  - The second wrapper shape (`h = x_lazy(…)` then `return run_in_legacy_execution_boundary(h.resolve)`) only
    discards a `_resolve_legacy_result_*` pattern that such a body cannot contain, so it is not written.
  - Grandfathered entries that are duplicated or attached to the wrong fragment, and ceiling duplicates, are not
    reported.
- **eager_resolve: the grandfathered directory** is read with `IO=lines, dir=…`. rrlab does not model `dir=` and
  warns that the input is missing. The directory is empty at the pinned commit, so nothing changes, and the empty
  list's signature verifies.

## Harness gaps (tools/rrlab)

The scratch copy of `dl.py` used for the scores differs from the current one in three places:

1. **complete-api records keep their code under `rule`** (and their path under `file_path`). The current `dl.py`
   gives such a record the check's name as its code, unless `--codeless` names another. This is what drops
   plan_composition_architecture to 3/13: its records carry SPC-001 to SPC-005 under `rule`. The fix is to read
   `f.get("code") or f.get("rule")`.
2. **`compare_fields` events are dropped.** That is all 82 plan_helpers_allowlist events and 78 of the 86
   eager_resolve events, so neither check has events under the current `dl.py`. Comparing these on keys only (path,
   line, code) is right for the loose contract. The scratch copy keeps them.
3. Exception records are skipped. The current `dl.py` already does this.

Gaps that stay in both copies:
- events with no files cannot run;
- `.scope` is ignored on direct calls;
- `artifact=` and `dir=` inputs are not modelled;
- monkeypatched module globals (eager_resolve's allowlist) cannot be expressed;
- an entry that never reports is compared on every code.

## Change requests

1. **RUN-FACTS.md, section 7 (plan_composition_probes).** It says the producer prints violations. In fact the
   linter's probe prints observations, and the rule decides. Replace the section's description of the output with:

   > The producer runs the live composition probe and writes one JSON object:
   > `{"fingerprints": [...], "first_refs": [...], "second_refs": [...]}`. `fingerprints` is the plan fingerprint
   > of each composed run, and `first_refs` and `second_refs` are the ref ids each fragment contributed. The rule
   > reports SPC-012 when two fingerprints differ and SPC-005 when a ref id is in both lists.

2. **tools/rrlab/dl.py**, gaps 1 and 2 above. In `events()`:
   - drop `"compare_fields"` from the skip test, so that it reads
     `if any(e.get(k) for k in ("compare_message",)) or e.get("path_skip_override"):`;
   - read the code as `f.get("code") or f.get("rule") or check`.

No change to `lib/core.rr` or to an existing library is needed.

## Not expressible

Nothing in these four checks. The remaining misses are harness limits, not language limits.
