# 0600: A string with templates is one token

Status: done (the user, 2026-11-21: "go ahead with the full change", on "Template code is still read as
separate tokens, where lean reads the whole string, interpolations included, as one token"). Follows 0598.

## Intro

**The problem.** Tier 1 cut a string with templates into tokens at each template:
- the string's record ran from its `"` to the first `${`;
- the template's code was records of its own;
- each `}` that ends a template opened a tail region (`tail`, `rtail`), a record of its own up to the next `${`
  or the closing quote.

So tier 2 read a string as several tokens, its parts split from each one. The lean lexer reads a string with its
templates as one token (`string_end`, `interpolation_end`). Its parser walks the token's bytes and lexes each
template's code where it reaches the `${` (`parse/expr.rs`, `string_literal`, `interpolation`).

## The decision

- **`resumes K …`**, a region's word in place of `after K …`: it has the same openers (0598), and the region is
  the rest of the token whose region its template closed.
- **Tier 1: one record a string.**
  - A read that closes at a template's `$` (`ReadArm::template`, the region's `or`) leaves its record open over
    the template's code.
  - The read of a region that resumes (`Reader::resumes`, at the template's `}`) makes no record. It ends the open
    record, or, closed at another `$`, leaves it open over the next template.
  - The template's code's records lie inside the string's. The record pass accepts a record inside an open
    string's record and nowhere else.
- **Tier 2: the string's parts, then the code's records, then the parts again.** The parts stop at a `${`. The
  code's records are read up to the template's `}`, the next opener of a reader that resumes (a tier-1 mask),
  where the parts go on. This is how the lean parser reads a template.

## Steps

1. The interpreter's tiers (`records::walk`, `records::pass`, `stream.rs`). Tested on Kotlin's grammar with
   its tails' `after` replaced by `resumes` (`a_string_with_templates_is_one_record`): the string's record runs
   to its last quote, there is no record at a template's `}`, and the tape is the lean parser's.
2. The CPU's tier 1 and tier 2 (`native_cpu_block`, `native_cpu_stream`).
3. Tier 1 and tier 2 as Rust (`bitlex_lex_rust`, `bitlex_rust`).
4. Metal.
5. Kotlin's grammar: `resumes`.

## What changed (step 1)

- Bitlex: `Region::resumes`.
- The compiler:
  - `ReadArm::template`, `Reader::resumes`, their rir text and their outputs;
  - `records::read_end`, `walk`, and `pass`'s nesting check;
  - `stream.rs`'s resume at the template's `}`.
- `kotlin.bitlex`: `tail`'s closer is `dq_close or t_dollar`, which means the same, so its `or` is its template.
- `bitlex_lex_rust`: a closer whose `or` has its own length is one expression, and a region's comment shows its
  `or`.

## What changed (step 2)

- Both tiers make one level of templates: a template in a template's code refuses the file (the interpreter's
  walk too). Kotlin's grammar refuses that already ("a `${…}` template with … a template").
- `native_cpu_block`, the CPU's tier 1:
  - A template's arm and a resuming reader's arm set flags in the exit's refusal value, whose sign alone refuses
    (`TEMPLATE_FLAG`, `RESUME_FLAG`). Only those rules' exits test them.
  - The open record's cursor is a word of the file's (`open_slot`, `RecPart::open`), stored and read only on
    those paths. A read that resumes writes its last into the open record, and stays where it is: its own record,
    written, is the next token's place. The gap marks' state is as the token before left it.
- `native_cpu_stream`, the CPU's tier 2: `Mem::Strings` counts the strings open over a template's code.
  - A template reader's closer piece ends at the closer's own last (`closer_last`).
  - With a string open, `split_resume` enters the resuming reader at its opener where that comes before the next
    record. The opener's code is the reader's tag.
- The test also checks the CPU's records and codes, and its tier 2 over both tier 1s, with templates across
  blocks' edges.

## What changed (steps 3 to 5)

- Kotlin's grammar: `tail … resumes str`, `rtail … resumes rstr`. The test reads the crate's grammar, out of the
  slow tier.
- Tier 1 as Rust (`bitlex_lex_rust`): a string's row that closes at a template keeps its record's index (`held`).
  A row that resumes writes its last there and goes on past its token with no record of its own.
- Tier 2 as Rust (`bitlex_rust`): `P::strings` counts the strings open, set by the closer of a split.
  - `advance`, past each token, enters the resuming reader (`resume_R`) at its opener where that comes before the
    next record. Its pieces stand at the record before that one, so the next token's is as after any split.
  - A reader that resumes has no `split_R`, as it has no record.
- The JIT's tier 1 reads a token again where the loop does (a wait, a block read again), so the open record's
  slot is left as one read leaves it. A string ended keeps its record's cursor, marked (`STRING_ENDED`): a second
  read of its tail patches the same record. A template's close finds no string open, or its own.
- The masks' records, which a device reads (Metal: its record pass codes them, and the JIT's tier 2 reads them):
  - A region's record is its opener alone (page 40). A reader that resumes has none, start or last.
  - The loop's records nest, the string's last after its code's records, which no mask holds. So a grammar that
    resumes a region with no tier 2 split is refused.
  - `split_enter` enters a split region at its opener, the record whole or its opener alone. The parts end at the
    closer's own last (`closer_last`), as `records::read_parts` ends them, not at the record's last. Until now
    the masks' records of a Kotlin string were tokens as they were, and tier 2 refused them, with or without
    templates.
- Metal's code is unchanged: its record pass codes the masks' records, and its tier 2 is the JIT's. The test
  runs that path on the CPU: the masks' positions, coded by `records::pass`, then the JIT's tier 2. Kotlin has no
  Metal test.
- The test: every tier on every text: the interpreter, the JIT's tier 1 and tier 2, the masks' records, and tier 1
  and tier 2 as Rust. The texts include a gap before a template's `}`, a file that ends at a string, and tails
  read across a block's edge.

## What changed (step 6): which `}` ends a template

Until now a template's tail opened at the first `}` on its line, by the bytes (`t_close = rbrace & (prev(t_lex) |
prev(t_body))`), and the grammar refused, by an escape, "a `${…}` template whose `}` is in a string of its code". That escape refused
valid Kotlin (`"${f("}")}"`, `"${'}'}"`), and lean reads them. Now the regions say which `}` ends a template:
only the first `}` outside the code's own strings and characters, while a template is open. The escape is gone,
and `t_close = rbrace`.

- The regions (`block::WalkKind::resumes`, `resumes` in the text form): a kind that resumes, closing at its own
  closer (the string's quote, not its `$`), ends the template the string was in. The walk sets `last` to none
  there, and a `}` after it opens no tail. The JIT's walk (`native_cpu_walk::template_last`), tier 1 as Rust's
  walk and its test build's are the same.
- The rounds (`Lowering::regions`, bitlex's `Regions::body`) have a second carry, `H_ended`. It holds the places
  after the round before's closes of a resuming kind at its own closer (`prev(c & ~C2)`). Those places are added to
  the kinds that end a template's fill, and are taken from the openers of a kind `after` one. `verify_walks` wants
  that carry exactly where a kind resumes. The test checks every output of the rounds against the walk's on every
  text.
- `t_code = fill(prev(t_brace), o_tail | o_rtail)`: a template's code runs to the `}` its tail opens at. Tier 1 as
  Rust keeps it as a span whose stop is regions' openers. A token of a row that reads those regions ends it, and
  its checks run there over the span; a span the file ends in is checked to the end. The rows that resume keep
  their closer (`close`), which the string ended tests.
- `split_enter`: in a grammar with templates, every split region's masks' record is its opener alone, its parts
  no records, so it enters there. In another grammar a record of its opener alone is split already (tier 1's,
  `Split::Tier1`: JSON's A records read by B), a token as it is, as before step 5.
- The texts moved from the refused lists to the test of every tier, with strings, characters and `}` in a
  template's code, and code after a string ended.
