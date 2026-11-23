# 0598: A template's tail goes back to the string its template closed

Status: decided (the user, 2026-11-21: "go ahead with the full change", on reading strings as the lean lexer
does; this is its first step). Follows 0596 and 0332 (steps 24 and 35, where `after` and the tails were
designed).

## Intro

**The problem.** A region kind with `after K …` (a template's tail, `tail` or `rtail`) opened only where the
last region *opened*, of the kinds with no `after`, was one of `K`. So the tail after `"""${f("a")}"""`'s `}`
was a string's tail, because `"a"` in the template's code was the last region opened. Three escapes refused what
that rule got wrong:
- "a raw string in a template's code";
- "a string, a character or a backtick name in a raw string's template";
- the tail named `chr` and `tick` too, so a character in a string's template still found its tail.

The lean lexer has no such rule. Its string reader keeps the string's kind over the template's code, as on a
stack (`string_end`, `interpolation_end`), and goes back to it at the template's `}`. The generated Rust kept
`plain`, set at every region a token opened, so every string, character and comment row wrote it.

## The decision

- **`after K …` reads the last template, not the last region.** A kind with `after` opens only where the last
  region *closed at its second closer* (`or`: the template's `$`), of the kinds with no `after`, is one of `K`.
  A string, a character or a comment in the template's code closes at its own closer and leaves that unchanged.
- **The rounds.** For each plain kind with an `or`, `t_K = c & C2`, its closes at the `$`. Then
  `o_tail = O & ~H & prev(fill(t_A | …, t_B | …))` (`Regions::body`, the masks lowering). The walks agree: the
  interpreter's, the CLIF one (`template_last`) and the Rust gate set the state where a close is at `C2`.
- **The kinds `after` names have an `or`** (verified in Bitlex and in the block verifier). Kotlin's `str` is
  `dq & ~dq3 .. dq_close or t_dollar`, which means what `dq_close | t_dollar` meant, since both closers are one
  byte. `tail` is `after str` only.
- **In tier 1's Rust, `template` replaces `plain`.** It is set in the two string rows, at a close on the `$`
  (`template_close`), and read at the `}` that opens a tail. No other row writes it.
- **The two escapes go.** The grammar reads them, and the lean parser's tapes agree
  (`a_tail_is_the_strings_whose_template_it_ends`).

## Why

The criterion is the lean lexer's code: the state its string reader keeps over the template's code is the
string's kind, set when the template opens. The old rule needed escapes because it tracked a state the lean
lexer does not have.

## What changed

- `tools/bitlex/src/syntax.rs`: `Region::after`'s meaning, `Regions::body`, and the validation.
- `block.rs`: the interpreter's walk and the verifier. `native_cpu_walk.rs`: `template_last`.
  `bitlex_masks.rs`: the rounds read the templates.
- `bitlex_lex_rust.rs`: `template` in the walk gate and the token loop (`template_close`); no `plain`.
- `kotlin.bitlex`: `str … or t_dollar`, `tail … after str`, the two escapes and their masks removed.
- `tier1.rs` and `tier2.rs` regenerated, and `kotlin.answers` blessed.
- Tests: `a_tail_is_the_strings_whose_template_it_ends` (new, `same`); the refused texts that the grammar now
  reads moved out of `what_the_grammar_does_not_read_is_refused` and `the_template_spans_are_the_interpreters`.

Kotlin: 57/57 with the corpus; the compiler's 150; bitlex 11/11; `rerail-kotlin-aot` builds without
`test-hooks`.

Next: one token per string, with the template's code in its record. That changes what tier 1 hands tier 2.
