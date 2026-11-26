# 0636: Kotlin's escaped dollars and runs of dollars, as the lean parser reads them

Status: decided (2026-11-23, on the standing order to make the Kotlin parser's Rust read as the lean
parser's does). Follows 0635.

## Intro

**The problem.** The lean parser's string reader (`string_literal`, after tree-sitter's scanner)
reads three things otherwise than Kotlin's own escapes. 0635 left two of them refused, and a probe
found the third:

- **`\$` before the quote** (`"Outer\$"`): the string ends at that quote, and the content of the
  piece before it (from the string's start, or from the last template or name) makes no leaf. The
  1,266-file corpus has one such file.
- **The byte after a `\$` is plain.** So `\$\"` ends the string at the escaped quote. In `\$\$b`, the
  second `$` starts a template, because the backslash after the first `\$` escapes nothing.
- **A run of `$`s that starts no template is one content leaf** (`"$$"`, `"a$$ b"`). The grammar
  made a token of each `$`. Where a template follows the run, each `$` before the template's is a
  leaf of its own, and the grammar already had that.

## The decision

- **Backslashes that escape**: `bs` is every backslash but the byte after a `\$` (`esc0`, the plain
  reading). `esc_dollar`, `dq_close`, `t_dollar` and `s_dollar` take their escapes from `bs`.
  - `bs` comes from `esc0`, and the true `\$` from `bs`. The two disagree only on a `$` after a
    `\$` and more backslashes. Where a backslash follows such a `$`, the file is refused: whether
    that backslash escapes is not known there.
- **`\$` before the quote**: the `\$` is a part of the string's own:
  - `esc_q` where its piece has no content before it;
  - `esc_qd` where a content byte comes before it (`gap_byte`: a byte of the string's body, not
    of a template's name and not a `$`).

  Tier 2 ends a string with `[esc_q | drop esc_qd] '"'`. `drop` takes the content token before
  `esc_qd`, which is the piece's whole content, because the `\$` is in the same piece. Both
  kinds are implied.
  - A token mark could not say this. Tier 2 splits a string's record into its parts, and a token
    mark between parts never holds.
  - A closer kind of its own (`q_esc`) could not say it either. A reader's closers take their
    kinds from their first byte.
- **Runs of `$`s**:
  - `t_run` holds on a `$` whose run, of up to four, ends where a template starts (a name's
    letter or `{`).
  - Otherwise the run is one `string_content` (`d_first` to `d_last`).
  - Runs of five or more are refused.
- **Tier 2 as Rust** (`bitlex_rust.rs`) writes `drop` (`drop_made`). The node made last at the
  level comes off the tape's end, as the interpreter's `DEAD` node is left out. Where that node is
  not the last on the tape, the file is refused.
- **`named`'s two lines are one** (a `before` with `with`, from 0635). The program holds 64 mark
  lines at most, and this frees one.

## Why

The lean parser's reader is the criterion. Its `\\` arm passes the byte after `\$`, and ends the
string at a quote there with no content leaf. Its `$` arm writes a run that starts no template as
one leaf.

`drop` keeps the decision in tier 2, where the string's tokens are. The part's two kinds give
tier 2 what it cannot look back for: whether a content token comes before the `\$`.

Left refused:

- `$$"…"` strings, whose templates take as many `$`s as their prefix;
- a `\$` before a backslash and then a quote;
- runs of five or more `$`s.

## What changed

- **`kotlin.bitlex`**:
  - the masks `esc0`, `bs`, `esc_q_at`, `gap_byte`, `d_start`, `t_run`, `d_run`, `lone1`,
    `d_first` and `d_last`;
  - the parts `esc_q`, `esc_qd` and the run's `string_content`;
  - `string_literal`'s end, the `implied` line, and `named` as one line;
  - the escapes for an unknown backslash and for runs of five or more.
- **The compiler**: `bitlex_rust.rs`, `drop` in tier 2's Rust (`drop_made`).
- **Regenerated**: `kotlin.answers`, plus `kotlin_aot`'s `tier1.rs` and `tier2.rs`.
- **Tests**: `escaped_dollars_and_dollar_runs_are_the_lean_parsers`.
