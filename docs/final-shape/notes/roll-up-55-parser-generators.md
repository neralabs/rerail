# Roll-up 55: Parser-generators-final-shape (d558cc0919) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at b94592cd0d (roll-up 54, Q475). Theirs: Parser-generators-final-shape at d558cc0919.
The merge base is 08deed5e74 (roll-up 39). Theirs brings 26 commits:
- JSON's and the `.rerail` parser's Rust written from the grammar (0553-0556, 0560, 0562);
- the deprecated IR-to-Rust lowerings removed (0565: `rust_stream`; 0569: `clif_rust`, `rerail-pp-aot`);
- page 40's fixes to tier 1 and tier 2 in simdjson's structure (0581-0586, 0591, 0599).

## Items

1. **The decisions' numbers.** Both sides had renumbered parser decisions 0547-0549. Ours named them 0557-0559,
   theirs 0577-0579, and theirs' own texts and page 40 cite 0577-0579. Theirs' numbers stand, and ours' three copies
   and their index rows go. Theirs' 0561 and 0570 collide with ours' (a climb's readers; a keyed walk), so theirs'
   become **0562** and **0569**. The index notes their old numbers, and theirs' references (0565, AGENTS.md,
   `clif_ukl/shape.rs`) follow.
2. **Tier 2's facts: one home, theirs'.** Engine-crating's split (61d419e4ab, b58d3db07b) moved tier 2's analyses and
   the direct run's facts out of `native_cpu_stream` and `rust_stream` into `stream_analysis` and `stream_facts`, so
   that the Rust lowering, as its own crate (`rerail-rust-stream`), and the JIT could share them. Theirs removes that
   lowering (0565) and keeps the facts where the JIT reads them (`native_cpu_facts`, pruned to what is reached), and
   it changed the analyses in place (`split_parts`, `Mem`, `fetch_of`).
   - The split's moves were checked item by item: they are moves, with `pub` added.
   - With no second reader, the split's reason is gone. Its invariant, that the JIT reads no Rust lowering, holds in
     theirs' layout.
   - So `native_cpu_stream`, `native_cpu_tape` and `native_cpu_clusters` are theirs', and `stream_analysis.rs` and
     `stream_facts.rs` go.
   - `native_cpu_clusters` is the JIT crate's again. Its callers (rr, Kotlin, C directives, the parser tests) name
     `rerail_cpu_stream::native_cpu_clusters`.
   - `record_tables` stays the compiler's: theirs did not change it.
3. **The removed lowerings go whole** (0565, 0569):
   - the `rerail-rust-stream` and `rerail-pp-aot` crates, with their members, crate-order rows and lock entries;
   - `rust_stream.rs`, the old `rr_aot` build script, and `aot_tests`;
   - the C directives' `generated` feature, `tape_format`'s `WRITER_RUST`, and the parser tests' dependencies kept
     only for them (`rerail-rust-stream`, `rerail-device`).

   Ours' `rerail-rir` dependencies and test hooks stay (crate split 7a).
4. **`clif_ukl`: theirs' structuring (`shape`), ours' `UKL_FUNC`** (77e2fdb1cd). Theirs' drop of the `absorbed` check
   stands: `ty` already refuses vector types.
5. **The generators: both sides' edits; the outputs regenerated.**
   - Ours' tier-1 generator writes the `plain` region only where a region's `after` reads it, so rr's `tier1.rs` is
     regenerated.
   - The headers are each generator's literal, as the structure gate's manifest has them (the Kotlin tiers).
   - JSON's, rr's and Kotlin's tiers are each their generator's output.
6. **Windows stays:** the C directives' benchmark `cfg`, the parser tests' Windows target, and `rr_parser`'s JSON
   benchmark without the msvc `cfg` (ours' e13b43f703: simdjson builds on Windows).

## Audit

`scripts/check/merge_audit.py`'s 781 lines are items 1-6:
- 652 are `native_cpu_stream`'s, and 30 more are `native_cpu_clusters`' and `native_cpu_tape`'s: the split's moves
  undone (item 2);
- the decision files and rows, renumbered (item 1);
- the removed crates and tests (item 3);
- the generated headers (item 5).

## Checks

On spark2, CPU, one platform:
- `cargo check --workspace --tests` is clean, and `check_crate_dag.py` passes;
- `json_rust_is_current`, `json_rusts_tapes_are_the_jits` and rr's and Kotlin's `rust_is_current` all pass, with rr's
  `rust_` tests.
