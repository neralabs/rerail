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
| versioned_merged_identity | 383 | 100 | 10 / 13 | lowers |
| post_op_side_effects | 1,329 | 214 | 7 / 22 as rrlab runs it; 21 / 22 with real signature checks | lowers |
| **total** | **3,632** | **687 + 34** | **180 / 199** (194 / 199 with real signature checks) | |

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

**post_op_side_effects: 15 misses come from rrlab's `@ed25519_ok`, which is a stub that always returns 0.** Every
signature fails, so POSTOP201, POSTOP202 and POSTOP002 (every member, because no override is trusted) are reported
everywhere. To check the rule itself, three scratch tools were used:
- a debug copy printed the three signed messages that the rule rebuilds for each event;
- `openssl pkeyutl -verify` checked them against the data layer's public key;
- a variant replaced `@ed25519_ok` with those verdicts.

Every message the repository signed verifies. The variant scores 21 / 22, including the permissive-base64,
stale-override and new-factory probes. One miss remains: `test_runtime_allowlist_governance_rejects_boolean_opt_out`
calls `_collect_runtime_allowlist_bypass_violations_from_tree` on `input.py`. The rule reads the bypass only in
`l2070_batching/post_op_allowlist.py`, and reports it at line 1 of post_op.py, as the plugin does. This is an
entry-emulation miss. The scratch tools are in the group's scratch directory, `pos/tool/verify.py` and
`pos/tool/mkvariant.py`.

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
4. **rrlab (no edit made): `@ed25519_ok` is a stub.** `openssl pkeyutl -verify -rawin` with the key wrapped as DER
   (`302a300506032b6570032100` + key) checks real signatures, and would make the signed checks score as they run.
5. **check_rr (no edit made): undefined relations.** A rule that used `call_arg` without including python/calls
   passed check_rr, and Soufflé then refused it ("Undefined relation"). A check that every body atom is declared
   somewhere would catch this before rrlab.
