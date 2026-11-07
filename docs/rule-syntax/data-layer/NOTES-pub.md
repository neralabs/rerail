# NOTES: group `pub` (batches 7–8)

Nine checks, hand-written from the Python linters at `2b1504adc`. All nine parse and every clause lowers in the
engine's reader (`rr-census` prints nothing for any of them).

| Check | Python lines | `.rr` lines | rrlab dl (exact / events) | Reader |
|---|---|---|---|---|
| executor_publication_repairs | 147 | 20 (+34 library) | 5 / 5 | lowers |
| revision_ancestry_invariants | 201 | 47 | 5 / 6 | lowers |
| publication_registry_writes | 271 | 38 | 24 / 24 | lowers |
| unknown_contract_defaults | 297 | 59 | 30 / 30 | lowers |
| grammar_mutation_invalidation | 303 | 86 | 45 / 45 | lowers |
| declared_fact_interrogation | 321 | 64 | 42 / 42 | lowers |
| history_serverplan_projection | 380 | 59 | 12 / 12 | lowers |
| versioned_merged_identity | 383 | 109 | 10 / 13 | lowers |
| post_op_side_effects | 1,329 | 235 | 18 / 22 | lowers |
| **total** | **3,632** | **717 + 34** | **191 / 199** | |

The tables (forbidden names, reviewed callers, fate overrides) are short, 4 to 16 entries, so they are inline facts
and not generated `.part.rr` files.

## Misses

**revision_ancestry_invariants, `probe:batch7:line-free-descriptor` (CLI vs plugin).** The linter has two front ends
that disagree, and the rule follows the suite plugin. The `violations()` CLI prints `forbidden_descriptor` and
`removed_surface` without a line, and the recording puts them at line 1. It also has no source trigger. The plugin
reports the real line, and runs only on a file that contains "revision", "ancestry" or "stmt_ancestry_chain". These
triggers are case-sensitive. The probe file holds only `class SqlRevisionAncestryDefinition` (capital letters), so
the plugin reports nothing and the CLI reports line 1. The plugin probe `production-descriptor-line` records the
same file with no findings, and the rule matches it.

**versioned_merged_identity, 3 misses, all harness gaps.**
- `test_unreviewed_constructor_fails_closed` and `test_aliased_constructor_is_still_an_unreviewed_constructor`
  pass `expectedCallers: []`. An empty list option gives no `option` row, so the rule cannot tell it from an absent
  option. It falls back to the twelve default callers, and reports VMI001 (one key on the steps directory). See the
  change request below.
- `test_missing_reviewed_constructor_fails_closed` has no files. rrlab does not run an event without files, but
  VMI001 is a repository-level finding ("a reviewed caller has no constructor"). On any file the rule reports it, at
  the recorded key (`impl/common/l2070_server_plan/steps`, line 1).

**versioned_merged_identity, fixed after review:**
- A constructor in a decorator, a default or a base belongs to that def or class, as on the linter's scope stack:
  vmi_caller and vmi_known use match-frames' mf_up and mf_def. `@deco(SqlVersionedMergedCte(…))` on an expected
  `def build()` is now clean; before, it gave VMI002 with caller `dec.py:` and VMI001. The scope chain above the
  def stays on `nearest`, because no def or class sits inside a decorator. The reader also refuses a value-building
  recursion through mf_up (UnboundedValueCycle).
- `(SqlVersionedMergedCte)(…)` is a constructor: vmi_callee takes parentheses off.
- vmi_backed is one recursive relation (a builder call, or a name whose known value is backed). VMI003 is a
  reviewed constructor with no backed read_projection.
- Duplicate file names. The plugin reports each finding at the last path recorded for its caller, and both roots
  can hold a `dup.py` with the same `build`. The linter then gives one key, where the rule gave two.
  vmi_report_path now puts them on the l9700_sql_graph_query file, taking the files in PRODUCTION_ROOTS order. That
  order is how `source_files` lists them; that `view.files` keeps it is unconfirmed.
- A walrus binding (`if (p := catalog…(…))`) is not an assignment to the linter (only Assign and AnnAssign are
  recorded), and the rule agrees: VMI003.

**grammar_mutation_invalidation, fixed after review:** `.scope tests false` replaces `exclude "**/tests/**"`, which
let `test/` directories through. 45 / 45 still.

**post_op_side_effects: 4 misses, entry emulations.** `test_runtime_allowlist_governance_rejects_boolean_opt_out` calls
`_collect_runtime_allowlist_bypass_violations_from_tree` on `input.py`. The rule reads the bypass only in
`l2070_batching/post_op_allowlist.py`, and reports it at line 1 of post_op.py, as the plugin does. This is an
entry-emulation miss. The other three are the bare-entry-list events below. (rrlab now verifies Ed25519 with
libsodium; before that the stub failed every signature, and an openssl check of the rebuilt messages agreed.)

**post_op_side_effects, fixed after review:**
- POSTOP000 is now gated on "no other plugin code fired" (`pos_any`). The earlier gate also asked for an empty
  registry and no reachable symbols, so renaming PostOpKind and PostOpBehavior in post_op_specs.py gave nothing
  instead of POSTOP000.
- Extraction failures are modelled (`pos_allow_broken`). The plugin's allowlist reader raises when the first
  SIGNED_POST_OP_SYMBOLS is not a set, list or tuple of string literals, or when there is no annotated
  `ALLOWLIST_SIGNATURE: str = …`. The fact engine then reports FACT-EXTRACTION-FAILED at line 1 of the allowlist,
  the registry reads as empty and unsigned, and the file's POSTOP203 checks are dropped.
  - `ENFORCE_… = True` with `frozenset({OTHER} | {…})` now gives 202, 205 and FACT-EXTRACTION-FAILED, and no 203.
  - An unannotated `ALLOWLIST_SIGNATURE = "…"` now gives 202, 205 and FACT-EXTRACTION-FAILED. The review listed only
    {202, 205}, but the signature reader raises there too, and `_fact_extraction_failures` reports that.
- The tests call the registry checks with bare entry lists. The recording writes those as a SIGNED_POST_OP_SYMBOLS
  file with no signature, which reads as broken: FACT-EXTRACTION-FAILED, and no POSTOP204/205/206 from the entries.
  Three events (`_collect_broad_runtime_authority_violations`, `_collect_exact_runtime_symbol_coverage_violations`
  ×2) miss for that reason. They are entry emulations: a rule must not change what it reports by a test option, so
  the coordinator removed a `pos_entries_only` fact keyed on `entryPoint` that kept them exact.
- Only the first module-level `def assert_post_op_symbol_signed` is the verifier, as in the linter.
- Idiom: the two ALLOWLIST_SIGNATURE readings are one clause over signing.rr's `unsigned_sig`, and `pos_dataclass`
  uses python/names' `decorator`.

## Deliberate choices

- **post_op_side_effects follows the plugin.** The CLI's `_collect_violations(overrides)` trusts the overrides without
  verifying the fate signature; the plugin verifies it. The signed values that live in the linter's own source
  (POST_OP_FATE_OVERRIDES, POST_OP_FATES_SIGNATURE, POST_OP_SURFACE_SIGNATURE) are facts in the rule. The options
  `fateOverrides`, `fatesSignature` and `surfaceSignature` replace them, which is how the tests patch them.
- **post_op_side_effects: the taint visitor is not ported.** `_RuntimeOpBehaviorVisitor` (POSTOP103–113) is never run
  on the repository: neither `_collect_violations` nor the plugin calls it, and only unit tests feed it snippets. No
  event records it.
- **post_op_side_effects: base64 is decoded leniently.** Python's `base64.b64decode` drops characters outside the
  alphabet, so `"…==\n\t!!!"` verifies. The rule strips them before verifying (`@replace_all(s, "[^A-Za-z0-9+/=]", "")`).
- **executor_publication_repairs numbers lines as `str.splitlines()` does,** through the new `pub-lines.rr`. A probe
  has U+2028 inside a line; core's `src_line` splits only at `\n`, `\r\n` and `\r`.
- **revision_ancestry_invariants and history_serverplan_projection count lines as `line` does,** for regex matches
  over the file. That is `\n`, `\r\n` and `\r`, while the linter counts `\n` only. They differ only on a file that
  contains a lone `\r`.
- **Source triggers.** The plugins read a file only if it contains one of their trigger strings. Where a finding
  already implies the trigger, it is left out. Where it does not, it is a test on the file text (`rai_text`,
  `dfi_file`, `vmi_file`, `pos_allow_read`).
- **post_op_no_internal_dispatch (the sibling check):** its vocabulary (enum-case comparisons, payload probes) does not
  apply here; nothing was shared.

## Vocabulary added

`lib/data-layer/pub-lines.rr` adds `py_line(d, n, s)`: line n of a document as `str.splitlines()` numbers it. It
also splits at `\v`, `\f`, `\x1c`–`\x1e`, `\x85`, U+2028 and U+2029. A test on the text of each line (the many
"read the file, enumerate its lines" scripts) needs this to match the scripts' line numbers.
- **Users:** executor_publication_repairs. rust_sql_grammar_boundary and serverplan_name_inference need the same
  numbering (group-2.md, `LIB source_line`).
- **Known difference:** a file whose last line ends in an extra separator and no `\n` gets one more line, which is
  empty.

## Library and core change requests

1. **core, options: an empty list option.** `option("expectedCallers", …)` gives one row per element, so `[]` and an
   absent option look the same, and a rule with a default list cannot honour an explicit empty one. The request is
   to add:
   ```
   // option_given("expectedCallers"): the policy sets this option, even to an empty list.
   .decl option_given(name: symbol)
   ```
   With it, `vmi_expected(c) :- vmi_default(c), !option_given("expectedCallers").`
2. **core, `@ed25519_ok`: say how `sig` is decoded.** Python's `b64decode` is lenient by default (it drops
   characters outside the alphabet), and the data layer's signatures rely on it. Proposed comment text:
   `// sig is decoded as strict base64; a rule that must accept what Python's b64decode(validate=False) accepts
   strips other characters first.`
3. **rrlab (no edit made): `min` over `match_all` captures.** `l = min k : { match_all("(?<m>x)", sp), line(m, k) }`
   gives 1 whatever the line. The same capture outside an aggregate gives the right line. revision_ancestry_invariants
   puts the matches in a relation first (`rai_chain_at`), which is also clearer.
4. **rrlab: `@ed25519_ok`.** Done: rrlab now verifies with libsodium.
5. **python/ast.rr, `ast_name` (changes POSTOP200). Done in bd0bb04d8.** It counted names that are strings in `ast`, and misses one Name.
   - It counts `case` captures (`case x`, `P(a=px)`, `… as q`, `*rest`, `**more`), the later parts of a
     dotted value pattern (`RED` in `Color.RED`), and PEP 695 type parameters (`T`, `*Ts`, `**P`).
   - It misses the walrus target (`w` in `(w := 1)`), which nameref leaves out as a `name` field.

   Proposed text, to add after `not_name`. It was checked on a probe: only `v`, `Point`, `Color` and `w` remain
   Names, as in `ast`.
   ```
   // `case` captures are strings in ast (MatchAs, MatchStar, MatchMapping.rest), not Names: `case x`,
   // `case P(a=px)`, `case [*rest] as q`, `{**more}`. A one-part dotted name is a capture unless it is a class
   // pattern's class.
   .decl py_capture(n: node)
   .decl py_pattern_class(d: node)
   py_pattern_class(d) :- kind(c, "class_pattern"), child(c, d, 0).
   py_capture(n) :- kind(d, "dotted_name"), child(d, n, 0), !child(d, _, 1), inside(d, _, "case_pattern"),
       !py_pattern_class(d).
   py_capture(n) :- (kind(p, "as_pattern") ; kind(p, "splat_pattern")), inside(p, _, "case_pattern"), child(p, n, _),
       kind(n, "identifier").
   not_name(n) :- py_capture(n).
   // In a dotted value pattern `Color.RED` only the first part is a Name; the rest are attributes.
   not_name(n) :- kind(d, "dotted_name"), inside(d, _, "case_pattern"), child(d, n, i), i > 0.
   // PEP 695 type parameters hold their name as a string (TypeVar(name='T')).
   not_name(n) :- type_param(_, _, n, _, _, _, _).
   // A walrus target is a Name (ctx Store); nameref skips it as a `name` field.
   ast_name(n, x) :- kind(w, "named_expression"), field(w, "name", n), text(n, x).
   ```
6. **check_rr (no edit made): undefined relations.** A rule that used `call_arg` without including python/calls
   passed check_rr, and Soufflé then refused it ("Undefined relation"). A check that every body atom is declared
   somewhere would catch this before rrlab.
