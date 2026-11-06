# Parser generators: machines as bit operations, and Python without automata

Status: design with a prototype measurement, 2026-11-06. It follows `03-bitlex.md` §3.2 and
`05-many-states.md`.

## Intro

The Python lexer has two `machine`s: `strings` (13 states) and `fstrings` (25 states). This
document asks whether they can become the same kind of operation as the rest of the lexer:
masks, shifts, `fill`, `toggle` and prefix sums, unrolled over 64-byte words.

There are two routes, and they meet:

1. **Lower any machine to bit operations.** Krohn–Rhodes theory splits every finite automaton
   into two kinds of part. A *reset* part ("set by one input, cleared by another") is a `fill`.
   A *permutation* part is a prefix sum in a group, and `toggle` is the case of the group of
   order 2.
2. **Rewrite Python's lexing so no machine is needed.** Python's quotes turn out to act as
   reflections of a triangle. Two quote kinds and the code state form the group of symmetries of a
   triangle, so a toggle and a count mod 3 say which string a byte is in. Comments and short
   strings end at a line end, so everything except triple-quoted strings is local to one line.

A prototype of route 2 agrees byte for byte with the `strings` machine on every file of both
corpora where it settles in one pass (§4).

## 1. Is the current lowering branch-free?

Yes. `03-bitlex.md` §3.2 lowers a machine as a scan of transition maps:

- each event's map is a 16-byte vector;
- composing two maps is one `TBL`;
- the state at an event is its prefix map applied to the start state.

No control flow depends on the data. The data decides only table indices, as a class lookup does.

What it is not is a *bit* operation:

- it works on a compacted list of events, not on 64-bit masks;
- its carry between blocks is a map, not a bit;
- its cost is per event, with a gather and a scatter around it.

The routes below remove those differences.

## 2. Route 1: decompose the machine

### 2.1 The parts

A machine's inputs act on its states as functions. The functions an input sequence can produce
form the machine's *transformation monoid*. Krohn and Rhodes (1965) showed that every finite
machine is a cascade of two kinds of simple machine. Each level reads the input and the states of
the levels before it.

| part | what it does | Bitlex operation | carry |
|---|---|---|---|
| reset (flip-flop) | set on `s`, cleared on `e`, unchanged otherwise | `fill(s, e)` | 1 bit (borrow) |
| group Z2 | flip on `a` | `toggle(a)` | 1 bit (parity) |
| group Zn | add on `a`, mod n | a prefix sum mod n | ⌈log2 n⌉ bits |
| solvable group, such as S3 | a cascade of the cyclic groups above | toggles and prefix sums, the later ones signed by the earlier | a few bits |

- **Cascades are straight-line code.** A part's inputs are masks built from the input and from
  earlier parts' masks. Each part is one `fill` or one scan, so the whole machine is a fixed
  sequence of word operations, unrolled per block.
- **Non-solvable groups do not fit.** The smallest is A5, the rotations of an icosahedron. They
  need the table scan, and no lexer seen so far has one.
- **A machine without groups** (*aperiodic*, or *star-free*: Schützenberger, 1965) is a cascade
  of `fill`s alone.

A generator can compute the monoid of a small machine and emit the cascade, falling back to the
table scan when it finds a non-solvable group or the cascade grows too deep.

### 2.2 The groups in Python's quotes

Take the three states code, in `'`, and in `"`, and number them 0, 1, 2.

| input | on code | on `'` | on `"` | as a function of `x` (mod 3) |
|---|---|---|---|---|
| `'` | → in `'` | → code | unchanged | `x ↦ 1 − x` |
| `"` | → in `"` | unchanged | → code | `x ↦ 2 − x` |

Each quote is a *reflection* `x ↦ b − x`, with `b = 1` for `'` and `b = 2` for `"`. The other
kind's string is the reflection's fixed point, so a `"` inside `'…'` changes nothing, as Python
requires. Reflections generate S3, the symmetries of a triangle. S3 is solvable: Z3 with a Z2 on
top.

After `n` reflections from code:

```text
x_n = b_n − b_{n−1} + b_{n−2} − …   = (−1)^n · Σ_j (−1)^j b_j      (mod 3)
```

So, as masks over one line:

```text
p     = toggle(quotes)                       # the parity of each quote event: Z2
c_j   = ±1: +1 when (quote is ') xor p_j      # the reflection's offset, signed by the parity
A     = prefix sum of c, mod 3               # Z3
in a short string  ⇔  A ≠ 0                  # the sign (−1)^n does not change zero
which kind         = the opening quote's byte
```

- **Prefix sums mod 3** run as a Hillis–Steele scan over two bit planes: six rounds per 64-bit
  word, each a shifted add mod 3 of about eight logic operations. When events are sparse, they can
  also run over the compacted events.
- **The carry** between blocks is 1 parity bit plus a value mod 3: 3 bits. It is a group element,
  so it is invertible, and there are never several candidate runs to keep.

The prototype checks this closed form against applying the reflections one by one, on every
event of both corpora.

### 2.3 Resets: comments

`#` is not a permutation: it sends code to comment and leaves the strings alone. It is a reset,
cleared by the line end. The first `#` at which `A = 0` starts the comment:

```text
cmt = fill(hash & (A == 0), nl)              # from the first code '#' to the newline
```

Quotes after it on the same line feed the scan as well, so `A` is wrong there, but `cmt` covers
those bytes. The scan restarts at the next line, so the error stops there.

## 3. Route 2: Python's lexing without a machine

### 3.1 Short strings and comments are line-local

A short string cannot contain an unescaped newline: tree-sitter makes it an error, and so does
the `strings` machine. A comment ends at the line end. So at every unescaped newline outside a
triple-quoted string, the state is code. The reflection scan of §2.2 and the comment fill of §2.3
are *segmented*: they restart at each such newline.

A backslash-newline inside a short string is escaped. It is not a segment boundary, and the
string continues, as Python requires.

### 3.2 Triple-quoted strings

Only triple-quoted strings span lines. Taken alone, they follow the same geometry:

- a `'''` inside a `"""` string is a fixed point;
- each triple of the other kind is a reflection.

So the triple-quoted regions are the same toggle and count mod 3, over the triple-quote events of
the whole file.

Triple-quote events are the quote runs of three, aligned from the start of each run: `''''` is
`'''` then `'`. This matches the `strings` machine and Python's tokenizer when the run starts in
code or inside a triple-quoted string.

### 3.3 Where the two meet: hidden triple quotes

A `'''` inside a short string or a comment is text, not a delimiter:

```python
x = "'''"          # a short string holding three quotes
# use ''' for docstrings
```

The triple scan (§3.2) does not know that, and the line scan (§3.1) needs the triple scan to know
which lines are code. Two ways out:

**Speculate and verify.**

1. Assume every triple-quote event is live. Run the triple scan, then the line scans.
2. Check that no live opener lies in a short string or a comment, and that no hidden one lies
   outside them.
3. If the check fails, give every triple-quote event the status this pass implies, and rerun.

A consistent pass is the right answer, and the only one. Whether a triple-quote event is a
delimiter depends only on the bytes before it, so by induction from the left there is exactly one
self-consistent assignment. Each pass is straight-line mask code. A file that has not settled
after a fixed number of passes falls back.

**Per-line hypotheses** remove the passes. A line that holds a triple-quote event can start in
one of three states: code, inside `'''`, or inside `"""`. Its bytes up to the first triple of that
kind are local work under each hypothesis. The result is the line's action on the three states,
with an error for impossible starts.

- **In the common case the action is a reflection** (or the identity). Composing lines is then
  the same toggle and count mod 3, over lines instead of quotes.
- **A line with a hidden triple** may act otherwise: `x = "'''"` keeps code as code, but makes
  "inside `'''`" an error.
- **Composing actions** on three states is a 3-entry map scan, one `TBL` per line with a triple
  quote. That is the only place a map is left, and it touches a small fraction of lines.

## 4. Measured: the prototype

`prototypes/nofsa.rs` (plain Rust, no dependencies; run with a file listing paths) implements route 2 with speculate-and-verify. It classifies
every byte as code, string or comment, and compares the result with the 13-state `strings`
machine of `lexers/python.bitlex`, simulated byte by byte. All files in each list are included,
including those tree-sitter rejects.

| | data-layer Python | CPython 3.15 |
|---|---:|---:|
| files | 13,088 | 2,054 |
| both reject (an unterminated string) | 1 | 2 |
| **settled in 1 pass** | 13,016 (99.46%) | 2,009 (97.8%) |
| settled in 2–3 passes | 39 | 21 |
| settled in 4–8 passes | 22 | 9 |
| not settled in 8 passes | 10 | 13 |
| **bytes classified differently from the machine, in settled files** | **0** | **0** |
| the closed form of §2.2 against direct reflection | equal on every event | equal on every event |

- **Exact where it settles.** No settled file differs from the machine in any byte.
- **The files that need passes quote code.** Examples are codemods, linters and tests that hold
  `'''` or `"""` inside strings.
- **Passes cost a full lexing pass each,** because one misread triple quote shifts every triple
  after it. That is why per-line hypotheses (§3.3) are the better design for the 0.5–2% of files
  with hidden triple quotes. On the 98–99.5% that settle in one pass, speculate-and-verify costs a
  single pass plus the check.

## 5. F-strings without a machine

Not prototyped. The same two ideas apply.

- **Depth is a prefix sum.** Inside an f-string body, `{ ( [` add 1 and `} ) ]` subtract 1.
  - Literal text is depth 0, and an interpolation is depth ≥ 1. A `:` at depth 1 starts the
    format spec, which runs to the brace that returns to depth 0.
  - There is no depth bound and no states per depth. Today's machine has 5 states per level, for
    depths 0–4.
- **Speculate and verify the two exceptions:**
  - `{{` and `}}` are escapes in literal text but two braces in code (`f"{ {1:2}}"`).
  - A quote inside an interpolation starts a string whose braces do not count.

  Assume `{{` and `}}` are escapes and that interpolation quotes pair by the reflection scan.
  Compute depth, then check that every escape pair lies at depth 0 and every counted quote at
  depth ≥ 1. Repeat on failure. Both exceptions are rare, so one pass should settle nearly every
  file. That is to be measured.
- **Nested f-strings** are the same computation over the inner string's body, one level per pass.
  This is the cascade `f1`, `f2` already uses.
- **PEP 701 quote reuse** (`f"{x["a"]}"`) is the case where an interpolation's string uses the
  outer quote. With depth as a prefix sum, a quote at depth ≥ 1 belongs to the interpolation
  whatever its kind. So the triple and short-string scans would have to skip f-string
  interpolations, which needs the depth first: one more speculate-and-verify round.

## 6. What changes in the lexer

| machine today | replaced by | carry between blocks |
|---|---|---|
| `strings`, 13 states | segmented reflection scan for short strings, comment `fill`, reflection scan for triple quotes, and a check | parity + mod 3 (lines); parity + mod 3 (triples); 1 bit (comment) |
| `fstrings`, 25 states, two instances | depth prefix sum, spec `fill`, reflection scan for inner strings, and a check | depth (an integer), 1 bit (spec), parity + mod 3 |

- **No event compaction.** Every step is a word operation over 64-byte blocks, as in simdjson.
- **Carries are group elements** (parities, residues and counts) and borrow bits, so a block's
  carry-in is applied with XORs, adds and selects.
- **The check** is one more mask pass, and its failure triggers either a rerun or the line
  hypotheses.

Recommended order:

1. Build the short-string and triple-quote scans with speculate-and-verify. That replaces
   `strings` for 99.5% of data-layer files.
2. Add per-line hypotheses for the rest.
3. Measure the f-string version (§5) before replacing `fstrings`.
