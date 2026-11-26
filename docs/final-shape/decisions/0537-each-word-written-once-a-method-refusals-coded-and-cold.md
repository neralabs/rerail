# 0537: Each word written once as a method; one room check; refusals coded and cold; the source laid out

Status: superseded by 0565 (audited 2026-11-23). The five items are the user's (2026-11-20: "please implement all of
these things"). How each is done is the implementer's choice, not the user's, and is marked so below. What the tests
hold is under "Checks". What they do not hold is under "Not checked". Follows 0532, 0533, 0535. Page 40, step 42.

## What the user asked

The gaps found when the generated JSON Rust was set beside simdjson's and the lean parser's
(`json_lean/src/stage2.rs`):

1. Write each word once, as a method called from each position (string, atom, number).
2. Hoist the room and full checks out of `open_node`, as the lean parser's up-front bound does.
3. Fix the test language.
4. Give the refusals codes and mark them cold.
5. Indent the output and name the loops.

## What was done (the implementer's choices)

1. **Methods** (`rust_stream`):
   - `string`: a region's loop, where every way of it but the closer's goes back to its part's `TOKEN` by words of
     one way each (`straight_back`). The caller's `TOKEN` is `s.string()?;`. Loops of the same code share one method
     (`string_methods`). There is none where the program has context beside its returns.
   - `atom`: which of a run's words the token is (one more than its place, 0 for none). The caller matches on it.
   - `number`: `run_R_check_C`, and where that cannot tell, the run's end and the words on to past the check.
   - The names and the sharing rule (`Facts::method`: one method a code, `base`, then `base_N`) are the implementer's.
2. **The stack's room.**
   - `stacked` also returns the most nodes a rule holds open at a word (`OneFn::most`).
   - `open_bound = (DEPTH + 1) * most + DEPTH`. This formula is the implementer's. It assumes each of at most `DEPTH`
     nested calls keeps its container open, and that no rule holds more than `most`. Nothing proves it; the checks
     below test it at the depth's limit.
   - The host reserves that depth (`write16`). The run tests it once, at its start (`W16::holds`), and `open_node`
     tests it only where there is no bound.
   - The lean parser sizes its stacks once, at `DEFAULT_MAX_DEPTH + 1` (`json_lean/src/lib.rs`). Its tape bound,
     `len + len/4 + 2`, is a different thing; the format 16 writer's node room was already reserved by the host.
3. **The test language** is every key a direct run writes (`rust_stream::direct_entries`), then the files' keys.
4. **Refusal codes.**
   - The 18 reasons (`native_cpu_stream::refusal::NAMES`), and which reason each refusal site gets, are the
     implementer's. Each site's reason was chosen by reading its code.
   - Each refusal is `refuse(why::…)`, `#[cold] #[inline(never)]`. The run returns `refusal::MARK | why`, and the host
     names the reason (`refusal::of`, `refusal::text`).
5. **Layout.**
   - Rule bodies and methods are indented by brace (`indented`).
   - Labels are named by `Shape::tree_named`. The scheme is the implementer's: a loop `'ROLE_PC`, a merge `'to_ROLE_PC`,
     a dispatch `'entries_N`, a copy `_K` after.
   - A switch with no byte of its own is its default alone, and no dead code follows a refusal.

## Checks

- `json_tests::simdjson_forms_rust_trees_are_the_jits`:
  - one `fn string`, `fn atom` and `fn number` each, called from each place, and one `part_stop`;
  - no word `UNNUMBERED`;
  - the tapes equal the JIT's on every input, now with documents nested 1,022 to 1,024 deep (arrays, objects, both
    by turns). That is the open-node bound at the depth's limit.
  - The refusals' reasons, on files the JIT refuses as well:
    - `[1 2]`, `{"a" 1}`, `[tru]` and `["\q"]` give `BYTE`;
    - `[01]` gives `CHECK`;
    - `"open` and `["\` give `EMPTY` (tier 1's total of open regions, tested by the rule's first word);
    - an array, an object and the two by turns, nested 1,025 deep, give `DEPTH`.
- `aot_tests::grammars_rust_tapes_are_the_jits`: every grammar's Rust in formats 15, 16 and 16z writes the JIT's
  tapes, with the language from `direct_entries`.
- `rerail-rr-aot` (generated, on a farm host): 21 tests.

## Not checked

- These reasons are reached by no test: `ROOM`, `WORD`, `SPAN`, `END`, `STRING`, `TOKEN`, `RETURN`, `COUNT`,
  `STOP`, `NUMBER`, `WRITER`, `NODES`, `WIDTH`, `FIELD`. `STRING` is not reachable in JSON: `EMPTY` refuses an open
  region first. Their names say what each site tests, as read from its code, and nothing more is claimed.
- `open_bound` in a grammar other than JSON's, beyond its tapes equalling the JIT's in the AOT test's documents (none
  nested near the limit).
- Nothing is claimed about speed: nothing was measured.

## Resolved: the depth's limit

Past 1,024 levels the JIT and the Rust disagreed. On JSON, the JIT's call (`Push` without native calls) stored its
return with no depth test, and so did the interpreter (`stream::run`), so both parsed a document nested 1,025 deep.
The Rust refused it, as simdjson and the lean parser do. The difference came in with `80e54a2a9d`, and this step's
deep documents found it.

The user (2026-11-20): "there should be a 1024 limit". Every call in every run is now limited to
`native_cpu_stream::DEPTH` (1,024) nested calls: the JIT's plain call and the interpreter's `PUSH` test it, as the
JIT's native call and the Rust already did. The checks run documents nested 1,022 to 1,027 deep. Those to 1,024 give
the JIT's tapes; those past it are refused by the JIT and by the Rust, for `DEPTH`.
