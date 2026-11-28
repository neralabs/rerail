# 0713: A template's code nests by the level, and Kotlin reads it so

Status: decided by the user (2026-11-26, roll-up 136: "Level + depth"), built in the roll-up. Replaces 0621's
`nests O .. C depth D` and the JIT's stack of open records (0621, 0635). Keeps 0698's `level … nest`, and 0625's
kind that nests itself.

## Intro

A string template's code holds brackets, and strings with templates of their own: Kotlin's `"${ "${a}" }"`,
Python's `f"{f'{a}'}"`, a `{ {k: v} }` in an interpolation. Two feeder lines built this in the same week.
Swift-Kotlin-Scala-support (0621, for Kotlin) kept one stack of frames, each a template or a bracket, and the JIT's
token loop kept a stack of open records with a copy for a wait (`open_slot`, `OPEN_DEPTH`, `OPEN_COPY`).
Parser-generators (0698, for Python) kept a bracket level and a template depth: two made bits at each region's
opener, which the token loop reads to keep one record word per depth (`OPEN_SLOTS`). Go-C-and-C-support merged
the first onto a hub that had the second. The two are written to the same lines, so the merge had to keep one.
The user chose the level and depth.

## Decisions

1. **One surface: `regions H level O to C max N nest M depth A B else D`.** `max` bounds the frames open in
   templates' code, templates and brackets together, as 0621's `depth` did; `nest` bounds the templates alone.
   `nests O .. C depth D` (`RegionNest`) is removed from the syntax, its body and its backends.
2. **Kotlin's grammar is ported**: `regions hidden level lbrace to rbrace max 4 nest 4 depth t_d0 t_d1 else
   t_deep`. Four frames, as 0621's four levels (0621: eight did not finish a cold compile in four minutes).
3. **A template's `{` is its closer's last byte**, hidden: `or t_dollar bytes 2`, `or t2_dollar bytes 3`, as
   Python's `{` is its f-string's second closer. Otherwise the level would count the template's own `{` as a
   bracket. Kotlin's `t_lex`, `t_brace` and their assume go: the `{` is hidden, so no code. The string's parts
   already read `${` as two bytes (`interpolation_expression_start = t_dollar bytes 2`).
4. **The level accepts a kind that resumes** as well as one with `after` (`syntax.rs`): Kotlin's tails resume
   their strings.
5. **A kind that nests itself (0625) in a group with a level is the group's rounds, with no walk.** Kotlin's
   block comments nest (`nests 3 else c_deep`), and no level walk knew such a kind. The masks compiler
   (`bitlex_masks::nesting`) writes 0625's levels as carries of the level's group, `K_lv1..K_lvN` and the
   innermost open before each byte (`K_in0`, which a reader's closer reads), as the regions' body does without a
   level. The group then pushes no `Walk`, so the CPU runs its rounds as it ran 0625's, and Metal runs them as it
   runs every group. Without a level such a kind stays the regions' body (`nested_regions`).
6. **The evaluator's level walk reads what the backends read** (`eval::walk_regions`): a closer past its
   opener's bytes (`Region::opens`, a raw string's `"""`), and a kind that nests itself, its closer the first with
   none of its levels open and its `else` an opener past them.
7. **Tier 1 as Rust lowers a level** (`bitlex_lex_rust`): its walk keeps the frames (`nest`, `depth`), the
   templates among them (`templates`, to `nest M`) and the depth bits at each opener. A `}` on a template's
   frame sets the `else`, as the evaluator does. A frame past `max`, or a template past `nest` where `nest` is
   the lower bound, is refused by the token loop where it would push the frame: the `else`'s escape on that
   mask is not written (as a kind's own levels past `nests`). The old refusal ("regions with a level, not
   lowered to Rust yet") goes.
8. **The JIT's token loop keeps a string's open record by its depth** (`OPEN_SLOTS`, 0698). The stack of open
   records and its copy for a wait (0621, 0635) are not taken. The generated Rust's token loop keeps its own
   frames, so it holds the open records as the first `opened` of an array as long as `nest M` (innermost last),
   refusing a push past it and a tail with none open.

## Checked

Relax, on this Mac (no device tests; Metal runs the group as rounds, unchanged here, and is not run):

- `rerail-kotlin-parser --lib`: 89 passed, 11 ignored. Its Rust (`kotlin_aot`) and shipped answers written
  again (`rust_is_current`, `the_shipped_answers_answer_the_grammar`).
- Kotlin's wide corpus (`lean-corpus/kt200`, 1,266 files): 1,252 the same, 0 refused, 0 differ (14 the lean
  parser refuses). The parser as Rust: 1,252 the same, 0 refused, 0 differ; tier 1's records differ in 0.
- `rerail-python-parser --lib`: 27 passed (answers written again). Its corpus (1,644 files): 1,642 the same,
  2 refused, 0 differ; 0 whose rounds differ from the walk.
- `rerail-bitlex --lib` 26 passed; `rerail-compiler --lib` 46; `rerail-cpu-stream --lib` 74;
  `rerail-rr-parser --lib` 58 (its device tests skipped; `rr_aot` written again, unchanged);
  `rerail-parser-tests` json and rr 18; `rerail-json-aot` 2; `rerail-c-directives` answers 3 (the directives'
  answers written again: 244 questions the merged compiler asks).
- `relax check` of every crate the merge touches: 0 errors, except `rerail-parser-tests` without
  `test-hooks` (its `json.rs` imports `ladder`, a test-hooks module).
