# Parser generators: machines as bit operations, and Python without automata

Status: design with prototype measurements, 2026-11-06. It follows `03-bitlex.md` §3.2 and
`05-many-states.md`. Done since: the constructs of §8 are in Bitlex (`03-bitlex.md` §2.7),
`lexers/python.bitlex` uses them and has no machine, and its tokens equal tree-sitter's
(`03-bitlex.md` §5). §9 says what changed on the way.

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

Two prototypes measure route 2 on the corpora (§4, §5):

- **Strings and comments** agree byte for byte with the `strings` machine wherever they settle.
  With per-line hypotheses, 99% of lines settle in one round and all but 5 within five.
- **F-string interpolations and format specs** equal tree-sitter's on all 42,956 data-layer
  f-strings. On CPython, all but 20 Python 3.12-specific ones match. Four rounds settle 99.9%.

§6 shows how a `"""` split across a 64-byte boundary is handled with a few carried bits. §7 turns
the whole design into fixed work with no data-dependent branch.

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

Speculate-and-verify over the whole file converges slowly when a file holds many hidden triple
quotes. One misread triple shifts the pairing of every triple after it, so each pass fixes about
one (§4.1). Per-line hypotheses confine the damage to one line.

**Per-line hypotheses.** A line can start in one of three states: code, inside `'''`, or inside
`"""`. Under each, the line's bytes are classified locally:

- **Inside a triple string:** everything up to the first triple of that kind is string. From
  there on the line is in code.
- **In code:** the short-string scan, the comment fill and any triple strings that open and close
  on the line.
- **Inside the line,** speculate-and-verify settles the triple quotes. A wrong guess now corrupts
  only the rest of its own line, so it settles in a round or two (§4.2).

The result is the line's *action*: the state it ends in for each of the three it may start in, or
an error for an impossible start.

- **Most lines are the identity.** Only 2.7% of lines hold a triple-quote candidate.
- **Most of the others are reflections,** such as a docstring's first line, which swaps code and
  inside `"""`.
- **About 100 lines per corpus are not one-to-one.** `"""RSA PKCS#1 v1.5"""`, read as starting
  inside `"""`, closes at the first `"""` and then turns the rest of the line into a comment. So it
  ends in code whichever of those two states it starts in.

So lines compose as maps on three states, not as group elements. A map on three states is three
2-bit fields. Composing two is three selects, and it is needed only for lines that hold a
triple-quote candidate. The line's real start state then selects which hypothesis's masks hold.

## 4. Measured: the prototypes

`prototypes/nofsa.rs` is plain Rust with no dependencies, run with a file that lists paths. It
compares against the 13-state `strings` machine of `lexers/python.bitlex`, simulated byte by
byte.

### 4.1 Whole-file speculate-and-verify

Every file in each list is included, among them those tree-sitter rejects.

| | data-layer Python | CPython 3.15 |
|---|---:|---:|
| files | 13,088 | 2,054 |
| both reject (an unterminated string) | 1 | 2 |
| settled in 1 pass | 13,016 (99.46%) | 2,009 (97.8%) |
| settled in 2–8 passes | 61 | 30 |
| not settled in 8 passes | 10 | 13 |
| **bytes classified differently from the machine, in settled files** | **0** | **0** |
| the closed form of §2.2 against direct reflection | equal on every event | equal on every event |

The files that need passes quote code: codemods, linters and tests.

### 4.2 Per-line hypotheses

Each line that holds a triple-quote candidate was classified under all three start states. The
result was compared with the machine run over the same line from the same state.

| | data-layer Python | CPython 3.15 |
|---|---:|---:|
| lines | 4,207,502 | 1,065,750 |
| lines with a triple-quote candidate | 116,441 (2.77%) | 28,409 (2.67%) |
| their action is not the identity | 47,353 | 22,615 |
| their action is not one-to-one | 95 | 102 |
| (line, start state) pairs that agree with the machine | 349,321 | 85,223 |
| pairs that differ | 1 | 0 |
| pairs not settled in 6 rounds | 1 | 4 |
| rounds | 1: 99.3%; at most 5 | 1: 99.0%; at most 5 |

The pairs that differ or do not settle are runs of quotes whose grouping into triples depends on
the state before them, such as `''''one''''` inside a short string, or `'''` → `''''''` in a
comment. Each sends its file to fallback: 2 files in the data layer, and 4 lines in CPython
(including `test_fstring.py`).

## 5. F-strings without a machine

`prototypes/fstr.rs` builds interpolations and format specs this way and compares them with
tree-sitter's `interpolation`, `format_expression` and `format_specifier` spans. It does this for
every f-string in files that tree-sitter parses without error.

### 5.1 The method

The *context* before each byte of an f-string body is four values:

- the depth, an integer;
- whether the byte is in an inner string, and which kind;
- whether it is in a comment;
- whether it is in format-spec text.

Each round computes every byte's *role* from the previous round's context, then the new context
from the roles:

| bytes | role | decided by |
|---|---|---|
| a run of `{` at depth 0 | pairs are escapes; an odd last one opens an interpolation (+1) | the context at the run's first byte only |
| a run of `{` at depth ≥ 1 | each opens (+1): a dict or set, or a nested field in a spec | as above |
| a run of `}` starting at depth `d` | the first `min(d, length)` close (−1), the rest are `}}` escapes; an odd remainder is an error | as above |
| `( [` and `) ]` at depth ≥ 1, not in spec text | +1 and −1 | the previous context |
| `'` `"` at depth ≥ 1, not in spec text | inner strings, by the reflection scan of §2.2, restarted at each interpolation | a scan within the round |
| `#` at depth ≥ 1, not in an inner string | a comment, filled to the newline | a fill within the round |
| `:` at depth 1, not in an inner string | the format spec starts, filled until depth returns to 0 | the previous context |
| `\N{` in literal text of a non-raw f-string | an escape through its `}` | local |

The new depth is a prefix sum of the roles. When a round leaves the context unchanged, it is the
answer, and the only possible one:

- every role depends only on the context strictly before its byte (a brace run, on its first
  byte);
- so by induction from the left there is exactly one self-consistent context.

**Two things make it converge fast.**

1. **Inner strings and comments are scans within the round,** not facts carried from round to
   round. A first version carried them, and paired one quote per round. It needed up to 29 rounds.
2. **A first guess that ignores depth.** For one round, every `{{` and `}}` counts as an escape
   and an odd remainder counts as ±1. That gets most depths right at once, and the exact rounds
   only correct and confirm.

A rule that depended on bytes inside its own run was a bug the measurement caught. An early
version decided each `{` of a run from its own context, so a wrong grouping could be consistent
with itself (`{prefix}/{{name}}` grew a phantom interpolation). Deciding a run from its first byte
restores the uniqueness argument.

### 5.2 Measured

| | data-layer Python | CPython 3.15 |
|---|---:|---:|
| f-strings | 42,956 | 5,998 |
| f-string body bytes | 2.04 MB, 1.22% of all bytes | 0.29 MB, 0.78% |
| **interpolations and specs equal to tree-sitter's** | **42,956 (all)** | 5,978 |
| different | 0 | 7: a format spec nested inside a nested field's spec, all in `test_fstring.py` |
| not supported | 0 | 13: a triple-quoted string inside an interpolation (11), a newline in a one-line f-string's interpolation (2); both are Python 3.12 forms |
| rounds, including the confirming one | 1 (no interpolation): 575; 2: 31,209; 3: 10,544; 4: 621; 5–7: 7 | 1: 576; 2: 4,374; 3: 969; 4: 60; 5–6: 6 |

So four rounds settle 99.98% of data-layer f-strings and 99.9% of CPython's. Seven rounds settle
all of them.

### 5.3 What is left

- **Nested format specs** (`{x:{w:0}}`) need the spec rule at every depth, not only at depth 1.
- **PEP 701 quote reuse** needs the triple-quote and short-string scans to skip interpolation
  code. That is one more round across the two layers.
- **Nested f-strings** are an inner string with an `f` prefix. Their bodies get the same rounds,
  one level later.

## 6. A `"""` split by a 64-byte boundary

Take a `"""` whose first quote is byte 63 of block `k`, and whose other two are bytes 0 and 1 of
block `k + 1`. Every fact it needs is either read across the boundary or carried in a few bits.

| fact | how it crosses | carry or halo |
|---|---|---|
| "this quote is not escaped" | `after_odd_run(backslash)`; a run of backslashes may end block `k` | 1 bit: the run's parity |
| "three quotes start here": `dq & next(dq, 1) & next(dq, 2)` | `next` reads bytes 0 and 1 of block `k + 1`. The input is in memory, so this is an unaligned load, not a carry | a 2-byte halo |
| "this run of quotes is grouped from its start" (`''''` is `'''` then `'`) | below | 3 bits |
| "bytes 0 and 1 of block `k + 1` belong to the triple at 63" | `span(triple, 3)` looks 2 bytes back | the last 2 bits of the triple mask |
| the triple scan's state | the reflection scan | 1 parity bit + a residue mod 3 |
| the line's state under each hypothesis | short-string scan, comment fill, per hypothesis | 3 × (parity + mod 3 + 1 bit) |

**Run grouping without knowing where the run started.** A run of quotes may begin many blocks
earlier, and triples are counted from its first byte. Let `P0`, `P1`, `P2` be the masks of bytes
whose file position is 0, 1 or 2 mod 3. Since 64 ≡ 1 (mod 3), they rotate by one position from
each block to the next. A block picks its three constants by its index mod 3, which is a select,
not a branch. Then:

```text
run_start = q & ~prev(q_same)                         # q_same: same quote character, unescaped
F_r       = fill(run_start & P_r, ~q_same)            # bytes of runs that started at phase r
aligned   = (F_0 & P_0) | (F_1 & P_1) | (F_2 & P_2)   # 0, 3, 6, … bytes from the run's start
triple    = aligned & next(q_same, 1) & next(q_same, 2)
```

The three `fill`s carry one borrow bit each. The grouping is then right however long the run, and
wherever it started.

**On the device,** the same facts cross a threadgroup boundary as part of the summary in
`06-block-parallel.md`. The triple scan's carry is a group element, so the carry-in is applied by
the closed form of §2.2: a select on the parity and an add mod 3.

## 7. No data-dependent branches: spend work instead

A mispredicted branch costs 13–20 cycles, in which a core could have issued 50–100 SIMD instructions. So the
design avoids every branch whose direction depends on the file, and pays for that with work that
is the same for every file:

- **All three line hypotheses are always computed,** for every line. Lines without a triple
  candidate give the identity, and their hypothesis lanes cost the same mask work as the main lane.
  That triples the short-string and comment work, which is a few tens of operations per 64 bytes.
- **Rounds are unrolled to a fixed count, never looped until done.**
  - Inside lines: 5 rounds over the gathered lines that hold a triple candidate, 2.7% of lines.
    Round 1 settles 99%.
  - In f-strings: 1 first-guess round and 4 exact rounds over the gathered f-string bodies, 1.2%
    of bytes. That settles 99.98%. Seven rounds settle all of the data-layer and CPython f-strings.

  A body that is still changing after the last round sets the file's error bit. That is one OR per
  file, read once, like every other fallback condition.
- **Gathers replace branches.** Lines and bodies that need rounds are compacted with prefix counts
  (simdjson's flattening, or a scan on Metal). The rounds then run over dense lists, so their cost
  is in proportion to the lines and bodies that need them.
- **What remains data-dependent:**
  - Loop trip counts, which are sizes: blocks, gathered lines, gathered bodies.
  - The per-file fallback decision.

  A branch on a file's fallback bit is taken about once in 300 files, so it is predicted. No branch
  is taken per byte, per event or per token.

## 8. What changes in the lexer

| machine today | replaced by | carry between blocks |
|---|---|---|
| `strings`, 13 states | short-string reflection scan and comment fill per line, under 3 hypotheses; triple-quote reflection scan; line actions composed as 3-state maps; 5 unrolled rounds inside lines | per hypothesis: parity, mod 3, 1 bit; triples: parity, mod 3; run grouping: 3 bits |
| `fstrings`, 25 states, two instances | depth prefix sum; inner strings by reflection scan; comment and spec fills; 1 + 4 unrolled rounds over gathered f-string bodies | depth (an integer), parity, mod 3, 2 bits |

Recommended order:

1. Write these as Bitlex constructs:
   - `reflect(events, b)` for the mod-3 scan;
   - `hypotheses` for per-line lanes;
   - `rounds N { … }` for unrolled fixed points, with an error on non-convergence.
2. Port `prototypes/nofsa.rs` and `prototypes/fstr.rs` into the reference evaluator. They are the
   oracles for the mask versions.
3. Cover nested specs and PEP 701 (§5.3). After that, no file in either corpus needs either
   machine, other than the few already sent to fallback.

## 9. Built: what changed from the design

All three steps of §8 are done. The prototypes are ported into `tools/bitlex` as oracles, the
lexer is written with the new constructs, and the evaluator compares both with tree-sitter.

- **Nested format specs** need no level per depth. A second count for the innermost field
  referred to its own closes and gained one field per round. Two questions about frames replace
  it: `depth.frame(fopen)` (is the frame around this byte a replacement field?) and
  `depth.since(colon)` (has that field passed its `:`?). Both are matching by depth.
- **Python 3.12 quote reuse** is an outer round. The interpolation code found in one round is
  hidden from the string scan in the next. Each round moves the end of such an f-string past one
  more quote. One CPython file needs 8 rounds; files without such an f-string need 1, or 2 when
  they hold an f-string. The second confirms that no interpolation ended open.
- **A newline inside interpolation code** no longer ends a line, which covers the two
  "unsupported" Python 3.12 cases of §5.2.
- **Bracket depth has no bound.** `head(rb, depth)` closes as many levels as the depth before the
  run.
- **A comment ends at any newline,** even one after a backslash. Its line then goes on in code,
  found with one more carried mask in the line rounds.

| | data-layer Python | CPython 3.15 |
|---|---:|---:|
| files tree-sitter parses | 13,042 | 1,993 |
| tokens equal to tree-sitter's | 13,042 | 1,990 |
| f-string fields equal to tree-sitter's (files) | 13,042 | 1,990 |
| falls back | 0 | 3 |

The three CPython fallbacks are a triple-quoted string inside an interpolation, `1or 0`, and the
run of quotes from §4.2 that does not settle. Rounds taken are in `03-bitlex.md` §5.
