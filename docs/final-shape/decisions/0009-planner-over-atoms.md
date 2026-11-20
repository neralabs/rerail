# 0009: The planner lowers programs, statistics and inputs to the same integers

Status: accepted
Date: 2026-10-27

## Question
`logic::plan` (T's planner, Phase 6 row 5) compares and builds symbols as text, as T's facts file
held them. The base relations are now integer vtabs (decision 0008), and `logic::native` holds symbols as
store atoms. How do a program's string constants, the statistics the planner prices with, `@cat`
and its splits, and the outputs meet atom-valued columns (O13)? This affects every rule the logic layer runs
and every target.

## Prior work consulted
- ast-index-spike design index, topic 21: planning resolves names in the query's grammar. With
  the primary grammar's names, a Rust query found 0 results instead of 71 (D S133 L727-747). The index carries global codes (D S071).
  Agreement: agrees; kind and field constants are lowered in the program's dialect.
- History index 2.22: the Python interner was used to look up Rust names in a mixed store
  (DH L61-70). Agreement: agrees, for the same reason.
- History index 2.21 and design index topic 20: interners are re-derived from the grammar and
  fixed at registration (DH L3187-3191, L3319-3343; D S130). Interning on first observation made
  bytes order-dependent. Agreement: agrees; kinds and fields are grammar codes (no interning),
  and names use the store's one append-only table (C12).
- Design index topic 4: planner statistics are persisted and read, never counted at plan time
  (D S174, S178). Agreement: agrees; the statistics are T's tables, keyed by the same atoms as the columns.
- Decision 0008: base relations are integer vtabs, so kinds are kind atoms, fields are field atoms and
  strings are name atoms.

## Decision
- **Each column has a domain.** The domains are int, node, kind, field and name. Declared types
  give int, node and name (`sym`). `node.kind` is a kind column and `field.f` a field column. A
  derived column takes the domain of the variables written into it, inferred to a fixpoint over the
  rules. A variable used in two domains is an error at lowering.
- **Constants are lowered before planning.** A string constant becomes an integer by its domain:
  a name atom (interned), a kind atom or field atom in the program's dialect (absent: `-1`, which
  matches nothing). `.kindset` members become kind atoms. The planner then sees only integers,
  so its statistics are keyed by the same integers: `stat_kind`/`stat_edge` by kind atoms and field
  atoms, `stat_val` by the lowered values.
- **Strings exist only inside three functions** over the connection's mirror of `atoms`
  (`setfix::Interner`): `sym_text(a)`, `sym_atom(text)` (interns) and `sym_cat(a, b)` (`a.b`).
  `@cat`, its splits and `flat_bin`'s operator test are rendered through them. New atoms are
  appended to `atoms` after each statement.
- **The run is on a connection to the content store.** Inputs are the bound vtabs, the statistics
  are an attached schema, and derived relations are `temp` tables. Outputs are decoded by domain.
  T's text mode remains for the planner's goldens over T's facts file.

## Evidence
Correctness check (O13): T's programs over the vtabs of the fixture corpus, with T's statistics
atomized at load, give the same plans as T's goldens. After decoding, they also give the same
outputs.

## Consequences
- A program is lowered for one dialect. A batch of several dialects runs a program per dialect
  (topic 21).
- `stats::ingest` must write the planner's statistics keyed by atoms when it replaces T's tables
  (Phase 3 row 6 to the planner).
- Native strata take the lowered program unchanged (O13 closure).
