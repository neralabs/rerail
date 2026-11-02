# T16 notes: the cross-file inputs from this crate's tapes (O39, O40)

Branch `jeremybarnes/final-rerail-shape-facts-crossfile-xref` from the port branch (`3c7e13e0e`)
merged with `35c010235`. Runs are under `~/rerail-bench/final-shape/xref/runs/` (`runs/` below),
each with `COMMIT.txt`, `load-start.txt`, `load-end.txt`, `stdout.txt`, `stderr.txt` and
`results.json`. Driver: `~/rerail-bench/final-shape/xref/run-test.sh`. **[unverified]** = not run
here.

## 1. What Y1 and the symbol table read

Y1 (`ygraph::summary`) reads exports 3 (`in_scope`) and 7–13: native's call summary
(`calls::summarize`) of the content under three symbolic module variants, after
`as_references` with the pack's links (`cgn_def`, `cgn_edge`, `cgn_alias`, `cgn_opaque`,
`cgn_module`, `cgn_open.open`, and the `cgn_reference_alias` rows whose key holds
`.<base-slot>.`). The symbol table (`y3::input`) reads exports 14–26: the X6 pack's thirteen
pattern inputs and four per-file stages over them (`proj_inventory`, the seed of
`proj_imports`' tokens, `proj_explicit_exports`, `dc__module_names`). facts-a produced the
summaries with this crate's `calls::summarize` (`cgnative`) and the inputs with its own leaf
executor (`xexport`).

## 2. The producer (`facts::xinputs`)

- `cgnative`: facts-a's extraction, copied, trimmed to the per-content part, on this crate's paths.
- `pack`: the pack's inputs, each compiled as a `budget` clause by the production compiler with
  the pack's `shared` define, run by `eval::stateless_rows_to` over the tape installed with
  `TapeCacheSlot::seed`. The file is read as `x.py`; the `file` path predicates (`py_all`,
  `py_all_names`) are taken out and applied by the reader, so every row is a function of the
  content alone.
- `exports`: the rows at a path, shaped as the exports (scope, path predicates and module applied
  there). The annotation, decorator, `__all__`-name and dynamic-import exports are distinct.
- `store`: the rows as narrow tables keyed by `tape_objects.tape_id` (`content_store::xinputs`,
  20 tables, texts as atoms, cascading from the tape), a derivation `xinputs:{tape hash}` per
  content (reads the tape hash, outputs the rows' digest), and `StoreSource`: (path, blake3)
  → source alias → tape (stored, else encoded as the tape family does) → set (stored, else
  produced and published) → exports.
- `y3::wire::Source`: the xref and symbols steps read the inputs of their changes on first use
  (`Shared::ensure`), through `summary::insert` and `input::insert`, so the readers are unchanged.

## 3. Exactness

| Check | Result | Run |
|---|---|---|
| Producer = X6 exports, every (path, content) of the replay corpus (19,801 lines, 19,738 contents), exports 3 and 7–26, as multisets and in order | **18,005,997 rows; 0 lines differ (multiset), 0 (order); 0 contents failed**; 65 s at 6 threads, load 70 → 40 | `runs/20261102-154541-diff-full` |
| Job sources = the exports' contents (`facts-b b3sum` of every source) | 19,801 / 19,801 | (shell check, not recorded as a run) |
| Before the distinct exports: duplicates only (no row on one side only) in 16, 17, 20, then 22, 25 | fixed in `exports` | `runs/20261102-153753-diff-every50`, `-154404-diff-full` |
| Unit: `content_store::xinputs` (round trip, replace, cascade; tables = spec), producer smoke test | pass | `runs/20261102-153600-unit` |

## 4. Not done

- The dead-code replay still reads the JSONL: `deadcode::replay` was not switched to
  `StoreSource` (ingest into the run's store, `Shared::source`), so the replay's exactness on
  these inputs (1,151 checks), the oracle test and the footprint and per-edit latency against
  T12 §4.1 were **not run**. The machine stayed at 1-minute load 40–85 throughout, above the
  quiet-run threshold.
- `StoreSource`'s round trip to `Summary`/`SymBlock` is checked only through the store's unit test
  and the producer's differential, not end to end against the JSONL loaders [unverified].
- O39: the xref and symbols derivations still take the path delta from `Pending`. The intended fix:
  the delta as a cache verified against the recorded `#tree:` digest, else a diff of the store's
  bound membership against the producer's, with the inputs read from the store.
