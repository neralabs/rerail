# 0519: Each word written once as a method; one room check; refusals coded and cold; the source laid out

Status: decided (the user, 2026-11-20: "please implement all of these things", the five gaps found between the
generated JSON Rust and simdjson's and the lean parser's). Follows 0505, 0508, 0516. Page 40, step 42.

## Intro

**What the comparison found.** The generated Rust for the simdjson form of JSON was set beside simdjson's
`json_iterator`/`tape_builder` and the lean parser's `stage2` (`json_lean/src/stage2.rs`). Five gaps were left:

1. A string's loop (`part_stop`, its content, its escapes and their `\u` checks) was written again at every place a
   string is read (the document, a key, an object's value, an array's value): four copies. So were the atom tests
   (`true`, `false`, `null`) and a number's read. simdjson and the lean parser write each once (`parse_string`,
   `is_valid_true_atom`, `parse_number`; `string`, `primitive`) and call it from each place.
2. Each node's `open_node` tested the stack's room (`open_cap`). The lean parser sizes its stacks once, from the
   input, and no node tests them.
3. The tests' language was every key their documents' tapes hold. A grammar's node word none of the files reached
   had no number, and its Rust refused there (`UNNUMBERED`): the source was not the grammar's whole.
4. A refusal was `Err(false)`, with no reason, and not marked cold. simdjson returns a `Code` from each error, after
   `cold_path()`.
5. The source had no indent, its labels were node numbers (`'b21`, `'m129`), and a switch with no byte of its own was
   a `match` with only `_ =>`.

## The decision

1. **Each word written once, as a method called from each place** (`rust_stream`):
   - `string`: a region's loop, where every way of it but its closer's goes back to its part's `TOKEN` by words of one
     way each (`straight_back`). Those ways' common last words come once, after the escapes' `match`. The method
     returns at the closer, and the caller's `TOKEN` is `s.string()?;`, then on from the closer's place. The regions
     of one code share one method (`string_methods`). There is no method where the program has context beside its
     returns.
   - `atom`: which of a run's words the token is, one load of its bytes each, as one more than its place (0 for
     none). The caller's arms `match` on it.
   - `number`: the run, its end and its check in one pass (`run_R_check_C`); where that pass cannot tell, the end by
     the run, then the place's words on to past the check. The caller calls it, then goes on past the check.
   - The methods are shared through `Facts::method`, one method per code, named by its role (`atom`, `number`,
     then `atom_1`, …).
2. **One room check for the stack.**
   - `stacked` returns the most nodes a rule holds open at a word (`OneFn::most`).
   - `rust_stream::open_bound` is `(DEPTH + 1) * most + DEPTH`: calls nest at most `DEPTH` deep, and each keeps its
     container open.
   - The host reserves the stack for that bound (`write16`, `CompiledStream::open_bound`), not a guess of 64.
   - The run tests it once, at its start (`W16::holds(OPEN_BOUND)`), and `open_node` no longer tests `open_cap`.
   - Without a bound (no direct run, or calls that do not nest as the nodes), each node tests it as before.
3. **The tests' language is the grammar's.** `rust_stream::direct_entries` lists every key a direct run writes:
   - each `OPEN`'s and `LEAF`'s kind (every keyword's, for a keyword's leaf);
   - its field, and its supertypes' classes, keyed as the run numbers them.

   The JSON and AOT tests build their language from it first, then add the files' keys. The JSON test checks that no
   word is left `UNNUMBERED`.
4. **Refusals have codes and are cold.**
   - `native_cpu_stream::refusal` holds the reasons (`NAMES`), and the source's `mod why` is written from it. `ROOM`
     is the tape's room run out.
   - Every refusal is `refuse(why::…)`, a `#[cold] #[inline(never)]` function, and the rule's error is the code
     (`Result<_, u32>`).
   - The run returns `refusal::MARK | why`, where no word's place is.
   - The host names the reason: `refusal::of` in `Refused::why`, and `refusal::text` in `write16`'s message.
5. **The source is laid out as a person writes it.**
   - Every rule body and method is indented four spaces a brace (`indented`). Blank lines go, and a statement's
     closing braces each get a line.
   - `Shape::tree_named` names each label (`Names`). The rules name a loop by its word's role and place
     (`'token_130: loop`), a merge `'to_…` (`break 'to_close_22;`), a dispatch `'entries_N`, and a copy by its number
     after.
   - `if`/`else` arms and `match` arms are multi-line.
   - A switch with no byte of its own is its default alone, with no `match`.
   - A run whose token is no word refuses without making its end.
   - Nothing follows an unnumbered leaf's refusal.

## Checks

- `json_tests::simdjson_forms_rust_trees_are_the_jits`:
  - one `fn string`, one `fn atom` and one `fn number`, called from each place;
  - one `part_stop` in the source;
  - each number's arm is `s.number(st)?;`;
  - no `UNNUMBERED` word;
  - the tapes equal the JIT's. The textual checks read the source with each line's indent dropped (`flat`).
- `aot_tests::grammars_rust_tapes_are_the_jits`: every grammar's Rust in formats 15, 16 and 16z writes the JIT's
  tapes and refuses where it refuses, with the language from `direct_entries`.
- `rerail-rr-aot` (generated, on a farm host): rr's AOT tiers, 21 tests.
- The compiler's and clif's `rust` tests.

## Also

- Two clippy findings in `json_tests.rs` fixed (a range loop over `parent`, a needless reference).
