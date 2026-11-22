# Roll-up 39: engine-crating (a422e17a72) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at a3e87480d3. That is roll-up 38 (d01d8cdef4) plus t51's fixes (items 2, 3, 4, 6 and
10).

Theirs: engine-crating at a422e17a72, the crate split's steps 1a–4. It took roll-up 38 in its own merge,
8c012cf233, so the merge base is d01d8cdef4. What theirs brings since then is:

- the split itself (boot, registry, the six test crates, the compiler's five crates);
- that merge's resolutions and fixes.

Each item below follows t51's form:

- each side's reason;
- the requirement each side serves;
- the merged shape;
- the tests that hold it.

## The split's invariants (theirs), which every item is checked against

- The engine's `code_rule_dsl/mod.rs` mounts napi alone. Every other module is boot's, and every test is a test crate's
  (`rerail-tests-{demand,transducer,logic,compile,matcher,lang}`), mounted in place by `#[path]`.
- The compiler's tiers (`rerail-bitlex`, `-cpu-block`, `-cpu-pp`, `-cpu-stream`) sit above the front end
  (`rerail-compiler`). Metal reads only the front end. Lang and facts reach no tier (`cargo tree -i`).
- An analysis that emits no code lives in the front end, not in a tier. Examples: `lane_plan`, `byte_needs`, and
  `block_native::counted`. Each tier re-exports it under its old name, but that name is a tier's path.

Ours' seven commits add no module and mount no test, so the first invariant needs nothing from them. Their new tests
are in `warm_tests.rs`, which rerail-tests-logic mounts; its warm profile runs them. The second and third invariants
decide item 3.

## 1. `Batch::ensure`: one fix made twice

**Theirs** (bcd5bfda9d, "red at HEAD, fixed on the way"): "Batch::ensure chose what to make and claimed it under
two locks: two threads could make one document."

**Ours** (a3e87480d3, t51 item 10's run): the same race, found by `reads_side_by_side_make_each_document_once`
failing 13 of 30. It got the same fix: claim under the lock the need was found under.

**Merged.** The bodies are the same code. Only the comment differed, and theirs' is taken because it came first.

**Tests**: `reads_side_by_side_make_each_document_once` (rerail-tests-lang, `vtab::syntax`: 55 of 55).

## 2. A semi-join's kept statistics: two stores for one job (the only real conflict)

**Theirs** (8c012cf233, its merge of roll-up 38). R7 (`r7_a_run_after_an_edit_makes_no_document`) failed at the
hub's head: a run after an edit read no kept semi-join statistics, so it built every document.

- Theirs added a store beside the documents' table: `retain::semijoin_stats` and `put_semijoin_stats`, as
  `#semijoin-stats/<rel>` words.
- Its key is the relation's documents' digest XOR the held nodes' digest, or the documents' alone when measured whole.
- Both routes (the kept one and the corpus batch's) read it before counting.
- It gathers the held nodes for any semi-join.

**Ours** (e870c85994, t51 item 2). The same R7 failure, with the same cause.

- A semi-join's statistics are base statistics, kept per document in the documents' table under their own name
  (`held_stats_name`).
- They stand while the documents are those they were counted of, so no document is walked (`held_counted`, both
  routes).
- The frame-side `semijoin_stats` and `kept_input_stats` route went, as theirs' Q332 (d4aa849d18) had removed it.

**The requirements.** Both stand, and they are the same ones:

- Q396: a cold run counts only the held rows. The view's every row is computed from the tape (1 s on hogql).
- Q332: a warm run makes no unchanged document, and reads no row of one to get a count.

Each side meets both.

**Why ours' shape.** Keeping both would give a semi-join's statistics two kept forms, two digests and two writers.
t51 item 2 chose the documents' table because it is where Q332 put every base relation's statistics. One reader
(`base_stats`) and one writer (`put_base_stats`) cover them all. Theirs' store sits beside the table with an
encoding of its own (`encode_semijoin_stats`).

Theirs' extra key term, the held nodes' digest, adds nothing:

- The held nodes are a function of the exports' rows.
- The exports' rows are a function of the documents and the program.
- The program is the table's own key (`base_documents_ident(binds)`).

So the documents' digest decides it alone. After an edit, ours counts again over the documents the run holds, and
makes none (t51 item 2, "Done"). Theirs' held digest would differ there too, so it would count again just the same.

**Merged.** Ours. Theirs' store and the three places that read or write it (all auto-merged, no conflict marker)
were removed:

- the read in the table's tuple;
- `semijoin_stats_out`;
- its write loop.

Theirs' `Counted::Kept`/`Looked`/`Measured` is ours' `Counted::Held`. Its round-trip test,
`a_semijoins_kept_statistics_read_back_as_written`, tested only the removed encoding, so it went with it.
`corpus.rs` and `retain.rs` are ours' exactly (`git diff a3e87480d3` is empty for both).

**Tests**:

- R7 and the warm profile: 18 of 18.
- `keptstats::a_semijoins_held_rows_counted_by_document_are_their_measure`.
- R4's kept-statistics test and the session tests: in `plan::corpus`, 367 of 367 (rerail-tests-logic) and 169 of 169
  (rerail-logic).

## 3. Metal's block programs: ours' split, theirs' front-end paths

**Theirs** (a422e17a72, step 4). It changed only paths:

- `rerail_compiler::native_cpu_block::counted` became `block_native::counted`;
- `native_cpu_lanes::Plan` became `lane_plan::Plan`.

The reason is the invariant "Metal reads only the front end".

**Ours** (cdd5bf43c1, t51 item 6). `run_batch` is split under main's names (`check_supported`, `fit`, `FileTable`,
`dispatch`, `interpreter_blocks`, ...), with ours' bodies.

**Merged.** Ours' functions, with theirs' paths in them. Ours' split had moved the two `counted` calls (into
`FileTable::new` and `program_row`), so theirs' change had to be carried there by hand. The doc comments that named a
tier's path (`block_serial.rs`, `slabs.rs`, `block_program.rs`) name the front end's. No Metal source names a tier
now.

**Tests** (the M3, `RERAIL_GPU=cpu RERAIL_DEVICE=cpu`, no Metal):

- Clippy over rerail-metal is clean.
- rerail-metal: 25 of 25.
- The workspace's `packed`, `block_program` and `run_batch` tests, in whichever crate holds them now: 262 of 262.

## 4. `DeviceState`'s ledgers under test hooks

Both sides gave `pool` and `peak` the readers' cfg (`not(any(test, feature = "test-hooks"))`):

- theirs in step 1b's "two warning fixes";
- ours in t51 item 6.

It is the same change. Theirs' comment line is taken.

## Found on the way, fixed

- **Clippy** over the workspace, which no earlier roll-up ran whole, found 16 warnings in our crates. Each is fixed
  at its cause, with no allows:
  - json_lean's `walk` took nine arguments. Its three depth stacks are now one `Stacks`, as simdjson's parser holds
    them, and are destructured at the top so the body is unchanged. The other json_lean lints were a ranged loop
    (`zip`), `saturating_sub` and `is_multiple_of`.
  - `rule_api::compile` takes `&str`, but seven call sites passed `&x.to_owned()` or `&s.to_string()`.
  - boot's test-binary load hook stood before boot's items, so it moved to the end.
- **The edit bench** (`scripts/profiling/edit-bench.sh`) called `/usr/bin/time -l`, which is BSD's. On Linux it ran
  nothing (rc 125). It now takes GNU's `-v` there and reads either report.

## Found on the way: main's whole Python pack (both sides, before the merge)

The bench over main's whole Python pack (537 rules, `docs/rule-syntax/packs/python-main`) ran on neither side: a
cold run of one file was still preparing after 113 s. Fixed in the commit after this roll-up's:

- **The push pass held back one reader of a refused join a round.** "push the file-local joins into the chunks"
  (`typing/file_local/passes/push.rs`) was still open after 40 s on one file, 27 s of CPU on one thread and
  28.8 GB allocated. Two causes:
  - A helper of one content is made once for every rule whose component it is, but it was recorded against its
    first reader only. When the split refused it, that rule alone was held back a level. Each other reader made it
    again the next round and failed again, so the pass took a round per reader, each a split of the whole program
    (31,384 relations, 1.2 s).
  - Now every reader is recorded (`Readers`), and a refusal holds back all of them at once. A rule with several
    refused helpers goes back one level a round.
  - Traced on the pack: the two pushes (14,361 and 17,289 helpers made) take 3 rounds each. Without the fix neither
    ended within 50 s.
  - A span marks each round, and `PushLocalJoins::rounds` counts them.
- **The well-founded analysis scanned every helper for each atom.** "well-founded components (decision 0364 §5)"
  summarized 83,170 rules' helpers past 31 s. `order` scanned the whole helper map, every (relation, i, j), for each
  atom of each rule it read, and `summarize` calls it for each pair of node columns, each rule and each of three
  passes. The helpers are keyed by relation now, one lookup an atom.

Still open on that pack. Each is its own fix, after the CUDA-support roll-up:

- **A stratum's evaluation on one file.** `rr_narrow_p407_rr_up2` had run 20 s and allocated 50 GB, with the process
  at 7.9 GB resident.
- **ARG004 under strict SQL**, fixed after roll-up 40. On the first 100 rules, `RERAIL_SQL_STRICT=1` stopped the run: the
  native engine declined the stratum `dm_p67_ARG004_0` (`ruff-arg004-unused-static-method-argument.rerail`). Its rule
  is `dm_ARG004_0(v0) :- dm_ARG004_raw_0(v0), rr_d = @doc(v0), rr_d < RUN_VALUE_KEYS`: hiding's bound on the
  translator's `doc(rr_d, rr_p, _)` stayed after that read was dropped, and the read was what typed `rr_d`. The join
  spec now types `$d = @doc($n)`'s `$d` as what it is, a document key, an integer
  (`a_documents_key_compared_with_a_bound_runs_natively`).

## Measured

spark2 (aarch64 Linux, load 2–4), the profiling build of each side's CLI, `RERAIL_GPU=cpu`, `RERAIL_SQL_STRICT=1`.
The input is corpus0 (300 Python files) under main's 49 bandit rules, each run its own process at nice 19
(`tmp/runs/bench-step.sh`). CPU is user + sys.

| run | before (a3e87480d3) | after (the merge) |
|---|---|---|
| cold | 30.99, 30.08, 29.75 s | 36.47, 30.18, 29.74 s |
| warm | 0.44 s, 149 MB | 0.43 s, 148 MB |
| one file appended to | 3.57 s, 428 MB | 3.53 s, 426 MB |
| findings | 279 / 279 / 280 | 279 / 279 / 280 |

The cold runs alternated, before then after. The first after-run, 36.47 s, came first after the build. The next two
pairs agree within 0.1 s, so the merge costs nothing measurable. The top spans are the same strata at the same costs
(`p47_rr_demand_agg_rr_agg18`, 4.2 s each side).

## Checks

- `cargo check --tests --workspace`: clean (Linux).
- Clippy `--workspace --all-targets`: clean after the fixes above.
- `check_dev_features` and `check_no_block_copies`: pass. The second reads `git diff --cached`, so it ran on this Mac.
- The tests above, plus json_lean's: 2 of 2.
