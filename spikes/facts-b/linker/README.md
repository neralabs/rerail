# Reference-graph linker as SQL stages

`reference_edges.sql` re-expresses native's Python reference graph (refs mode:
`Graph::link_references` + `resolve_edges` + `reference_rows`, the `reference_edges` table of the
dead-code rule's `unreachable-internal-symbols` clause) as ordered SQL stages over per-content
exported facts and the worktree's path set. Each stage is `-- stage <name>` followed by one
statement (SQLite semantics; `-- index:` lines are speed hints for the harness only). Every stage
comments the native function it mirrors (`native/src/code_rule_dsl/...:line`).

## Input relations

| relation | columns | source |
|---|---|---|
| `member` | `path, hash` | the worktree's files (corpus TSV) |
| `cgn_def` | `hash, variant, stub, idx, name, start, end, flags` | cgn export, `as_references["547566c618e1c9e3"]` |
| `cgn_edge` | `hash, variant, stub, def, name` | 〃 (references + siblingLinks rows) |
| `cgn_alias` | `hash, variant, stub, key, target` | 〃 |
| `cgn_reference_alias` | `hash, variant, stub, key, target` | 〃 (only `<base-slot>` rows are read) |
| `cgn_opaque`, `cgn_module` | `hash, variant, stub, name` | 〃 |
| `cgn_open` | `hash, variant, stub, open, open_sites` | 〃 |

All values are text as exported (names with sentinel tokens `\x01<r>`). `cgn_super` and
`cgn_star` are loaded but not read: supers are rebuilt from the `<base-slot>` aliases (as
`as_references` does), and stars are empty in refs mode.

**Extra export columns needed: none.** The one order native depends on that a set loses — a
class's bases (`reference_bases`, BFS in `reference_order`) — is in the data: the
`<base-slot>` key carries `{classStart}:{ordinal:010}` (summary.rs:1087-1095), the zero-padded
position among the file's reference aliases, so it orders a class's bases as declared. Checked:
equal to the export's `cgn_super` array order for all 8,557 classes with bases.

## Stages

1. **Scope** `scope_glob`, `in_scope` — the rule's `scope.include/exclude` as globset regexes.
   `in_scope` is also the linked file set: a native workspace holds only files some installed
   rule's scope matches (`workspace/ingest.rs:279-292`), so with the dead-code rule the graph is
   linked over in-scope files only (3,110 of the corpus's 9,010).
2. **Module paths** `package_dir`, `file_name`, `package_climb`, `module_rel`,
   `module_name_part`, `module_path`, `module_segment`, `module_prefix`, `file_variant` —
   `python_module_path` / `python_module_in` (model.rs:806-859, package markers from the path
   set), Ruff's stem fallback, `ModulePath::qualified` (dirs by stem, None on a dot), the export
   variant (`module`/`init`/`unpackaged`, cgnative.rs:73).
3. **Substitution** `raw_name`, `name_sub`, `refused_path`, `linked_file` — cgnative.rs:163
   `substitute`, one token per recursion step.
4. **Substituted summaries** `s_def`, `s_edge`, `s_alias`, `s_opaque`, `s_module`, `s_open`,
   `s_base_slot`, `s_super` (as_references summary.rs:139-176: class_starts = last class def).
5. **Name tables** `path_alt` (closure.rs:513 `path_module`), `spelling` (closure.rs:136-145
   alternate spelling), `by_name` (SHADOWED defs unkeyed), `aliases`, `opaque`, `modules`,
   `supers`, `class_key`, `method_def`, `reference_class` (global short-name uniqueness),
   `own_name`.
6. **Class resolution** `base_target`, `base_class` (class_noting `<base>`), `bases`
   (reference_bases), `subclass` (python_class), `marker_class`, `class_root`, `order_dist` +
   `reference_order` (BFS position = (distance, least declared-ordinal sequence along a
   shortest path)), `imported_alias`, `fq_class` (reference_fq_class with hop count `k`),
   `type_key`/`type_unique` (reference_type), `digit`, `field_alias`, `class_of_walk`/`class_of`
   (general class_noting with hop count).
7. **Link closure** `link_reach` — a superset of every name link_python can read from the
   summaries' edge names (fallbacks, `^`, member/declared/imported/own aliases, prefix aliases,
   constructor-field members), each with its least hop cost, cut at 19.
8. **Exact link_python** (closure.rs:1685-1887) — `link_form` (the string guards in their
   order: `<type>`, `<field>`, `<bare>`, `<unqualified>`, `<self>`, `<super>`, `^`, plain),
   `link_plain` (8 exact def / 9 declared-import hop / 10 member / 11 prefix scan),
   `prefix_match` + `prefix_first` (longest matching prefix: filtered alias, def, opaque,
   module), `field_demand`/`field_candidate`/`field_hit`/`field_class` (reference_field with
   constructor_names `^_+`→'', the `<unannotated>` break, reference_type_members),
   `type_demand`, `method_demand`/`method_candidate`/`method_first` (reference_method: first
   class in BFS order), `compose_candidate`/`compose_first` (`<self>` composition: subclass by
   text, base ordinal, BFS position), `link_term` (definitions a name gives directly) and
   `link_kid` (names it unions or follows, with `delta` hops and `chk` = through `link_name`'s
   check). Precedence is NOT EXISTS over the higher-priority candidate.
9. **Hops** `link_result` — `link_name(name, 0)` as a recursion over hop *ranges*: a row says a
   definition is in the link for the caller's hops `h ∈ [lo, hi]`; a checked kid needs
   `h + delta ≤ 16` (MAX_HOPS), class-resolution ranges come from `k` (`fq_class`,
   `class_of`). Ranges shrink every step, so alias cycles end. On this corpus the limit never
   binds (the tightest root still has 7 hops of slack; the unbounded variant gives the same set).
10. **Output** `reference_graph_incomplete` (one row iff an in-scope file's summary is open,
    relational.rs:636; then `reference_edges` is empty), `reference_edges(__path, __start,
    __end, target_path, target_start, target_end, source_name, target_name, target_class)`.
11. **Refusals** `linker_refusal(path, construct)`.

## Refusals (`linker_refusal.construct`)

Rows only where the construct occurs in the data; the stages above never pick one alternative.

| construct | native | rows on 2b1504adc |
|---|---|---|
| module: none (empty stem) / named abc, typing, typing_extensions, functools / deeper than 17 segments / a name `.` or `..` | cgnative.rs:114-120 | 0 |
| path_module: non-ASCII path (Unicode `is_alphabetic` vs ASCII classes) | closure.rs:521-527 | 0 |
| substitution: relative import above the real package root / real module segment keeps a dot | cgnative.rs:163 | 0 |
| class_of `<base>`: first of several alias targets that are classes | closure.rs:2296 (alias entry = slot order) | 0 |
| reference_order: one class key's bases from several files | closure.rs:2108 (slot order) | 0 |
| own_name: first of several definitions under one key | closure.rs:2497 (slot order) | 0 |
| reference_type / reference_fq_class: first of several alias targets that are classes | closure.rs:2245, 2262 | 0 |
| reference_order: class hierarchy deeper than 64 | stage bound | 0 |
| link: name longer than 2048 characters | stage bound (`digit`) | 0 |
| reference_order: root not among class_root | stage completeness check | 0 |

## Results (2b1504adc)

Native edges N = 145,431, SQL edges M = 145,431, differences D = 0, refusals 0,
`reference_graph_incomplete` 0 rows. Run dir:
`~/rerail-bench/facts/phase-2/s3-crossfile/linker/20261022T215523-corpus-2b1504adc/results.json`.

## Running

```sh
python3 spikes/facts-b/scripts/linker_check.py --label corpus-2b1504adc \
  [--members ~/rerail-bench/facts/phase-2/s3-crossfile/exports/oracle-b6-jobs.tsv] \
  [--native ~/rerail-bench/facts/phase-2/s3-crossfile/linker/native-ground-truth/reference_edges.tsv] \
  [--export ~/rerail-bench/facts/phase-2/s4-schema/cgn-export.jsonl.zst] [--inputs-db /tmp/linker-inputs.sqlite] [--keep-db]
```

Python's `sqlite3` with `rx_matches`/`rx_extract`/`rx_replace` registered (Rust regex syntax
used: classes, groups, anchors, `\x01`, `\x00-\x7F`). ~70 s, ~4 GB RSS. Writes `results.json`
and `differences.tsv` into the run dir; exits 1 on an unattributed difference.

### Native ground truth

`~/rerail-bench/facts/phase-2/s3-crossfile/linker/native-ground-truth/`: `reference_edges.tsv`
(path, start, target_path, target_start, target_end, target_class), `members.tsv`,
`policy-reference-edges-dump.json`, `native-suite.log`. Produced by `conformance/data-layer/suite.ts
--cold-only` (engine `sql-dl60`) on `/tmp/dl60/suite-b5.json` filtered to `internal_dead_code`,
with a copy of the dead-code rule whose clause keeps its inputs and `references`, adds an input
`linker_defs` (every function/class definition, the anchor), and has no stages. Two engine
limits shaped it: a finding code is at most 256 bytes (so the code is
`start|target_start|target_end|target_class|target_path`, names re-derived from `s_def`), and a
clause reports at most 10,000 witnesses (`maxWitnesses`), so the clause is split into 24 clauses
by `(__start + target_start) % 24`. All 145,431 edges anchor (every edge source is a top-level
function or method, i.e. a `raw_functions` row too: 0 lost). Peak RSS 41.9 GB, 122 s. The b5
recording's in-scope files equal the corpus's (3,110), so the dump is the corpus's graph.
