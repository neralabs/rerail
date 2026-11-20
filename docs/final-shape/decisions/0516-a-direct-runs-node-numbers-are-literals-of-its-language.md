# 0516: A direct run's node numbers are literals of the language it is generated for

Status: decided (the user, 2026-11-20: "please address these", of 0508's §6, where D7 was left). Follows 0508,
0510. Page 40, step 41.

## Intro

**What D7 was.** In a generated direct format 16 run, each node's number in its language's table was loaded at run
time: `number16` read `numbers[k]`, the key's slot in an array the host built from the language it passed with the
document. simdjson writes a container's character as a constant, and the lean parser a kind's constant. The numbers
could not be constants because the generator did not know the language: the tests made one table a file (format
15's first-use table), after the Rust was written.

A language's table is fixed in production. Format 16 ships one per language (`tables/<tag>.tsv`, `f16::lang`), and
the engine writes every document of that language with it (`tape::formats::run_16`).

## The decision

1. **The generator takes the language** whose numbers a direct run writes: `rust_stream::rust_source_16(native,
   widths, tag, lang)`. Its numbers by key are `native_cpu_stream::numbers_by_key`, the same function
   `CompiledStream::numbers` gives the JIT's run.
2. **Each node's number is a literal at its word.** `OPEN` is `s.open16(N)?`. `LEAF` is `s.leaf16(N, end)?`, and a
   keyword's leaf, whose kind is its token's word, is a `match` of the word to its number, as the lean lexer's
   keyword `match` gives a kind. `number16` is removed, and the rule reads no table.
3. **A key the language lacks refuses the file at its word** (`rust_stream::UNNUMBERED`), as the JIT's run refuses
   it at `fail::NUMBER` when the word is reached. A keyword the language lacks falls to the `match`'s refusal.
4. **The source states its language**: `LANG`, the fingerprint of the table's text (`Lang::to_text`). It is part of
   the source the AOT build's fingerprint check compares, so a changed table is a changed program.
5. **The languages the sources are generated for:**
   - rr's AOT build (`rerail-rr-aot`): rr's shipped table (`f16::lang(RR_TAG)`, `rr_aot::lang`), production's;
   - the tests (`json_tests`, `aot_tests`): one language a grammar, every key its inputs' format 15 tapes hold, made
     before the Rust is written, in place of one table a file. The reference writer's tape and the JIT's are
     written with the same language.
6. **A window run** (not `coded`: the nodes passed from the rows at commit points) still reads the numbers by key
   at run time, as the JIT's run does: its keys are a row's, not the word's.

## Checks

- `json_tests::simdjson_forms_rust_trees_are_the_jits`: the source holds no `number16` and states `LANG`. The rule
  reads no numbers. Every node word is a call with its literal or an `UNNUMBERED` refusal. Every parsed file's 16
  and 16z home is the JIT's and the reference writer's.
- `aot_tests::grammars_rust_tapes_are_the_jits`: every grammar's Rust writes the JIT's tapes with the grammar's
  language.
- `rerail-rr-aot`'s `aot_format_16_tapes_are_the_jits`: the corpus with rr's shipped table.
