# 0550: The lowerings of an intermediate form into Rust are deprecated

Status: decided (the user, 2026-11-21, merging the hub into Swift-Kotlin-Scala-support: "please keep them
in, at a minimum, but clearly mark them deprecated. I will tell the other trees to merge and then remove
their use"). Follows 0415 (Kotlin's generated parser in the lean parser's shape).

## What

The Bitlex compiler writes a parser's Rust directly, from the grammar: tier 1 from the lexer's statements
(`bitlex_lex_rust`), tier 2 from the tree rules (`bitlex_rust`). That is how a grammar's parser becomes
Rust. These modules make Rust some other way, from an intermediate form, and are deprecated:

| Module | Makes Rust from |
|---|---|
| `rerail_compiler::rust_stream` | the native IR of tier 2 (`Structure`) |
| `rerail_compiler::clif_rust`, `rerail_clif::rust` | the JIT's Cranelift IR |
| `rerail-rr-aot` | `.rerail`'s parser, through both |
| `rerail-pp-aot` | the preprocessor's words, through `clif_rust` (`native_cpu_pp::rust_source`) |
| `rerail_tape_format::WRITER_RUST` | the writer's ops, pasted into `rust_stream`'s output |

Kotlin's are gone (rust_block, kotlin_aot's IR build): Kotlin is the direct generator's.

## Rules

- No new use. A grammar that needs Rust gets it from the direct generator, which grows to cover it.
- The uses that remain are two branches' work: Go-C-and-C-support's preprocessor (`pp_aot`, c_directives'
  `aot_build`, its tests) and Parser-generators-final-shape's JSON and `.rerail` Rust (`rust_stream`,
  `rr_aot`, `parser_tests`' `aot_tests` and JSON tests, rr_parser's tier 1 Rust tests). Those branches merge
  this one and remove their uses; then the modules go.
- The JIT shares analyses with `rust_stream` that are not a lowering: `WordForm`, `word_form`, `marks`,
  `direct_keys` and `open_bound` (the last two from `rust_stream`'s facts of a program). Before the modules
  go they move to `native_cpu_direct`, the JIT's own: after Parser-generators' merge, which still changes
  them (0541-0546, 0577, 0578).
- Each deprecated module says so in its first line, with this record's number.
