# 0492: A question is asked where a rule branches, and a word's class is one test

Status: decided (the user, 2026-11-19: "rules written as sequences and loops rather than
labelled-block states … judge based on the structure of the code"). Follows 0489 (Kotlin's
generated parser in the lean parser's shape). The first step of that item.

## Intro

**The problem.** The lean parser asks a question about the tokens ahead (`declaration_ahead`,
`accessor_ahead`, `type_args_end`) only in the function that branches on it. The generated tier 2
asked each mark that is not `hidden` in every state, because such a mark refuses the file where no
rule takes it. Kotlin had seven of them. So a state that reads a name (an import's first word)
tested `decl_mod`, `enum_class`, `acc` and the rest at its keyword arms. Each test became an
`if … else if …` chain inside each word's arm, and those arms could not be merged.

The word switch had the same problem. A state that reads a name listed every keyword as its own
arm: 40 arms in `import_header` alone, each a `break` or a refusal. The lean parser reads a word's
class instead, with `soft(word)` for the soft keywords and `ident_word(word)` for the words that
may be names.

## The decision

1. **A mark is asked only where a rule takes it.** In Kotlin's grammar, every mark a rule takes is
   `hidden`: the six added are `decl_mod`, `wild`, `obj_lit`, `ft_dot_at`, `enum_class` and `acc`.
   `generic` stays a guard: a `<` that opens type arguments where no rule reads them refuses the
   file.
   - Corpus: 452 the same, 39 refused, 0 differ (was 450, 41, 0). Two files the guards refused are
     now read, and they agree with the lean parser.
2. **A word switch has one arm per target.** Its words are listed as runs (`w::A..=w::B`), and the
   kind's own target is the default arm (`rust_stream::arms_or`).

The generated tier 2 (`tier2_aot.rs`) is 26,750 lines, from 43,619. `import_header` is 332 lines,
from 689.

## What is still apart from the lean parser

- **Word numbering.** The lean words are numbered by class (hard keywords, then `SOFT` and up), so
  `ident_word` is one compare. Ours are numbered in grammar order, so one class is several runs.
  Next: number the words so each class a switch tests is one run.
- **Labelled blocks.** Each state is still a labelled block and a `d` dispatch, where the lean
  parser has a sequence and a `while` loop. That is the next part of this item.

## Also

The writer's ops (0434 §6, from Parser-generators-final-shape) called each other as direct
branches (`colocated`) in any module. In a JIT module that is not one arena, two ops can be more
than a `bl` reaches apart: cranelift-jit's `compiled_blob.rs:87` assertion, which failed in about
one full compiler test run in two. `writer::define_ops` now takes `near`: the stream passes
`Cut::near`, and `jit::Writer` (one arena) passes `true`.
