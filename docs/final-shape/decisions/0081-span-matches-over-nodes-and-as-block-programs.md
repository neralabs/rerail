# 0081: Span captures over a node's own text, and span regexes as block programs

Status: accepted (2026-11-08). §1–§2 built; §3 built as a prefilter run by the CPU interpreter, its device launch
not wired.

(Numbered past 0080, which Parser-generators and wolfeel use for tier 2.)

## Intro

A regex capture can be a span: a piece of the file with a place, a line and a text, that a finding can sit on.
Decision 0054 made captures spans over two kinds of text: a document's whole source, and each of its lines. Two
things were left open:

- `match_all` over a node's own text (a string's contents, a comment), with captures as spans, was refused. A scan
  of each node's text is the per-node scan that 0041 and 0042 forbid. RUF001, RUF002 and RUF003 (look-alike
  characters in strings, docstrings and comments) stopped there.
- The scan over a source or a line ran the host regex. Decision 0041's chunk-at-a-time automata, which the device
  runs, were not used.

```text
// RUF001: a word of a string's contents, then a look-alike letter in that word.
confusable_text(t) :- kind(t, "string_content"), …                      a node's own text
confusable_word(t, w) :- confusable_text(t), match_all("(?<w>[\p{Alphabetic}\p{N}]+)", t), …
confusable_letter(w, ch) :- confusable_word(_, w), match_all("(?<ch>[ıſƄ…])", w).   a capture's text
```

## Decision

### 1. Captures over a node of a kind, and over an earlier capture

`rx_span(p, c, j, g, m)`'s span `c` may now be:

| `c` bound by | its spans (`front::SpanIn`) |
|---|---|
| `source(d, c)` | the document's text (0054) |
| `src_line(d, l, c)` | each line (0054) |
| `kind(c, "comment")` | each comment (0054) |
| `kind(c, K)`, any other constant kind | each node of kind K (`SpanIn::Kind`) |
| `rx_span(P, _, _, G, c)` | each span group G of pattern P captured (`SpanIn::Capture`) |

The binding can go through derived relations' columns, as in 0054. A column that different rules bind differently
(string contents in one rule, comments in another) holds each: the pattern runs over both.

The batch makes each pattern's spans after the pattern whose captures it reads (`CoreRows::matches`).

### 2. Each distinct text is matched once

A node's or capture's text is matched once per distinct text in the batch, and its matches placed at each span
that holds that text (`CoreRows::rx_memo`, texts up to 4 KB). That is 0042 §2: "where the text has a vocabulary,
the test runs once per distinct string". The texts here are a vocabulary's: string contents, comments and their
words. A line is memoised the same way. The source is scanned once per pattern, as before.

The lowering needed two fixes:

- An aggregate over `match_all` (`n = count : { match_all("(?<c>…)", w) }`) is a helper over the match's groups,
  which the body names nowhere (`body_vars`). Until now it counted one row per text.
- The helper's context is the rule's own literals, not inputs alone. `text(w, x)` binds `w`, but not to the spans
  `w` holds, so `span_patterns` could not find its containers.

### 3. A span regex is a block program: the bytes a match can end after

`transducer::packed::regex_block` compiles a source or line pattern into a block program, the transducer's side
program over bytes (`block.rs`). It is straight-line word operations, run by every block executor: the CPU
interpreter, Cranelift, and Metal's interpreter and generated MSL, each lane a chunk of bytes. The translation is
Parabix's bitstream regex (Cameron et al., PACT 2014):

```text
// marker m: bit i set when a match so far can end just before byte i
start            m  = within
byte set C       m' = prev(m & C, 1)
x | y            m' = x(m) | y(m)
C*               m' = fill(m, !prev(C, 1))     // one add with carry per word
other x*         m' = fill(m, 0)               // every later position: wider
^ $ (?m)^ \b     m' = m & where they hold      // from within, prev and eq '\n'
```

The output is delayed one byte, so a match ending at the file's end has a bit.

- A Unicode class is its UTF-8 sequences, as one byte set per position for each length.
- Ends are kept at character edges: not before a continuation byte.
- The marks are a superset of the ends the host's regex finds. They are exact for patterns of literals and
  one-byte classes with no look-arounds.
- A pattern that matches the empty string, or that grows past 2,048 operations, has no program and is scanned as
  before.

The batch runs each source or line pattern's program over the file first. A line with no mark in it is not
scanned; a file with none is not scanned at all. The host regex runs, captures and all, only where a match can
end. `RERAIL_SPAN_PREFILTER=off` turns this off.

Tests:

- `a_patterns_block_program_marks_every_end_and_exact_ones_only_those`: 12 patterns over six texts. The marks
  contain every end `find_iter` gives; for the exact patterns they equal every end of every matching substring.
- `a_patterns_block_program_runs_alike_on_every_executor`: the CPU interpreter, Cranelift, and Metal (interpreter
  and generated MSL, every lane width and class strategy) give the same words.

Measured: `noqa-sec-ratchet.rr` (`match_all("(?<m>SEC\d+)")` over every line) on one 4.4 MB file of 60,000 lines.
CPU time is 12.9–13.4 s user with the prefilter and 12.9–13.3 s without (debug build, load 64). This pattern starts
with a literal, so the host regex already skips to `SEC` with a SIMD search and the block program saves nothing.
The block program pays where the host has no literal to search for, and on the device, which scans the bytes
anyway.

## What the prior work says

- **Decision 0041:** text matching is chunk-at-a-time and per file, never per node per byte; device automata are
  chunked, with carries composed across chunks. *Agrees*: §3's program is chunked (`prev`'s halo and `fill`'s add
  carry are the carries), and §2 matches no node's text more than once per distinct text.
- **Decision 0042 §2–§3:** a text test runs once per distinct string where there is a vocabulary; a file is
  scanned once otherwise, with overlapping-mode ends. *Agrees*: §2 is its §2. §3's marks are its overlapping-mode
  ends, computed with bit operations instead of a DFA.
- **Decision 0054:** captures as spans, scanned per file and pattern. *Extended*, to node and capture spans.
- **Lessons ledger item 17** ("a per-row host callback was nearly all of SQL time"): matches are made in the
  batch, not per row. **Item 8** (sidecar bytes per node): spans are made only for the program's patterns and
  their matches.
- **Design index §4b** (text-candidate hierarchy ending in span confirm): §3 is the candidate step, the host's
  regex the confirm.
- **History index §2.5(a), S165:** Rust regex's lazy DFA is linear even on pathological patterns. That is why the
  host confirm stays the Rust regex, and why §3 is a prefilter and not a replacement.
- **Parabix** (Cameron, Lin, Medforth et al., "Bitwise data parallelism in regular expression matching", PACT
  2014; icgrep). `MatchStar` is the add-with-carry that `fill` already is.

## Open

- **The device launch.** The block program runs on the CPU interpreter in the batch. Metal's executors give the
  same words (tested), but the batch does not launch them: the binding (`Binding::Metal`) does not reach the
  batch's build, and the programs of a batch would go in one `metal_block_program::run_batch`.
- **Node, comment and capture spans** are matched on the host, once per distinct text (§2). On the device they
  would be the vocabulary's strings, matched by the same programs.
- **Repetition of a group** (`(ab)*`) is widened to every later position; exact would need a loop, which block
  programs refuse. Bounded repetition is unrolled up to 16.
- **Unicode `\b`** and CRLF line anchors are not checked (wider).
- **Nested nodes of one kind** (an f-string inside an f-string) are each matched, though their texts overlap.
  Memoised per distinct text; kinds that nest deeply are not bounded.
