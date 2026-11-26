# Roll-up 41: final-rerail-shape-rr-align (ae1421e5c0) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at 79030b146c. That is roll-up 40 (CUDA-support) plus the planner's head-bound
cross product (p407).

Theirs: final-rerail-shape-rr-align at ae1421e5c0. Its merge base with ours is roll-up 38 (d01d8cdef4). What theirs
brings since then:

- decision 0575: a narrow copy is its original less the annotations' change. There are three forms: a copy within
  one document (its original's rows under the bound), a diffed copy, and a rule written beside its annotation twin
  as its deltas;
- the push holds back every reader of a helper that is not file-local, in one round (9baa291010);
- a pushed join keeps a bound's test with the reader, and is named by its join (dee0982a0e, 2ea0e8170d);
- 0401's collection work: kept-facts collection is a roll-up (F-R4), a run an analysis run owns leaves its
  collection to the owner, and the whole-run cutoff applies under an analysis run (`manifest_root`);
- the parser's generated Rust written directly by the Bitlex compiler (`bitlex_rust`, `bitlex_lex_rust`, decision
  0550), with `rust_block` and Kotlin's old ahead-of-time build removed;
- 0523's preprocessor grid (the sort's local words, a top's rounds as one function);
- "a slab lease with no device is `UnsupportedBackend`" (87fe4b4a28), with the device tests' `no_device`;
- its merges of Go-C-and-C-support, Parser-generators and the hub (roll-ups 27 and 28 on that branch).

Each item follows t51's form: each side's reason, the requirement each side serves, the merged shape, and the tests
that hold it.

## The invariants every item is checked against

From ours:

- **The crate split** (roll-up 39). The compiler's sources stay in `crates/compiler/src`. Each backend is its own
  crate that mounts them: the Bitlex compiler is `rerail-bitlex`, tier 1 on the CPU is `rerail-cpu-block`, tier 2 is
  `rerail-cpu-stream`. A module theirs added to the compiler's `lib.rs` goes where the split put its kind.
- **Windows' device layer** (0440, roll-up 40). Every device gate names `windows` beside macOS and Linux.
- **Narrowing by shown columns** (roll-up 40, item 4). A rule that is not a widened original's own reads an original
  narrow only at an atom that feeds a column an output shows. Main's whole-rule form lost eleven typing tests' rows,
  and it made a narrow copy of the run's costliest stratum (8.02 s of narrow copies' time became 2.48 s).
- **A warm run writes nothing** (decisions 0211 and 0227, `the_run_after_a_cold_one_writes_nothing`).
- **Every persistent grid joins** (t51 item 4, 0540 §3). The grid is the groups that are running, not the host's
  count.

From theirs:

- **A copy costs the annotations' share, not the tree's** (0575).
- **A record only grows** (0401 step 8). A program's reader record names every entry the program put or asked.
- **Generated Rust is written from the grammar** (0550), never lowered from an intermediate form.
- **No device is the only skip** (87fe4b4a28). Where a device exists, any other device error fails the test.

## 1. Narrow copies: which reads (ours) and how each copy is made (theirs)

**The requirement** (decision 0165 §3, main's #923). A rule's rows are its own. A rule that reached no annotation
reads a widened original as hiding left it, `rr_narrow_<orig>`.

**Ours** (roll-up 40's `Narrowing`) decides *which* reads are narrowed. In a rule that sees, only an atom that reads
a variable its head shows (`shown_vars`) reads the narrow copy. Its narrow copies' set (`read`) is what those atoms
reach, transitively. A narrow copy's rules read every original narrow (`Redirect::All`).

**Theirs** (0575) decides *how* each copy is made. It keeps main's choice of reads, "every read of a rule that sees"
(`narrow_reads: bool`), and makes each copy cheaper:

- a copy within one document is a filter of the widened original (`one_document`);
- a copy whose original is diffable is the original's rows less the candidates its change points make (`Diffed`);
- a rule beside its annotation twin is written once a place, that place read as its delta (`beside_twin`);
- the rest are derived again, as before.

**Merged: theirs' copies over ours' reads.** The two decisions are independent: ours narrows fewer atoms, and theirs
makes each copy that remains cheaper. They meet in one place, the question "does this atom read a narrow copy?".
Theirs answered it with a rule-wide bool in five places; the merge answers it with one predicate.

- `Narrowing` is ours', now holding `widened`, so `sees(head)` keeps main's form. It answers once a rule:
  `Narrowing::redirect(v, r)` is `Redirect::Shown(shown_vars)` where the head sees, else `Shown(∅)`.
  `Redirect::narrows(a)` is the per-atom answer. `native_narrow(head)` is the native operators' answer, the same as
  `declare` renames them by.
- `Fold` carries a `Redirect` in place of theirs' `narrow_reads`. `Fold::narrowed(a)` is the one test, and `look` and
  `atom_as` use it.
- `beside_twin`: a place is an atom the rule reads narrow (`rw.narrowed(a)`). An atom it reads wide reads what the
  twin reads, so it has no delta. The deltas' rules keep the rule's own redirect, where theirs wrote `true`, which
  was that rule's value.
- `Diffed::write`: the wide side keeps the rule's redirect, the narrow side is `Redirect::All`. A change point is a
  read of a copy that the wide rule does not read narrow itself, at that place.
- `Diffed::new` and `one_document` read the program as the fold will write it: each atom's edge goes to the narrow
  copy where `redirect.narrows(a)`, and a native operator's where `native_narrow`.
- `Narrowing::declare` (ours) says of each copy what its original says: qualifiers, tapes, layouts, lattice, facts,
  shapes, nullary and natives. It takes theirs' `within`: a copy within one document reads its original, so it gets
  no join shape of its own. Theirs' inline qualifier copy and `declare_narrow` go, so one place declares a copy.
- Theirs' `RERAIL_NARROW_DIFF=off` stays, as an A/B switch, listed in `COMBINING` since it changes the combined
  program (`every_switch_lowering_reads_is_classified`). Theirs' `RERAIL_NARROW_DIFF_DEBUG` and its `eprintln!`
  lines go: a product path prints nothing.

**Tests**: theirs' `a_narrow_copy_of_an_original_within_one_document_is_its_rows_under_the_narrow_bound` and the
diff tests; ours' `a_policy_beside_one_that_reads_annotations_finds_what_it_finds_alone`,
`a_policy_reads_in_a_pack_only_the_guests_it_asks_for_alone` and `quoted_annotations_are_read_as_their_code`; and
`plan::corpus`, `typing::`, `wellfounded` and `rr::tape_tests`.

## 2. The push: two fixes of one bug

**The requirement.** A helper of the same content is made once and read by every rule whose component it is. If the
split does not take it, all its readers are held back in one round. Held back one reader a round, main's Python pack
never finished (ours), and ep2's `violation` took 400 rounds (theirs).

**Ours** (2889cbfd78): `Readers`, a type with `read`, `refused` and `names`.
**Theirs** (9baa291010): the same fix inline over a `Vec<(usize, String)>`.

**Merged: ours' `Readers`.** Theirs' hunks were the same fix written a second way. Theirs' other push changes are
kept, and they merged without conflict: a bound's test with the reader, `by_content` naming, and the reader test.

## 3. The manifest's root: one function, ours' path trait

Theirs moved the manifest's root into `manifest_root`, which `reuse` now reads too (7127549186: a hit re-owns the
recorded manifests). Ours had moved the same line onto the Windows canonical-path trait (3dc28883f3). Merged:
theirs' function, with ours' `root.canonical()`. Its insertion had moved `record_run`'s doc and
`#[allow(clippy::too_many_arguments)]` onto it, so they are put back on `record_run`.

## 4. A program's reader record in a wave: the gather unions it

**Found by the merge** (`the_run_after_a_cold_one_writes_nothing` and its two siblings failed). Theirs records each
run program's reads and puts as a reader record in the space (1175a375b0). Ours gathers a wave's puts for a space
into one block (Q243, `KeptGather`), and a later put of a key replaces the earlier one. A program's record is put
with each of its puts, so a wave of several kept the last put's entries only. The warm run then asked the others
again and wrote its record, a write in a warm run.

The gather is older than both sides, so theirs had the same loss. It showed here because ours' runs put a space
several times a wave.

**Merged:** the gather unions two puts of one reader record (`put_idents`, the decoding the writer's union uses too).
Both invariants hold: one block a space a wave, and a record that only grows.

**Test**: `a_waves_two_puts_of_a_programs_record_are_one_naming_both` (store), and the three corpus warm tests.

## 5. The Bitlex writers in the split

Theirs adds `bitlex_rust` and `bitlex_lex_rust` to the compiler's `lib.rs`, and keeps the deprecated lowerings
(0550). Ours' `lib.rs` lists only the front end: the Bitlex compiler is `rerail-bitlex`, and the deprecated
lowerings are `rerail-cpu-stream`'s.

**Merged:** the two writers are mounted by `rerail-bitlex`, beside the rest of the Bitlex compiler. The note each
writes into its output names `rerail_bitlex::…`, and `kotlin_aot`'s generated `tier1.rs` and `tier2.rs` say the
same, so `rust_is_current` compares like with like.

- `rust_block` is theirs' removal (0177ad94e4): its one reader was Kotlin's old ahead-of-time build. `cpu_stream`'s
  mount of it goes.
- `kotlin_aot`'s `build.rs`, `tests.rs` and `tier1_source.rs` go. Ours had changed them only for the split's
  paths.
- `tables_consts` is `pub`: tier 2 (theirs) reads it, and since the split tier 2 is another crate.

## 6. Device gates: ours' platforms, theirs' arms

Ours (0440) runs the device lane on Windows too. Theirs (87fe4b4a28) skips only `UnsupportedBackend`. With no device,
`lane::device()` returns that on every OS. **Merged:** theirs' match arms under ours' `cfg(any(macos, linux,
windows))`, in `c_directives::tokens`, `parser_tests`' block and compile tests, `executors::no_device` and
`rr_parser`'s `no_device`.

## 7. The preprocessor grid: theirs' state, ours' join

Theirs (0523) added the sort's local words (`PP_SORT_LOCAL`), the files, the stop word and the scan jobs to
`PP_GRID_BEGIN`, and set `g.groups = gsz / lsz`, the host's count. Ours (7e654c99d8) made every persistent grid
join, `ukl_grid_join`: the grid is the groups that are running (0540 §3), and the join sets `g.group` and
`g.groups`. **Merged:** theirs' additions, ours' join in place of the host's count.

## 8. Smaller choices

- **Manifests**: `device_metal` and `rr_aot` are ours' (the split's crates, `rerail-classic`). `kotlin_aot` is
  theirs' (tape_format alone: the generated parser has no compiler).
- **`Cargo.lock`**: ours', regenerated from the merged manifests.
- **`global-state.txt`**: both sides' entries. `pp_arena.rs::T` goes, since 0523 removed the static.
- **0165**: both sides' sections, ours' shown-columns narrowing and theirs' "a copy within one document".

## Checks

On a Linux farm host, CPU only (`RERAIL_GPU=cpu`):

- one `cargo check --tests` over every crate the merge touched, then clippy over them: clean. Clippy's 24 findings
  were fixed, not allowed: theirs' Bitlex writers (indexed loops, collapsible `if`s, `write!` with a newline),
  `native_cpu_stream` (a `Some(..).filter`, two casts to the same type), a test's needless borrow, and the merge's
  own (needless borrows of `read`, `values()`, slices made by `clone`, fixed-size chunks as `as_chunks`);
- rerail-tests-logic `plan::corpus`, `typing::`, `wellfounded`, `rr::tape_tests`: 230 of 230;
- rerail-store, rerail-logic, rerail-kotlin-parser, rerail-kotlin-aot, rerail-bitlex, rerail-rr-parser,
  rerail-c-directives: 967 of 967;
- after the lint fixes: rerail-store, bitlex, c-directives, cpu-stream and kotlin-parser 424 of 424; the narrowing
  and switch tests 5 of 5;
- the global-state guard passes;
- `a_waves_two_puts_of_a_programs_record_are_one_naming_both` under Relax, here: fails with the gather's union
  taken out and passes with it.

Not run: `kotlin_parser::tests::rust_is_current` (below), and no corpus bench (the merge's narrowing reads the same
atoms as roll-up 40's, and the copies are theirs' measured forms).

## Found on the way

- **`rust_is_current` takes over 160 s on a fresh host.** It compiles the whole Kotlin grammar, provers included,
  and the provers' answers are kept on the machine's disk (0414), so a fresh checkout proves them all cold. It ran
  past nextest's limit and was killed. It is the next thing to fix after this roll-up's merges.
