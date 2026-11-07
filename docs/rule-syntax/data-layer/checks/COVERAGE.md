# Coverage: the eight analyses mapped onto the `.rr` design

Each row is one check from group-1.md … group-8.md, in group order. The elements column lists what answers the
check's non-EXISTS needs, using the IDs below; "—" means every need was EXISTS. Elements that answer only CLI-only
needs are left out: the suite plugin is the path followed. The design read is lib/core.rr, lib/python/ast.rr,
names/imports/calls/flow, lib/data-layer/signing.rr and the worked ports in data-layer/.

IDs: L5 `join`, L6 `ordered by earlier`, L7 `dfs_first` / `dfs_first_memo`; N-src (source, src_line, line_count, end_line, col), N-op (op_at), N-parse
(syntax_error), N-order (earlier, visit_index, ast_depth), N-str (str_part), N-fn:@name, N-git, N-read (`.input IO=…`,
input_error), N-artifact (a CI-written JSON artifact); P-ast:rel, P-lib:rel, P-sign (signing.rr); LOCAL (a helper in
the check's own file); D-place (`@place` findings), D-code, D-sev, D-gate, D-on, D-budget, D-group, D-scope, D-mode;
CONTRACT (the loose findings contract makes the need go away).

Conventions:
- A signed list or baseline in one of signing.rr's two layouts is P-sign. Another layout is LOCAL, built from L5,
  N-fn:@json_str and N-fn:@ed25519_ok, as signing.rr says a repository does.
- "One finding that replaces the report" is D-gate plus D-place. A signed-list failure reported beside the other
  findings is D-place alone.
- A parse error dropped silently is the contract's Unknown outcome, so it needs nothing. N-parse is listed only
  where the check reports or tests it. N-parse + D-on means it reports it as a finding.
- A message-only need (sorted joins in messages, `repr`, `unparse` previews) and two findings at one key are
  CONTRACT.
- Checks that run repository code get their facts from N-artifact.

| check | group | enforcement | design elements used | status |
|---|---|---|---|---|
| linter_enforcement | 1 | hard-fail | LOCAL, N-artifact, P-sign, L5, N-fn:@json_str, N-fn:@ed25519_ok, D-gate, D-place | expressible + artifact |
| standalone_linter_entrypoints | 1 | hard-fail | — | gap: composite policy over member policies (`.requires`) |
| transaction_closures | 1 | hard-fail | P-ast:sig_param, P-ast:def_frame, LOCAL, CONTRACT | expressible |
| transaction_patterns | 1 | ratchet | P-ast:sig_param, P-ast:def_frame, LOCAL, N-read, L5, N-fn:@json_str, N-fn:@ed25519_ok, D-code, D-mode | expressible |
| explicit_semantic_ids | 1 | hard-fail | LOCAL, N-artifact | expressible + artifact |
| transaction_callback_wrappers | 1 | hard-fail | P-ast:sig_param, P-ast:ann_root, P-ast:def_frame, P-ast:class_path, LOCAL, CONTRACT | expressible |
| plan_scope_ownership | 1 | hard-fail | P-ast:expr_ctx, P-ast:sig_param, N-parse, CONTRACT | expressible |
| embedding_vector_orchestration | 1 | hard-fail | P-ast:class_path | expressible |
| hnsw_architecture | 1 | hard-fail | N-src, P-ast:ast_name, P-ast:docstring, N-str | expressible |
| conn_or_txn_union | 1 | hard-fail | P-ast:sig_param, LOCAL, P-sign, D-gate, D-place | expressible |
| connection_mode_split | 1 | hard-fail | P-ast:ast_name | expressible |
| async_sync_delegation | 1 | hard-fail | P-ast:body_stmt, P-ast:docstring, P-ast:dotted_expr, LOCAL, N-parse, N-read, L5, N-fn:@json_str, N-fn:@ed25519_ok, D-budget, D-group, D-gate, D-place | expressible |
| async_executor | 1 | hard-fail | — | expressible |
| hasattr_async | 1 | hard-fail | N-src | expressible |
| sql_grammar_escape_hatches | 1 | hard-fail | N-src, P-ast:leaf_name, P-ast:assign_target, P-ast:sig_param, LOCAL, P-sign, D-gate, D-place, CONTRACT | expressible |
| grammar_render_methods | 1 | hard-fail | P-ast:leaf_name, P-ast:def_frame, P-ast:string_form, P-ast:ast_constant, P-sign, D-gate, D-place | expressible |
| rust_sql_grammar_boundary | 2 | hard-fail | D-scope, N-src | expressible |
| grammar_callable_constructors | 2 | hard-fail | P-ast:leaf_name, P-ast:sig_param, P-ast:def_frame, N-order, LOCAL | expressible |
| grammar_str_params | 2 | hard-fail | N-fn:@unparse, P-ast:base_name, P-ast:sig_param | expressible |
| generated_index_sources | 2 | hard-fail | P-ast:sig_param, P-ast:def_frame | expressible |
| serverplan_bypass | 2 | hard-fail | LOCAL, P-ast:def_frame, P-sign, D-gate, D-place | expressible |
| mandatory_imperative_tripwire | 2 | hard-fail | P-ast:ast_name, N-str, P-ast:sig_param, P-ast:py_if, P-ast:py_orelse, LOCAL, N-fn:@unparse, N-parse, D-on, D-place | expressible |
| execution_authority_bypass | 2 | hard-fail | LOCAL, P-ast:ast_name, P-ast:def_frame, D-place | approximate: registry read statically, exact only while it stays literal |
| serverplan_steps | 2 | hard-fail | P-ast:dotted_expr, P-ast:leaf_name, P-ast:base_name, P-ast:ann_root, LOCAL, P-sign, D-gate, D-place, N-parse, D-on | expressible |
| post_op_side_effects | 2 | hard-fail | LOCAL, P-ast:assign_target, P-ast:ast_name, P-ast:sig_param, P-ast:str_const, P-sign, L5, N-fn:@json_str, N-fn:@ed25519_ok, N-parse, D-place | expressible |
| executor_publication_repairs | 2 | hard-fail | N-src | expressible |
| serverplan_name_inference | 2 | hard-fail | N-src | expressible |
| versioned_table_name_manipulation | 2 | hard-fail | N-order, N-str, P-ast:ast_name, P-ast:assign_target, P-ast:assign_value, P-ast:sig_param, P-ast:str_const, P-ast:fstring, P-ast:call_dotted | expressible |
| history_serverplan_projection | 2 | hard-fail | N-str | expressible |
| revision_ancestry_invariants | 2 | hard-fail | — | expressible |
| serverplan_state_tripwires | 2 | hard-fail | LOCAL, P-ast:py_frame, P-ast:chain_step, P-ast:call_name, P-ast:sig_param, N-order, N-fn:@unparse, N-read, P-sign, N-fn:@ed25519_ok, D-gate, D-place, L7 | expressible: `dfs_first_memo` reproduces the memoised DFS, cycles included |
| hot_path_reflection | 3 | hard-fail | P-ast:dotted_expr, LOCAL, P-ast:str_const, D-scope | expressible |
| graph_lazy_sql_plan | 3 | hard-fail | P-ast:ast_name | expressible |
| bounded_composed_graph_paths | 3 | hard-fail | N-src, D-scope, D-place, CONTRACT | expressible |
| serverplan_grammar_surface | 3 | hard-fail | P-ast:ast_name, N-parse, D-on | expressible |
| lazy_sql_surface | 3 | hard-fail | P-ast:chain_step, P-ast:py_frame, N-fn:@unparse, N-src, D-scope | expressible |
| bootstrap_lazy_sql | 3 | info | LOCAL, D-scope | expressible |
| schema_bootstrap_contracts | 3 | hard-fail | P-ast:sig_param, P-ast:ast_name, CONTRACT | expressible |
| schema_architecture | 3 | hard-fail | P-ast:sig_param, P-ast:ast_name, P-ast:chain_step, N-parse, D-on, D-scope | expressible |
| bootstrap_eager_resolve | 3 | info | LOCAL, D-scope | expressible |
| graphql_plan_ownership | 3 | hard-fail | P-ast:sig_param, P-ast:chain_step, P-ast:chain_root, P-ast:py_frame, N-parse, D-on, D-scope | expressible |
| lazy_adapter_ownership | 3 | hard-fail | P-ast:py_frame, P-ast:str_const, P-ast:sig_param, N-parse, D-on, N-src, D-scope | expressible |
| legacy_execution_surfaces | 3 | hard-fail | P-sign, D-gate, D-place, P-ast:from_module_raw, P-ast:chain_step, P-ast:class_path, P-ast:py_frame, LOCAL, N-order, N-src, D-scope, L7 | expressible: `dfs_first_memo` reproduces the memoised DFS (data-layer/sketches/transitive-eager.rr) |
| removed_compat_surfaces | 3 | hard-fail | P-ast:ast_name, D-scope | expressible |
| legacy_callback_quarantine | 3 | hard-fail | P-ast:chain_step, LOCAL, P-ast:sig_param, D-scope | expressible |
| pending_pipeline_effect_kinds | 3 | hard-fail | P-ast:ast_name, D-scope | expressible |
| versioned_lifecycle_legacy_surfaces | 3 | hard-fail | D-scope | expressible |
| plan_helpers_allowlist | 3 | hard-fail | P-sign, D-gate, D-place, P-ast:owner_path, P-ast:from_module_raw, LOCAL, L6, N-order, D-scope | expressible |
| serverref_purity | 3 | hard-fail | P-ast:str_const, P-ast:ast_name | expressible |
| serverref_sql_lowering | 3 | hard-fail | N-src, P-ast:docstring, D-scope | expressible |
| raw_execute_regression | 3 | hard-fail | N-src, P-ast:py_frame, D-scope | expressible |
| no_blanket_grammar_bypass | 4 | hard-fail | N-src | expressible |
| classvar_check | 4 | hard-fail | N-op, P-ast:cmp_pair, CONTRACT | expressible |
| backend_type_strings | 4 | hard-fail | N-src, CONTRACT | expressible |
| type_safety_patterns | 4 | hard-fail | N-src, L5, D-code | expressible |
| expression_owner_propagation | 4 | hard-fail | P-ast:call_name, P-ast:ast_name, P-ast:assign_target, P-ast:sig_param, N-order, L6, LOCAL, CONTRACT | expressible |
| broad_type_surfaces | 4 | metric | LOCAL, P-lib:rifrom, P-lib:module_path, P-ast:class_path, P-ast:def_chain, N-fn:@unparse, L5, N-read, N-fn:@json_str, N-fn:@ed25519_ok, N-src, D-budget, D-mode, D-scope | gap, closed: the vendored grammar parses PEP 696 defaults; P-ast:type_param |
| broad_type_surfaces_ratchet | 4 | hard-fail | LOCAL, P-lib:rifrom, P-lib:module_path, P-ast:class_path, P-ast:def_chain, N-fn:@unparse, L5, N-src, N-read, P-sign, D-budget, D-gate, D-place, D-scope | gap, closed: as broad_type_surfaces |
| no_source_inspection | 4 | hard-fail | N-src | expressible |
| bootstrap_cache_health | 4 | hard-fail | N-src, P-sign, D-gate, D-place, CONTRACT | expressible |
| plan_optimizer_v2 | 4 | hard-fail | P-ast:ast_name, N-str, N-src, N-fn:@count_matches, D-scope, D-place | expressible |
| plan_composition_architecture | 4 | hard-fail | N-artifact, P-ast:from_module_raw | expressible + artifact |
| dynamic_module_loading | 4 | hard-fail | N-src, LOCAL, P-sign, D-gate, D-place | expressible |
| unforgeable_grants | 4 | hard-fail | P-ast:call_name, P-ast:call_dotted, P-ast:assign_target, LOCAL, P-sign, D-gate, D-place | expressible |
| permission_context_system_flag | 4 | hard-fail | P-ast:call_dotted, P-ast:call_name, P-ast:ast_name, P-ast:py_if, P-ast:same_file_call, LOCAL, P-sign, D-gate, D-place, N-parse | expressible |
| permission_test_classification | 4 | hard-fail | P-ast:sig_param, P-ast:chain_root, N-read, D-budget, D-place, D-scope, LOCAL, CONTRACT | gap: JSON-Lines and nested-array JSON reading |
| permission_test_expansion | 4 | hard-fail | D-place, L5 | expressible |
| provider_storage_permissions | 4 | hard-fail | — | expressible |
| no_api_root_imports | 4 | hard-fail | N-str, LOCAL, CONTRACT | expressible |
| data_layer_facade_purity | 4 | hard-fail | LOCAL, P-ast:assign_target, P-ast:assign_value, P-ast:from_module_raw | expressible |
| data_layer_compat_aliases | 4 | hard-fail | P-ast:assign_target, P-ast:assign_value, P-ast:leaf_name | expressible |
| connection_caches | 4 | hard-fail | N-src | expressible |
| schema_introspection_data_paths | 4 | hard-fail | LOCAL, N-str, P-sign, D-gate, D-place | expressible |
| auto_api_route_descriptions | 4 | hard-fail | P-ast:docstring, P-ast:class_path | expressible |
| auto_api_plan_boundaries | 5 | hard-fail | P-ast:def_frame | expressible |
| error_handling | 5 | hard-fail | — | expressible |
| system_session_scope | 5 | ratchet | N-src, N-read, L5, N-fn:@json_str, N-fn:@ed25519_ok, LOCAL, D-budget, D-gate, D-place | expressible |
| system_grant_bypass | 5 | hard-fail | P-ast:ast_name | expressible |
| module_level_caches | 5 | hard-fail | N-read, D-gate, D-place | expressible |
| notification_lifecycle | 5 | hard-fail | P-ast:def_chain | expressible |
| unresolved_forward_refs | 5 | hard-fail | P-ast:ast_parent, P-ast:cmp_pair | expressible |
| permission_error | 5 | hard-fail | N-src | expressible |
| forwardref | 5 | hard-fail | — | expressible |
| sync_async_signatures | 5 | hard-fail | P-ast:sig_param, LOCAL | expressible |
| tstring_deps | 5 | hard-fail | CONTRACT | expressible |
| txn_required_methods | 5 | hard-fail | — | expressible |
| grammar_fragment_usage | 5 | hard-fail | LOCAL, N-order, P-ast:sig_param, N-parse, D-on | expressible |
| grammar_allowlist_burndown | 5 | hard-fail | — | expressible |
| polymorphic_tripwires | 5 | hard-fail | LOCAL, P-ast:owner_path | expressible |
| polymorphic_type_id_authority | 5 | hard-fail | P-ast:owner_path | expressible |
| async_patterns | 5 | ratchet | LOCAL, P-ast:sig_param, P-ast:chain_root, P-ast:py_frame, N-src, P-sign, N-read, D-budget, D-gate, D-place | expressible |
| render_fstrings | 5 | ratchet | P-ast:fstring, N-str, LOCAL, P-ast:def_frame, P-sign, N-read, D-budget, D-gate, D-place, CONTRACT | expressible |
| async_sync_boundary | 5 | ratchet | P-ast:call_dotted, LOCAL, P-ast:def_frame, P-sign, N-read, D-budget, D-gate, D-place | expressible |
| backend_coupling | 5 | info | — | expressible |
| matching_architecture | 6 | hard-fail | N-src, P-ast:ast_name, P-ast:unparen, P-ast:assign_target, P-ast:assign_value, P-ast:str_const, N-str, N-order, L5, LOCAL, D-place | gap: scope extended by a run-time option (`--application-root`) |
| impl_direct_execution | 6 | hard-fail | — | expressible |
| render_raw_strings | 6 | ratchet | N-str, P-ast:ast_parent, P-ast:docstring | expressible |
| rowid_comparisons | 6 | ratchet | P-ast:base_name, N-op, P-ast:cmp_pair, N-order, LOCAL | expressible |
| security_patterns | 6 | ratchet | P-ast:ast_parent, P-ast:ast_constant, P-ast:assign_target, LOCAL, N-src, D-sev, D-group, D-budget, CONTRACT | expressible |
| serverplan_dispatch_architecture | 6 | hard-fail | LOCAL, P-ast:unparen, P-sign, D-place | expressible |
| serverplan_quality | 6 | hard-fail | N-src, N-parse, P-ast:ast_name, P-ast:str_const, N-str, P-ast:py_if, P-ast:py_orelse, P-ast:assign_target, P-ast:assign_value, P-ast:chain_step, P-ast:dotted_expr, P-ast:base_name, P-ast:body_stmt, P-ast:ast_constant, P-ast:cmp_pair, P-lib:always_raises, LOCAL, N-read, P-sign, D-place, CONTRACT | expressible |
| serverplan_target_architecture | 6 | hard-fail | N-artifact, D-place, CONTRACT | expressible + artifact |
| versioned_merged_identity | 6 | hard-fail | LOCAL, D-place | expressible |
| architecture_rule_docs | 6 | hard-fail | N-artifact, D-scope, N-src, N-fn:@count_matches, N-git, D-place | gap: Git object existence for arbitrary history commits unspecified |
| conditional_architecture | 7 | hard-fail | LOCAL, D-place | expressible |
| admin_tools | 7 | ratchet | N-src, LOCAL, D-group, D-budget | expressible |
| admin_async_purity | 7 | hard-fail | N-src, LOCAL, P-sign | expressible |
| noqa_sec_ratchet | 7 | ratchet | N-src, D-code, D-sev, N-read, P-sign, D-gate, D-budget, D-place | expressible |
| grammar_mutation_invalidation | 7 | hard-fail | LOCAL, CONTRACT | expressible |
| declared_fact_interrogation | 7 | hard-fail | — | expressible |
| publication_registry_writes | 7 | hard-fail | — | expressible |
| unknown_contract_defaults | 7 | hard-fail | P-ast:bool_operand, LOCAL | expressible |
| regex_sql_parsing | 7 | ratchet | N-str, L5, LOCAL, CONTRACT | expressible |
| file_length | 7 | hard-fail | N-src, N-read, P-sign, D-gate, D-place, D-sev | expressible |
| markdown_docs | 7 | ratchet | N-git, D-scope, N-src, LOCAL, D-place, D-budget | expressible |
| eager_resolve | 7 | hard-fail | N-src, P-ast:path_module, LOCAL, D-code, N-read, P-sign, N-fn:@ed25519_ok, L5, D-place, CONTRACT | expressible |
| lazy_architecture | 7 | hard-fail | LOCAL, N-order, P-ast:path_module, N-read, P-sign, D-gate, D-budget, D-place, L7 | expressible: its transitive searches use `dfs_first_memo` |
| readthrough_authority_architecture | 8 | hard-fail | N-parse, D-on | expressible |
| authentication_plan_purity | 8 | hard-fail | P-ast:sig_param, N-parse, D-on | expressible |
| pure_lazy_runtime_purity | 8 | hard-fail | P-ast:expr_ctx, P-ast:sig_param, LOCAL, N-read, P-sign, D-place | expressible |
| cached_pure_lazy_allowlist | 8 | hard-fail | N-fn:@tree_digest, N-fn:@sha256, N-fn:@json_str, N-fn:@nfkc, N-fn:@unparse, N-fn:@ed25519_ok, L5, P-ast:py_qualname, P-ast:sig_param, P-ast:expr_ctx, LOCAL, D-place | gap: stored hashes are of `ast.dump` text; no `@ast_dump` |
| rtt_assertion_integrity | 8 | hard-fail | — | expressible |
| rtt_counting_workarounds | 8 | hard-fail | N-src, L5, N-fn:@sha256, N-fn:@json_str, P-sign, D-gate, D-place | expressible |
| route_guardrail_policy | 8 | hard-fail | N-read, P-sign, LOCAL, D-place | expressible |
| connection_structural_boundaries | 8 | hard-fail | N-src | expressible |
| bootstrap_engine_invariants | 8 | hard-fail | P-ast:sig_param | expressible |
| connection_service_architecture | 8 | hard-fail | P-ast:expr_ctx, P-ast:body_stmt | expressible |
| ignore_counts | 8 | ratchet | N-src, N-read, LOCAL, L5, N-fn:@json_str, N-fn:@ed25519_ok, D-budget, D-gate, D-place | expressible |
| test_budget | 8 | ratchet | N-artifact, N-read, LOCAL, L5, N-fn:@json_str, N-fn:@ed25519_ok, D-budget, D-gate, D-place | expressible + artifact |
| migration_progress | 8 | metric | N-git | expressible |
| grammar_class_survey | 8 | metric | P-ast:sig_param, P-ast:expr_ctx, N-read, P-sign, D-budget | expressible |
| tstring_syntax | 8 | metric | — | expressible |
| lazy_result_usage | 8 | hard-fail | P-ast:body_stmt | expressible |
| imperative_private_functions | 8 | metric | LOCAL | expressible |
| internal_dead_code | 8 | metric | LOCAL | expressible |

Since this mapping (2026-11-04), each gap below has a resolution in lib/core.rr (`@ast_dump`, `IO=jsonl` and
`*` pointer steps, `.scope dirs option(…)`, `git_commit`, `.requires`). The PEP 696 question was the grammar's:
Rerail's vendored tree-sitter-python already parses type-parameter defaults, and python/ast.rr has `type_param`. See [../DESIGN.md](../DESIGN.md), "Coverage".

## Gaps

These needs map to no element. Each is listed with its check, the need, the analysis's citation and a suggestion.

| check | need | citation | suggestion |
|---|---|---|---|
| standalone_linter_entrypoints | "this policy fails for each member policy with blocking findings", and a member timeout counts as a failure | lint_standalone_linter_entrypoints_plugin.py:127-152, :104-114 | a directive `.requires "data-layer:x", …` that reports one finding per member that is missing or blocks. Or leave it to CI orchestration: it is not a fact query |
| broad_type_surfaces, broad_type_surfaces_ratchet | the parser must accept PEP 696 type-parameter defaults (`class C[T: X = X]`); BTS009 reads `type_parameter.default_value`, and the repository uses the syntax | lint_broad_type_surfaces.py:701; impl/common/l2066_plan_optimizer/ir.py:49 | resolved: native/grammars/tree-sitter-python/PATCHES.md patch 1 adds `type_default` (fields `name`, `value`), and the engine's scope pass already binds it; python/ast.rr `type_param(o, p, nm, i, kind, bound, dflt)`. The `t` prefix of tstring_syntax is lexed by the same scanner as `f` (scanner.c), and string_form tells them apart |
| permission_test_classification | read the JSON-Lines inventory and the manifest's nested arrays (`groups[].operations[]`, `tests[]`). IO=json's pointer reaches one level of records, and IO=lines does not parse JSON | lint_permission_test_classification.py:552-560, 569-597; permission_surface_manifest.py:240 | N-read extensions: `IO=jsonl` (one row per line, with a line-index column) and wildcard pointers (`path="/groups/*/operations/*"`) that bind index columns |
| matching_architecture | `--application-root` adds directories to the scan and marks their files as application files; a missing root is an error | lint_matching_architecture.py:1162-1172 | a scope entry taken from an option, e.g. `.scope include option "application_root"`, plus a fact for documents it added. `--phase` is not a gap: each rule can test `option("phase", …)` |
| architecture_rule_docs | existence of commits and blobs anywhere in history (`git cat-file -e`), for the example SHAs the rule docs cite | lint_architecture_rule_docs.py:219-247 | specify `git_has(sha, path)` for any reachable SHA, answered on demand, and add `git_commit(sha)`. Today core describes it only for the `git_rev` revisions |
| cached_pure_lazy_allowlist | the stored body hashes are hashes of `ast.dump(include_attributes=False)` text. `@tree_digest` compares the same structure, but its digests differ, so every entry would have to be re-signed | verify_cached_pure_lazy_allowlist.py:159-170, 743-777 | N-fn `@ast_dump(n)`: CPython's `ast.dump` text, so that `@sha256(@ast_dump(n))` reproduces the stored values. Or accept a one-time re-signing under `@tree_digest`, which changes repository data |

Claims the analyses tagged LANG or NATIVE that need no new element:
- Findings at a path, line or directory are D-place. A code from a column is D-code.
- Ordered string aggregates are L5, and parse failures are N-parse.
- Foreign signature schemes are L5 + @json_str + @ed25519_ok, and JSON readers are N-read.
- no_api_root_imports needs no match offset: split the decoded value into lines by positive recursion over a
  capture, `(?s)(?<h>[^\n]*)\n(?<t>.*)`, and add the line index to the constant's line.
- serverplan_state_tripwires' argmin needs no new aggregate. `ast.walk` order is (depth, pre-order), so two nested
  `min`s over ast_depth and visit_index give it.
- sql_grammar_escape_hatches' all-or-nothing ceiling is a LOCAL `count : { candidate } > 16` guard on each finding.
- test_budget's 10% drop is measured from the observed sum, not from the signed ceiling, so it is a LOCAL rule and not
  `drop 10%`.
- broad_type_surfaces' signed JSONL is IO=lines, with each finding's line rebuilt by @json_str and compared as text.

Shared needs left LOCAL. Each is used by two or more checks but no library has it yet; they are candidates under
decision 0005:
- callback_arg (transaction_closures, transaction_callback_wrappers);
- declared_ref and param_type (imperative_private_functions, internal_dead_code);
- iter_name, as one #include (bootstrap_lazy_sql, bootstrap_eager_resolve);
- post_op_fate (serverplan_steps, post_op_side_effects);
- the path-derived symbol `er_symbol`/`la_symbol` (eager_resolve, lazy_architecture);
- the textual callee with a bare-attribute fallback (hot_path_reflection, unforgeable_grants, async_sync_boundary);
- annotation-mention variants (transaction_callback_wrappers, serverplan_steps, async_patterns,
  grammar_fragment_usage);
- signing layouts beyond signing.rr's two: transaction_patterns' categories; the count baselines of
  async_sync_delegation, system_session_scope and ignore_counts; the derived-row messages of cached_pure_lazy_allowlist.

## Ordered recursion verdicts

The first four are the LANG claims. The rest are LIB or EXISTS claims of order-dependent recursion.

- versioned_table_name_manipulation (LANG): no-L6. Every event (assignment, `for` target, def parameter) resets the
  name whatever it stores. So the latest event is a `max` over syntax, and only the stored value recurses, positively
  (data-layer/versioned-table-names.rr).
- plan_helpers_allowlist (LANG): L6. `imported_names[x]` is written only when the value resolves to a helper, so a
  non-resolving rebinding keeps the old alias. Whether `b = a` counts depends on a's alias state at that write.
- expression_owner_propagation (LANG): L6 for `src_code`. A name keeps the code of its latest assignment whose value
  has one, and that value's code can come from another name's state (sketches/owner-code.rr). The constructor aliases
  are monotone, with no kill: positive recursion (no L6).
- serverplan_state_tripwires (LANG argmin): not recursion. First-witness BFS order is (ast_depth, visit_index):
  N-order and two `min`s.
- async_sync_delegation (`final_rebinding`): no-L6. Only `x = <identifier>` writes qualify, which is syntax; the chain
  is followed positively.
- dynamic_module_loading (`in_docstring`): no-L6. A two-state machine over line numbers whose transitions negate only
  line text.
- grammar_fragment_usage (`grammar_class`, `builds_str`): no-L6. The sets only grow. A base resolves through the
  latest alias, and every import or assignment resets that alias. "Defined earlier" is existential.
- polymorphic_tripwires (`alias_at`): no-L6. An alias holds if some `T = X` ends before n in an enclosing scope: no
  kill, existential.
- async_patterns (`alias_from`, `txn_mode`): no-L6. Aliases are never removed. txn_mode recurses up `nearest`, not
  along order.
- rowid_comparisons (annotation map): no-L6. Last write wins over annotation events, which are syntax: a `max` with
  no recursion.
- serverplan_dispatch_architecture (`latest_store`, `executor_owner`): no-L6. Any reassignment replaces the alias (a
  non-owner one removes it), so the latest store is syntax and ownership recurses positively.
- versioned_merged_identity (`catalog_proj`): no-L6. The latest assignment before the constructor is a `max` over end
  offsets, and the projection recursion is positive.
- serverplan_quality and conditional_architecture (`always_raises`): no-L6. A chain over branches, as in flow.rr. The
  stoppers (`return`, a raise of another type) are syntax.
- eager_resolve (`ordered_alias`): no-L6. A monotone forward pass with no kill: "established before p" is
  existential.
- lazy_architecture (`scoped_rhs`, `callable_aliases`): no-L6. The latest assignment in an enclosing block is a `max`
  over syntax, and the kill on reassignment means every write resets.
- pure_lazy_runtime_purity (taint): no-L6. Taint is never removed: positive recursion over offsets.
- connection_service_architecture (aliases): no-L6. "Assigned before the probe" in the alias's own function is
  existential.

## Counts

Each check is counted once per element, prefix or family it uses. "—" rows count for none. N-artifact appears in six
checks, and five have the status `expressible + artifact`: architecture_rule_docs also uses it, but its status is a
gap. Every D-place row uses `@place`, which is why N-fn:@place is not counted on its own.

| family | checks |
|---|---|
| P (libraries) | 85 |
| N (native) | 79 |
| D (directives) | 70 |
| LOCAL | 60 |
| CONTRACT | 20 |
| L (language additions) | 21 |

| prefix | checks |
|---|---|
| P-ast | 76 |
| LOCAL | 60 |
| D-place | 46 |
| N-src | 38 |
| P-sign | 30 |
| D-gate | 27 |
| D-scope | 23 |
| N-read | 21 |
| CONTRACT | 20 |
| N-fn | 18 |
| D-budget | 16 |
| L5 | 16 |
| N-parse | 14 |
| N-str | 12 |
| N-order | 10 |
| D-on | 9 |
| N-artifact | 6 |
| D-code | 4 |
| D-group | 3 |
| D-sev | 3 |
| L7 | 3 |
| N-git | 3 |
| P-lib | 3 |
| D-mode | 2 |
| L6 | 2 |
| N-op | 2 |

| element | checks |
|---|---|
| LOCAL | 60 |
| D-place | 46 |
| N-src | 38 |
| P-sign | 30 |
| P-ast:sig_param | 28 |
| D-gate | 27 |
| D-scope | 23 |
| N-read | 21 |
| CONTRACT | 20 |
| P-ast:ast_name | 19 |
| D-budget | 16 |
| L5 | 16 |
| N-parse | 14 |
| N-str | 12 |
| N-fn:@ed25519_ok | 11 |
| P-ast:def_frame | 11 |
| N-fn:@json_str | 10 |
| N-order | 10 |
| P-ast:assign_target | 10 |
| D-on | 9 |
| N-fn:@unparse | 7 |
| P-ast:chain_step | 7 |
| P-ast:py_frame | 7 |
| P-ast:str_const | 7 |
| N-artifact | 6 |
| P-ast:class_path | 6 |
| P-ast:assign_value | 5 |
| P-ast:docstring | 5 |
| P-ast:expr_ctx | 5 |
| P-ast:leaf_name | 5 |
| D-code | 4 |
| P-ast:base_name | 4 |
| P-ast:body_stmt | 4 |
| P-ast:call_dotted | 4 |
| P-ast:call_name | 4 |
| P-ast:cmp_pair | 4 |
| P-ast:dotted_expr | 4 |
| P-ast:from_module_raw | 4 |
| D-group | 3 |
| D-sev | 3 |
| L7 | 3 |
| N-git | 3 |
| P-ast:ast_constant | 3 |
| P-ast:ast_parent | 3 |
| P-ast:chain_root | 3 |
| P-ast:def_chain | 3 |
| P-ast:owner_path | 3 |
| P-ast:py_if | 3 |
| D-mode | 2 |
| L6 | 2 |
| N-fn:@count_matches | 2 |
| N-fn:@sha256 | 2 |
| N-op | 2 |
| P-ast:ann_root | 2 |
| P-ast:fstring | 2 |
| P-ast:path_module | 2 |
| P-ast:py_orelse | 2 |
| P-ast:unparen | 2 |
| P-lib:module_path | 2 |
| P-lib:rifrom | 2 |
| N-fn:@nfkc | 1 |
| N-fn:@tree_digest | 1 |
| P-ast:bool_operand | 1 |
| P-ast:py_qualname | 1 |
| P-ast:same_file_call | 1 |
| P-ast:string_form | 1 |
| P-lib:always_raises | 1 |

| status | checks |
|---|---|
| expressible | 122 |
| gap | 7 |
| expressible + artifact | 5 |
| approximate | 1 |
