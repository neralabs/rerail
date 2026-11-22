# 0542: The token's state is the rule's local, not the run's fields

Status: done by the implementer. The item is the user's (2026-11-20, "address each of these in turn", the third of
0526's "Left"): "The token's state lives in struct fields. start, rec_end, next, last_end, code and pending are fields
of S, where the lean parser keeps v, end and next as locals". How it is done is the implementer's choice. Follows
0526, 0541. Page 40, step 46.

## The decision

1. **The token's state is a struct of its own, `T`**: `next`, `start`, `end`, `rec_end`, `code`, `last_end`, `at_end`,
   `need`, `live`, and `part_at` where a string's place is not its method's argument (0526). `S` keeps the run's
   buffers, the writer and the nodes' state.
2. **The one rule holds it as a local** (`fn r0(s: &mut S)`, its first line `let t = &mut T::NEW;`), as the lean
   parser's `walk` holds `next`, `v` and `end`. Where a program has several rules, the entry holds it
   (`let mut t = T::NEW;`) and each rule takes it (`t: &mut T`), since the token's state crosses the calls.
3. **A step that moves the token's state takes it** (`token_all(&mut self, t: &mut T)`, `string(.., t)`,
   `number(st, t)`, the checks, `accept`, a lazy mark's test). **A step that only reads it is given the values it
   reads**, as the lean parser's `open(…, start)` and `leaf(…, start, end)`: `open(kind_field, start)`,
   `leaf(kind_field, start, end, last_end)`, `open16(number, start)`, `leaf16(number, start, end)`,
   `close(end)` (the token before's end, `t.last_end`, where the node's own end is not given), `word_kind(word)`,
   `pattern_N(next)`, `refusals(code, next)`. `t` is the last argument, so a value read from it is read before it is
   lent.

`pending` stays a field of `S`: it is the writer's (the deferred extras), not the token's. In a direct format 16 run
no leaf is deferred, and its writes there are 0526's fifth gap, done in its own step.

## Checks

- `json_tests::simdjson_forms_rust_trees_are_the_jits`: `fn r0(s: &mut S)` begins `let t = &mut T::NEW;`; `S`
  has no `start`, `rec_end` or `last_end`; the methods' forms (`s.string(t.start + 1, t)?`, `s.number(st, t)?`,
  `s.check_1(t)?`); the tapes equal the JIT's, as before.
- The parser tests (85, all but the benches, Metal and the corpus), with `aot_tests::grammars_rust_tapes_are_the_jits`:
  every grammar's Rust, one rule or several, compiles and writes the JIT's tapes.
- `rerail-compiler`'s tests (127).
- `rerail-rr-aot` (generated, on a farm host): 15 pass, 6 ignored.

## Not checked

- Whether the compiler keeps `T`'s fields in registers: no machine code was read.
- Nothing about speed was measured.
