# 0627: Names past ASCII

Status: decided (2026-11-22, on the standing order to make the Kotlin parser's Rust read as the lean
parser's does). Follows 0600.

## Intro

**The problem.** The grammar refused every byte past ASCII outside a comment, a string or a character. The
lean lexer reads a name's letters past ASCII. Its `name_end` decodes each character and reads it by
`letter`:

- a letter in the scripts where it is sure (Latin-1 and Latin Extended, Greek, Cyrillic, kana, CJK, Hangul)
  goes on the name;
- a space or a punctuation mark ends it;
- any other character is refused.

Such a name is never a keyword (`tick`). In a string, its parser starts a template's name at any alphabetic
character past ASCII. So `"$é"` and `"$aé"` were read, but differently: the grammar's template name stopped
before the `é`. A `$` before a backtick was refused, though lean reads it as content.

## The decision

- **`uni`**: the UTF-8 bytes of each letter `letter` is sure of. They are computed from Rust's
  `char::is_alphabetic`, which `letter` reads, over its ranges, and written as classes of lead, second and
  third bytes. The non-letters in those ranges:
  - `d7 f7`;
  - `375 378-379 37e 380-385 387 38b 38d 3a2 3f6`;
  - `482-489`;
  - `3040 3097-309c 30a0 30fb`.
- **Escapes**: refused are a byte past ASCII in code that is not `uni`'s, and a template's name with such a
  byte. Also refused, as lean refuses them, are a name's letter touching a number (`1é`) and a name of such
  letters before an `@` (never a label).
- **The masks of names read `high`**, one byte, for a letter past ASCII: `w`, `word_run`, `s_word`,
  `id_dollar`, `lone`, `sfx_start`, `as_run`, and the assumptions at starts. In a file that is read, a byte
  past ASCII in code or in a template's name is a letter's, by the escapes. Reading `uni` there, the masks
  would look two bytes on either side. The writer's fill analysis enumerates the byte atoms at each such
  offset, and the compile did not finish in 25 minutes.
- **A `$` before a backtick**: the escape goes. `lone` (a `$` before no name) already says it.
- **Tier 1 as Rust**: `holds_after` enumerates the atoms of the classes its mask reads, not every class's.

## Why

The criterion is the lean lexer's `name_end` and `letter`, and the parser's template start. The one case
left is a template's name before a byte past ASCII that is no letter (`"$a€"`, `"$€"`). The lean parser
reads it, ending the name there. The grammar refuses it, as its name masks read every such byte as a
letter's.

The cold compile takes about 260 s against 140 s, proving the new masks once. The shipped answers keep the
warm compile as it was.

## What changed

- `kotlin.bitlex`: the classes, `uni`, the masks and escapes above. `tier1.rs`, `tier2.rs` and
  `kotlin.answers` regenerated.
- `bitlex_lex_rust.rs`: `classes_read`, and `holds_after` over a mask's own atoms.
- Tests: `names_past_ascii_are_the_lean_parsers` (Latin, Greek, Cyrillic, CJK, Hangul and kana names, first
  and after ASCII bytes, in templates, after `as`, a `$` before a backtick). The refused list has `a×b`,
  `b·c`, `1é`, `é@f`, `"$€"` and `"$a€"`.
