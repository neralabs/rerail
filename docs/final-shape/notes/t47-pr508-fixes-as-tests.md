# t47. PR #508's fixes as tests on the hub

## Intro

PR #508 (`tbelanger/final-rerail-shape-integration`) brings the final-shape engine onto `main`. It merged this hub
at 38de57032 and fixed what it found, in three rounds. Since then the hub has moved on, and many of those fixes
landed on code the hub has rewritten since (0276 steps 5–6: the version is the only path, the tree is gone).
The PR ships first; this hub ships later as its own PR.

So before the hub merges the PR, each of Tim's changes that conflicts with the hub becomes a test here. Run against
today's hub, a test either passes (the hub already has the behaviour, in its own form) or fails (the fix still has
to be ported). A failing test is kept, ignored with its reason and a work-queue item, until the fix lands. Either
way, none of these fixes can regress by the time this hub becomes a PR.

A test keeps Tim's input and his expected result. Where his compares the version with the tree's answer, the
port states the tree's answer as the expected value, since the tree is gone here.

## Decisions (2026-11-18)

- The hub stays on Rust 1.99.1, not the PR's 1.95.0.
- The PR ships store v5; the hub is v6 or later.
- rustfmt is run over this tree and every worktree later, not now.
- `main` takes the PR squashed; the hub re-anchors after it.
- Windows (0401's mapped files, open-file limits) comes last.
- A Mac without Metal runs on the CPU, and `--metal` fails there.
- Tapes stay on format 16 and what we add to it.

## Classes

- **Engine:** the engine's behaviour. A test each, on the hub's code path.
- **Shipped path:** what W's matcher and the shipped tape do (gap tests, implied tokens, walk budget). The hub must
  keep the behaviour on format 16: a test each.
- **Portability:** Windows, Linux and x86-64. Tests come with the Windows work, at the end.
- **Mechanical:** clippy, rustfmt, the rename, re-blessing, the data move, CI. Taken by the merge or not at all;
  no test.

## The ledger

Status: `pending` (no test yet), `passes` (the hub has it), `fails` (ignored here, queue item named), `n/a`.

| Commit | Class | Change | Tim's test | Hub test | Status |
|---|---|---|---|---|---|
| 62eff0866 | mechanical | Build: no library crate names its checkout again (decision 0224) | — |  | n/a |
| 0206654a5 | engine | Read a symlinked corpus root's real part before the waves too | the_run_after_a_cold_one_through_a_symlink_writes_nothing | `corpus::tests::the_run_after_a_cold_one_through_a_symlink_writes_nothing` (new; `cold_then_warm` split out) | passes: ported, with the observer sweep keeping both names of a symlinked root (same cursor) |
| 401a3eb06 | engine | corpus tests: the cold/warm test's warm run is started beside another test's switch too | — | `the_run_after_a_cold_one_writes_nothing` (the hub's own) | fails: Q243 |
| ba4eabe9e | engine | switches: a test starts a process of the engine holding the switches, as a run holds them | — | `a_second_corpus_run_deletes_and_creates_its_store_again`, `cross_file_rows_are_memoised_per_import_component`, `git_seeing_no_file_under_the_root_is_kept` (the hub's own) | passes (some after the guard fixes below) |
| b9a4f92d1 | mechanical | What names the rule-file extension says `.rerail`: includes, the registry, the engine, the CLI | — |  | taken with the merge (`02-rewrite.py` over the hub's new names) |
| 7c74fbbf1 | mechanical | Rule files are named `.rerail`, were `.rr`: renames only | — |  | taken with the merge |
| db14816c8 | mechanical | rustfmt: the lines this branch changed in the listed files, as check-rustfmt lays them out | — |  | rustfmt: later, across every worktree |
| 66ff4e437 | shipped path | tape: a walk (dfs) leaves its nodes' parents to the kept parents; a node's text, children and name inline into the match | — |  | n/a: performance, no behaviour to pin |
| 64fc5a1b6 | shipped path | compile: the token tests' walk borrows the rule, a gap test refuses a relation before copying it; budget polls and tape | — |  | n/a: performance, no behaviour to pin |
| 26bf95bbe | engine | corpus: the prepared program's tree is mutable only where the debug check edits it | — |  | n/a: the tree it guards is gone (0276 step 6) |
| e7ed8180c | shipped path | tape: a node read of format 15 inlines into its caller; Tape::path, unused since the kept parents, removed | — |  | n/a: format 15 (the hub reads format 16) |
| 765ed3f2e | shipped path | tape: the matcher reads a parent no step knew from the document's kept parents; a tape lookup searches from the root by | a_node_reached_without_its_parent_reads_the_kept_parents | `tape::tests::a_node_reached_without_its_parent_reads_the_kept_parents` (new; format 15's byte count left out) | passes |
| 5b0f9b550 | shipped path | tape, budget: a node's kind and the matcher's call poll inline into the matcher's crate | — |  | n/a: performance, no behaviour to pin |
| d120d80c8 | mechanical | Engine: format tape/tests.rs after the merge of the hub's #735 tables | — |  | rustfmt: later |
| 7410a263b | shipped path | `using` as a name: format 16's ECMAScript tables and the lean parser's tokens written again for #735's grammars | using_as_a_name_reads_without_an_error_on_the_products_path | `tape::tests::using_as_a_name_reads_without_an_error_on_the_products_path` (new) | fails: Q245 |
| 9036be38c | engine | Rules: rerail rules test on a .rr file asks for a checkout, whose .rr programs and library its tests read | test/cli/rule-draft.test.ts |  | pending: TS CLI test (`test/cli/rule-draft.test.ts`), Q248 |
| 66c7f997d | mechanical | Tests: the structure gate passes on the engine line, with its kept records and engine layout as named exemptions | — |  | n/a |
| 5e0b69a10 | mechanical | Tests: the data-layer port's recordings, rule corpus and measurement records leave git for S3 pack test data | — |  | taken with the merge; tests that read the moved data fetch it |
| 9cc6eb22f | engine | Rules: rerail rules test on a .rr file says the .rr library is missing outside a checkout, and its usage names .rr | test/cli/rule-draft.test.ts |  | pending: TS CLI test (`test/cli/rule-draft.test.ts`), Q248 |
| ed1872a09 | engine | Uninstall: --delete-data deletes the folders and locks the engine keeps beside the content store | test/cli/setup-cli.test.ts |  | pending: TS CLI test (`test/cli/setup-cli.test.ts`), Q248 |
| 34fac0909 | mechanical | Engine: format main's Rust files the merges left unformatted | — |  | rustfmt: later, across every worktree |
| ec8d270f3 | engine | setfix rows: a frozen relation of no rows keeps its width | rows_started_from_a_kept_relation_of_none_hold_their_own | `facts::rows::tests::a_relation_started_from_an_empty_one_keeps_its_rows` (the hub's own) | passes |
| 2866ab672 | engine | packed_batch tests: the test batch's header counts its node, as a result's file is checked against it | — | `compact_result_page_*` (the hub's own) | passes (some after the guard fixes below) |
| c9c3d41cc | engine | Ladder: tier 1's contract takes a coded lazy mark's hit, and a file sent to the fallback owes its totals alone | — | `rr_steps_parse_as_the_reference_and_the_engine` (the hub's own) | passes (some after the guard fixes below) |
| c80b57f39 | engine | MergeIdentical on the version keeps a request relation its own, as the tree's merge does | a_request_relation_keeps_its_own_on_a_version_as_on_the_tree | `preparation::merge::tests::a_request_relation_keeps_its_own_on_a_version_as_on_the_tree` (new) | passes |
| 23a7bb8f1 | mechanical | The merged tree builds and is clippy clean on Linux under Rust 1.95.0, both CI commands, the hub's new code included | — |  | n/a |
| 751f92ac0 | engine | rr tests: an input is read by an atom of a rule (or a native), not by a string equal to its name | — | `dropping_unreached_clauses_prunes_to_the_same_program*` (the hub's own) | passes (slow tier: `dropping_unreached_clauses_prunes_to_the_same_program`) |
| 5420af2ef | engine | Guards: the hub's segment tables and new state listed under 0104, and an arm count is no tier 2 table read | — | `every_kept_map_is_state_or_listed`, `every_table_is_facts_or_listed`, `only_the_compiler_reads_tier2s_states` (guards) | passes (some after the guard fixes below) |
| aff7892a2 | mechanical | The merged tree is clippy clean under both CI commands, and its tests build for Windows | — |  | n/a |
| 4f9110496 | engine | phase 0: a graph kept as an image names no plans, so the run after a cold one records nothing | — | `the_run_after_a_cold_one_writes_nothing`, `a_second_run_in_one_process_records_the_first_runs_manifest` (the hub's own) | passes: the hub's cause fixed instead (an empty prepared set is not new); images keep their plans (Q150) |
| 3d3cea4a7 | mechanical | The merged tree builds its tests and is clippy clean on Linux under Rust 1.95.0, both CI commands | — |  | n/a |
| d87ef6159 | engine | phase 0: a seed whose kept graph no longer holds is derived again, not taken back from the kept map | — | `programs_are_kept_by_the_hashes_of_what_they_read` (the hub's own) | passes (some after the guard fixes below) |
| cb7b3b76e | engine | Ladder: a file tier 1 sends to the fallback owes tier 2 its totals alone | — | `rr_steps_parse_as_the_reference_and_the_engine` (the hub's own) | passes (some after the guard fixes below) |
| 53361e039 | engine | Parser tests: the ladder's ids fit the tape, tier 1 kept to 0177's contract, tests re-blessed | a_call_nothing_reaches_is_not_counted | needs `clif_text::reachable_calls`: the test lands with the port | pending: Q248 |
| 0d1f9497e | engine | Tier 2's lazy marks take the token's Next and Start from their caller | a_lazy_marks_token_is_its_callers | `rerail-rr-parser::tests::a_lazy_marks_token_is_its_callers` (new) | passes |
| c424072db | engine | parser_tests: tier 2's table guard takes a reader indexed in the records' readers for a token reader's arms | — | `only_the_compiler_reads_tier2s_states` (guard) | passes (some after the guard fixes below) |
| 2ba469012 | engine | Tests: the logic crate's guards list the hub's state, and Git's kept answer is asked only where a journal runs | — | `every_kept_map_is_state_or_listed`, `every_thread_local_is_listed_with_its_class`, `git_seeing_no_file_under_the_root_is_kept` | passes (some after the guard fixes below) |
| 370b90f9c | engine | A pack holds the programs that ask for the same guests: one program's parsed annotations are no other's | a_policy_reads_in_a_pack_only_the_guests_it_asks_for_alone | `rr::tape_tests::a_policy_reads_in_a_pack_only_the_guests_it_asks_for_alone` (new) | passes: ported (`rr::guest_asks`; a group shares its guest requests) |
| 4146acbd9 | engine | A pack's program reads its library's spans and relations of kinds as its run alone does | a_match_over_a_library_relations_node_finds_in_a_pack_what_it_finds_alone, a_programs_nearest_over_a_library_relation_of_kinds_finds_in_a_pack_what_it_finds_alone | `corpus::tests::{a_match_over_a_library_relations_node_finds_in_a_pack_what_it_finds_alone, a_programs_nearest_over_a_library_relation_of_kinds_finds_in_a_pack_what_it_finds_alone}` (new) | passes: ported (versions: `SpanPatterns` given `Library::spans`, `kinds_of` over the library) |
| 4937ec0eb | mechanical | file_local tests: the branded-atom check reads `contains`, as clippy 1.95 asks | — |  | n/a |
| afeb076e5 | engine | A pack's program reads its library's kind sets with their kinds | a_programs_nearest_over_a_library_kind_set_finds_in_a_pack_what_it_finds_alone | `corpus::tests::a_programs_nearest_over_a_library_kind_set_finds_in_a_pack_what_it_finds_alone` (new) | passes: ported (`KindSets` given the library; the merge's `library_sets`) |
| 184b78e1c | engine | RERAIL_VERSION_TRACE is classified: it changes no program | — | `every_switch_lowering_reads_is_classified` (guard) | passes (some after the guard fixes below) |
| 4c2ce6f8a | engine | Locate on the version moves an unlocated relation cross-file, round after round, as the tree's locate does (0290) | a_relation_left_unlocated_moves_cross_file_on_a_version_as_on_the_tree | `typing::file_local::tests::a_relation_naming_no_document_moves_to_the_cross_file_part` (the hub's own) | passes |
| eb4a92d28 | engine | simplify: reads of a relation the stated facts emptied are decided, and a request's copy equal to it is the request | a_requests_copy_with_its_own_rules_is_read_as_the_request, reads_of_a_relation_the_stated_facts_emptied_are_decided | `preparation::simplify::tests::{a_requests_copy_with_its_own_rules_is_read_as_the_request, reads_of_a_relation_the_stated_facts_emptied_are_decided}` (new, on the version) | passes |
| 218e24cc1 | engine | rr::lower: a request's demands are undone with it, named by the reader, and kept only with their readers | — | `rr::tests::dropping_unreached_clauses_prunes_to_the_same_program*`, `lowering_is_idempotent_*` (the hub's own) | passes (slow tier, run with `--ignore-default-filter`: `dropping_unreached_clauses_prunes_to_the_same_program`, `lowering_is_idempotent_data_layer`) |
| 363b91c0b | engine | A pack's programs read their library as each read alone does: kind sets, bounded cycles, stated facts | a_librarys_kind_set_is_a_relation_its_includers_read_in_a_pack, a_programs_facts_of_a_library_relation_ask_of_the_library, a_value_cycle_a_library_relation_bounds_is_bounded_in_a_pack | `corpus::tests::{a_librarys_kind_set_is_a_relation_its_includers_read_in_a_pack, a_programs_facts_of_a_library_relation_ask_of_the_library, a_value_cycle_a_library_relation_bounds_is_bounded_in_a_pack}` (new) | passes: ported (versions: `lower::keep_declared_kind_sets`, `cycles::check_given`, facts ask) |
| 1999e204e | engine | run-policies: a batch's policies of a language of several dialects run as one pack a dialect (0167) | a_batchs_policies_of_several_dialects_run_as_one_pack_a_dialect | `rr::tape_tests::a_batchs_policies_of_several_dialects_run_as_one_pack_a_dialect` (new) | passes: ported (`policy_groups` by dialect, `run_pack` once a dialect) |
| 62b554c01 | engine | memo packs: a test's store is copied with its packs, and a pack whose file is gone is not begun again | a_store_copied_without_its_packs_reads_no_other_entrys_bytes, a_template_is_published_and_copied_with_its_memo_packs | `content_store::frames::tests::a_store_copied_without_its_packs_reads_no_other_entrys_bytes`, `test_store::tests::a_template_is_published_and_copied_with_its_memo_packs` (new; the hub copies `FOLDERS`) | passes |
| a6146af5b | mechanical | Clippy clean on Linux under Rust 1.95.0, both CI commands, the hub's new code included | — |  | n/a |
| 70026ed6b | engine | corpus: a memo hit known by its digest alone names its source in the run's manifest | — | `the_run_after_a_cold_one_writes_nothing`, `a_second_run_in_one_process_records_the_first_runs_manifest` (the hub's own) | passes: the hub names digest-known reads already (`read_unstored`, `memo::named_digests`) |
| a44e11ce2 | portability | deferrable slabs build on Linux: one MS_SYNC writes a slab back there | — |  | pending: portability, last (Q248) |
| 3e95617df | engine | text readers guard: upkeep's finish times and format 16's tables are allowed | — | `no_rule_or_fact_is_read_from_text` (guard) | passes (some after the guard fixes below) |
| 652c47351 | engine | blocks: the GPU join runs on the process's one device and queue | — | `executors_are_shared` (the hub's own) | passes (some after the guard fixes below) |
| 4120b372e | engine | memo: a string's own rows are no file's text when a chunk attributes its base rows | — | `value_types_are_ws_value_types` (the hub's own) | passes (some after the guard fixes below) |
| 0a02302f6 | engine | text readers guard: coverage.rs reads its ledger twice | — | `no_rule_or_fact_is_read_from_text` (guard) | passes (some after the guard fixes below) |
| 4a48620a1 | portability | blocks: an anonymous region is reserved address space on Linux, as on macOS (MAP_NORESERVE) | — |  | pending: portability, last (Q248) |
| a60a79dcc | mechanical | CI: the integration branch stays on Rust 1.95.0 | — |  | not taken: the hub stays on 1.99.1 |
| a462ad9d0 | engine | What the merged tree's tests found: a producer's stratum, main's Java kit, the identity test, two ledgers | — | `producer_reads_of_projection_views_keep_the_projection_gate` (the hub's own) | passes (some after the guard fixes below) |
| ed4d6d1f3 | engine | Capabilities name the hub's two guest languages, as the registry supports them | — |  | ported here: `capabilities.json` names `python-format` and `eslint-directive`; the TS test `code-rule-schema.test.ts` holds it (its comparison checked by script) |
| 22aac631c | mechanical | Clippy clean on Linux under Rust 1.99.1, both CI commands, the hub's code included | — |  | n/a |
| 3b83172e5 | portability | Windows: rerail-blocks maps its memory with Windows' calls, and the hub's tests read the portable thread clock | — |  | pending: portability, last (Q248) |
| e4b20ff10 | portability | JSON on MSVC: the Bitlex parser and tree-sitter-json, as simdjson's route reads files | — |  | pending: portability, last (Q248) |
| adfb49fac | engine | Revert the ports of 0201 §5 and §6 (00eae7eaa9, c6c3bb0680): ruff-ruf013 loses every finding with them | — | `rr::tape_tests::ruff_ruf013_finds_an_implicit_optional` (new) | passes: the hub's cause fixed instead (a guest request's context keeps a disjunction of membership tests) |
| 3fe6f8227 | engine | A negated conjunction binding the rule's variables in untied parts is bound by the rule's context | a_negated_cross_product_is_bound_by_the_rules_context, alternatives_past_the_bound_are_a_helper_relation, an_aggregates_context_reads_the_clauses_demand | `rr::tests::a_negated_cross_product_is_bound_by_the_rules_context` (new; the 0201 tests went with adfb49fac) | passes: ported (`cross_product` in `negated_conjunction`) |
| c6c3bb068 | engine | An rr context may read the clause's demand: a derived atom over the head's variables, or the clause's first atom (decisi | — |  | n/a: reverted by adfb49fac (Q247) |
| 00eae7eaa | engine | rr lowering stays bounded: a disjunction past 16 alternatives is a helper relation, within_stop is two alternatives (dec | — |  | n/a: reverted by adfb49fac (Q247) |
| 02439e914 | engine | flatten --lowered prints the program run-rr runs, lowered: its size is the lowering's | — |  | n/a: a developer tool's option (`flatten --lowered`), taken with the merge |
| 7529228ce | shipped path | relation: a sibling walk spends a matcher call for each implied token it passes, as a walk over the full tree does (RER- | a_sibling_walk_spends_what_a_walk_over_the_full_tree_does | needs `budget::matcher_calls`: the test lands with the port | pending: Q248 |
| 2021ce4e6 | portability | phase0: an include's file is named as its module is loaded, so its own includes are found on Windows | an_include_is_named_as_its_module_is_loaded |  | pending: portability, last (Q248) |
| c30f4c153 | portability | Windows: canonical paths carry no `\?\` prefix (dunce), and the cost store holds no connection between writes | — |  | pending: portability, last (Q248) |
| fd3f6bd03 | portability | Windows: the engine's lib and tests compile there; Unix calls behind cfg(unix) or a portable helper | — |  | pending: portability, last (Q248) |
| 43b572e44 | shipped path | nearest: the walk down a subtree goes by parent lookups, not over node_order's keys (0121's keys are not dense) | nearest_descends_across_granule_boundaries_as_the_climb_finds | `sql_executor::tests::scaling::nearest_descends_across_granule_boundaries_as_the_climb_finds` (new) | passes |
| 1fa56df8c | portability | stream: tier 2's own functions return through a return area on x86-64 | — |  | pending: portability, last (Q248) |
| b95d685a5 | mechanical | The merged hub clippy clean on Linux, both commands under -D warnings; tape_format on sha2 0.11 | — |  | n/a |
| 4cee65f06 | shipped path | Compiling a rule builds each regex and each pattern's grammar once per process | — |  | n/a: performance, no behaviour to pin |
| c16ead00f | mechanical | Pattern program digests, the RIR dumps and the pinned captures re-blessed for implied tokens at the end of children (RER | — |  | passes: blessed from the hub's engine (14 pattern digests, the RIR dumps, the captures pin = Tim's value) |
| 65ff68df5 | shipped path | Patterns: the end of children takes the implied tokens left out after the last matched child (RER-131) | ast_pattern_end_of_children_meets_implied_tokens, test_children_end_meets_implied_tokens | `packed::compile::matcher_tests::ast_pattern_end_of_children_meets_implied_tokens` (new); the vendored half needs `tokenless_matched` | passes: ported (`Doc::implied_after`, `Slotted::children`; the tape's `at(index, up)`) |
| c00a1d9f0 | shipped path | Engine: implied tokens' places — gap-test captures, stored-neighbour walks, token comparisons, partly kept gaps and sour | — |  | pending: with RER-131 (Q246) |
| eabd93c69 | shipped path | Patterns: an ellipsis before the end of children leaves the implied tokens the candidate keeps (RER-131) | — | `may_match_ellipsis*` (vendored ast-grep-core) | n/a: 83b741861's code again, with a TS test |
| e1e0209ae | shipped path | Fact rows: a parent's gaps read once per file, and the CPU's files sweep in parallel | — | `the_backends_find_the_same_keys_on_real_files` (the hub's own) | fails on the farm for want of data: Q248 |
| 83b741861 | shipped path | Patterns: an ellipsis before the end of children leaves the implied tokens the candidate keeps (RER-131) | — | `may_match_ellipsis*` (vendored ast-grep-core) | ported with 65ff68df5 (its `may_match_ellipsis*` tests are the vendored crate's, not run in the workspace) |
| 48bf01694 | shipped path | Gap tests answer a token's own neighbours, a token on the way to a child, and counts of tokens (t31) | — |  | pending: with RER-131 (Q246) |
| 241c19955 | shipped path | Gap tests answer a token's own neighbours, a token on the way to a child, and counts of tokens (t31) | — |  | pending: with RER-131 (Q246) |
| acc0c3667 | engine | test store: collect the template before publishing it | — |  | passes (`a_tests_store_is_a_copy_of_the_template_and_goes_with_its_test`) |
| 329c3a1ec | shipped path | S1 gate: W's matcher reads a file's scope table once, and the fixtures run in parallel | — |  | n/a: performance, no behaviour to pin |
| 22860768f | engine | plan: a run behind the run takes the profile only when no one holds it | — | `projection_reason_traces_real_consumers_and_ignores_dead_scalars` (the hub's own) | passes (some after the guard fixes below) |
| bb56a0815 | mechanical | Engine: clippy clean across the remaining engine sources and tests; no behaviour change | — |  | n/a |
| d4e841f10 | mechanical | logic: clippy clean (no-op drops removed, guards held as Any, iterators for index loops); no behaviour change | — |  | n/a |
| 035b04088 | mechanical | store: clippy clean (item allows for tuple types and id counts, test-only helpers under cfg(test)) | — |  | n/a |
| a4454ba10 | mechanical | compiler, parser tests, prof, wasm JIT: clippy clean on Linux (iterators for index loops, cfg-scoped allows for macOS-on | — |  | n/a |
| 5fc16438a | mechanical | Engine: the facade's re-exports and the modules mounted for tests alone are allowed what the engine's own build does not | — |  | n/a |
| 7d398d48b | portability | Tests: the watchdog's and the journal's on Linux | — |  | pending: portability, last (Q248) |
| 4bd012007 | engine | plan: a run waits for the last run's connections closing beside it | — | Linux only (`/proc/self/fd`): with the portability work | pending: Linux only, with the portability work |
| 2de288fc0 | engine | profile: a run's waves are the profile's only when it holds the profile | — | `explain_shows_engines_plans_devices_and_decisions` (the hub's own) | passes (some after the guard fixes below) |
| e946ff720 | engine | plan: the coverage gate takes the device column from its ledger where there is no Metal device | — | `every_operation_runs_on_every_executor`, `every_operation_a_shipped_rule_uses_has_a_probe` (a Mac without Metal) | passes (some after the guard fixes below) |
| 948c8ad05 | engine | phase0: a seed's graph leaves out the implicit core.rr of a module the run was also given as a file | — | `bootstrap_resolves_as_the_builtin_reader_does` (the hub's own) | passes (some after the guard fixes below) |
| 47fae7350 | mechanical | Engine: clippy's suggested fixes across the merged crates (let chains, needless borrows and casts, repeated allows); no | — |  | n/a |
| d9d081d56 | mechanical | accel: clippy clean on Linux (the cost model's Sample is macOS's, type aliases, let chains) | — |  | n/a |
| 0085298f4 | portability | tier 1: the shape checks read calls and jumps in the host's machine code | — |  | pending: portability, last (Q248) |
| 2f6a5b9d8 | portability | stream: tier 2 on x86-64: the unsigned ISA only on arm64 macOS, frame pointers for tail calls | — |  | pending: portability, last (Q248) |
| 66e167a15 | mechanical | The merged tree builds: the workspace's test binaries compile on Linux | — |  | n/a |
| 87ed02c4e | portability | repeat's CPU readings on Windows: the process's user and kernel times | — |  | pending: portability, last (Q248) |
| d9c403ebf | engine | A store created again at its path has a writer of its own | a_store_created_again_at_its_path_has_its_own_writer | `content_store::write_back::tests::a_store_created_again_at_its_path_has_its_own_writer` (new) | passes |

## The second run

The guard fixes pass (`every_switch_lowering_reads_is_classified`, `every_kept_map_is_state_or_listed` with
`NOT_YET_MAPS_PIN` lowered to 22, `every_table_is_facts_or_listed`), and so do the slow tier's two smallest of
218e24cc1's families and acc0c3667's store test. The per-pack families of `dropping_unreached_clauses_*` and
`lowering_is_idempotent_*` are left for a phase gate's full run.
