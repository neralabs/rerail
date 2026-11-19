# Parser generators: automata with many states

Status: design, 2026-11-06. It follows `03-bitlex.md` §3.3, the data-parallel automaton. The
Python lexer no longer uses the machines this page takes as examples: `08-without-automata.md`
replaces them with bit operations, and Python 3.12 quote reuse (§5 here) is handled by rounds.
The methods remain for grammars whose machines do not decompose.

## Intro

`03-bitlex.md` runs each `machine` as a data-parallel automaton (Mytkowicz, Musuvathi and Schulte,
ASPLOS 2014). Instead of the current state, it carries a *map*: for every state the machine could
have started a stretch of input in, the state it ends in.

- **Applying an event:** each event has a table `T_e` of `N` entries. The new map is
  `T_e[map[s]]` for every `s`, which is one table lookup per lane.
- **Joining stretches:** two maps compose the same way. So a long input splits into pieces that
  are processed independently and then joined by a scan.
- **At 16 states or fewer** the map is one 16-byte vector, and applying an event is a single NEON
  `TBL` instruction. The two Python machines were sized to fit that (13 states, and first 14).

This document asks what happens past 16, past 64 and past 256 states. It starts from what a map
costs, then gives five ways to keep the cost down. A measurement on real files (§3) shows that the
cost depends far more on how many runs stay distinct than on how many states exist.

## 1. What a map costs

The map has `N` byte entries, one per start state. Applying an event's table needs, per 16 entries
of the map, one lookup into a table of `N` entries.

| states | map | NEON per event | Metal per event |
|---:|---|---|---|
| ≤ 16 | 1 register | 1 `TBL` (1-register table) | 1 lane read per thread, 16 threads |
| ≤ 32 | 2 | 2 `TBL` (2-register table) | 1 per thread, 32 threads: one simdgroup |
| ≤ 64 | 4 | 4 `TBL` (4-register table) | 2 per thread |
| ≤ 128 | 8 | 8 × (`TBL` + `TBX`), about 24 instructions | 4 per thread |
| ≤ 256 | 16 | 16 × (`TBL` + 3 `TBX`), about 100 instructions; the table no longer fits in registers | 8 per thread, table in threadgroup memory |

- **NEON's `TBL` reads up to 4 registers, 64 bytes.** `TBX` is the same lookup, except that lanes
  whose index is out of range keep their old value. A 256-entry table is four 64-byte quarters:
  one `TBL` on the index, then a `TBX` on the index minus 64, 128 and 192.
- **On Metal** each thread holds some entries of the map, and each lookup is a read from
  threadgroup or constant memory at the entry's value. A simdgroup of 32 threads holds a 32-state
  map one entry per thread.

The cost is quadratic in `N`: `N` entries, each a lookup into `N`. But it is paid per *event*, not
per byte. The Python `strings` machine sees one event per 53 bytes. At that rate:

- 256 states would cost about 2 instructions per byte;
- 16 states cost 0.02.

So a large machine over sparse events is affordable. A large machine over every byte is not.

## 2. Five ways to keep it down

### 2.1 Take counters and stacks out of the state

Most large lexer automata are large because they count. Bracket depth inside an interpolation,
nesting of f-strings, and nesting of block comments in languages that allow it are counters or
stacks. Each level multiplies the states:

- the `fstrings` machine has 25 states because it holds bracket depth 0–4 in its state;
- at depth 0–2 it had 14.

Counting is what prefix sums do well. `01-branch-free-design.md` §3 finds bracket depth and
matching brackets for the whole file with one scan. A grammar that is *visibly pushdown* (Alur and
Madhusudan, 2004) can always be split this way: its input symbols are known in advance to be push,
pop or neither. What is left is a small automaton per nesting level, plus a scan.

The catch is that the automaton sometimes decides what counts:

- a `{` in f-string literal text opens an interpolation;
- a `{` in an interpolation is a dict;
- `{{` in literal text is an escape.

When the automaton decides which bytes are brackets, the count cannot be taken first. Two ways
out:

- **Cascade (§2.3):** a lower layer marks the brackets, then the scan counts them.
- **Keep a small bounded counter in the state,** as `fstrings` does, with a fallback past the
  bound. The corpora set the bound: the deepest interpolation in either is 4.

### 2.2 Products: run independent parts separately

If a machine is the product of two machines that do not read each other's state, run them
separately. They cost `|A| + |B|` lanes instead of `|A| × |B|`.

Python's lexing is close to this. `strings` and `fstrings` together would be about 13 × 25 states
as one machine. Split, they are 13 and 25, and `fstrings` runs only over f-string bodies.

### 2.3 Cascades: let one machine read another's state

When B's events depend on A's state but A does not read B, run A, then B. This is the cascade of
the Krohn–Rhodes decomposition. The cost is |A| + |B| at run time.

In `lexers/python.bitlex` there are three layers:

- `strings` finds strings and comments;
- the f-string bodies it leaves become the input of `f1`;
- the nested f-string bodies `f1` leaves become the input of `f2`, the same machine again.

Each layer adds one round of joining across blocks (`06-block-parallel.md` §4). A layer's
summaries can only be joined once the layer below has been resolved. For a lexer with two or three
layers this is cheap: each round is a small scan over block summaries.

A cascade can also be joined in a single scan, when one round per layer costs too much. The
summary of a block is then a pair:

- A's map;
- for each start state of A, B's map.

Two summaries compose by

```text
(mA1, fB1[·]) ∘ (mA2, fB2[·]) = (mA2 ∘ mA1,  s ↦ fB2[mA1(s)] ∘ fB1[s])
```

That costs `|A| × |B|` lanes in the summary, but only at block boundaries. Within a block, A and
B still run one after the other.

### 2.4 Track only the runs that stay distinct

The map has `N` entries, but most of them hold the same few values. After a newline in code, every
start state that was not inside a string or a comment ends in the same state.

Mytkowicz et al. call this *convergence*. Their implementation keeps only the distinct values,
with an index from each start state to one of them. The cost is then `k` lanes, the number of
distinct runs, instead of `N`. §3 measures `k`.

Two lowerings:

- **Fixed `k`:** keep `k` candidate states, say 4, and a 1-byte index per start state.
  - Applying an event updates the `k` candidates, one lookup each into the `N`-entry table, which
    is a gather of `k`.
  - Then any two candidates that became equal are merged. A merge is a compare and a select, with
    no branch.
  - If more than `k` stay distinct, the block is redone with the full map, or the file falls back.
    That is one test per block, not per byte.
- **Speculate:** start each block from its most likely state, such as code, and walk it serially.
  When blocks are joined, check the guess. A wrong guess redoes that block.
  - Its cost depends on the data, which `03-bitlex.md` §3.4 rules out for the main path.
  - It is useful on the device, where a few redone blocks are cheaper than carrying wide maps.

### 2.5 Bit-parallel NFAs, for patterns

A `check` pattern, or a token regex, may have a large DFA but a small NFA. Hyperscan's LimEx
engine and the classic shift-and algorithm (Baeza-Yates and Gonnet, 1992) run an NFA of up to
64, 128 or 512 positions as a bitset:

```text
S' = ((S << 1) | start) & reach[byte]            # plus extra shifts for repetition and epsilon edges
```

- **Per byte:** one load, a shift, an OR and an AND, with no branch. Serial within a stretch.
- **Joining stretches:** the summary of a stretch is an `n × n` boolean matrix: which positions,
  set at the start, are set at the end. Composing two is a boolean matrix product, which costs
  `n²/64` word operations for 64-bit words.

So NFAs fit the block model too. They are worth it when the DFA explodes and the pattern has to
run across block edges. For a `check` over one token, the token rarely crosses a block, and the
NFA just runs serially per token in a gathered list.

### 2.6 Smaller alphabets

Tables are indexed by event or byte class, not by byte. This decides the table count, not the
state count. Even so, it is what keeps tables small enough to sit in registers. The minimal DFAs
of the number patterns have 14 (JSON), 33 (Python integer) and 19 (Python float) byte classes.

## 3. Measured: runs converge

The `strings` machine was run over the first 400 files of the CPython 3.15 standard library
(9.1 MB). For each block, the twelve states other than `error` were all used as starts. The table
counts how many distinct states other than `error` the twelve runs ended in. Newlines were
counted as events whether or not their line had a quote or `#`, which makes events denser than in
the real machine.

| block | blocks | 1 | 2 | 3 | 4 | 5–6 | 9–12 |
|---|---:|---:|---:|---:|---:|---:|---:|
| 64 B | 142,482 | 0.00% | 0.03% | 64.5% | 12.4% | 20.0% | 3.1% |
| 1 KB | 9,094 | 0.08% | 7.0% | 92.8% | | 0.11% | 0.02% |
| 16 KB | 785 | 1.4% | 40.9% | 57.7% | | | |

- **Three runs remain almost everywhere:** in code, inside a `'''` string, and inside a `"""`
  string.
  - A run that started inside a triple-quoted string stays there until a matching triple quote
    arrives. Most blocks have none.
  - Every other start converges within a line or two.
- **Larger blocks converge further.** Over 16 KB, 41% of blocks are down to two runs.
- **64-byte blocks often keep 4–6 runs.** A block that ends in the middle of a line has not yet
  seen the newline that merges the comment and short-string runs.

So a map with room for 4 candidates covers 99.9% of 1 KB blocks in this sample. It does not
depend on how many states the machine has. A machine of 64 or 256 states, built like this one,
would still carry about 3 runs. The `N`-entry table is read through a gather of 3–4 entries, not
composed lane by lane.

## 4. Choosing, per machine

The generator can choose a lowering per machine from its state count, and from convergence
measured on the corpus:

| states | default lowering |
|---|---|
| ≤ 16 | full map, 1 `TBL` per event |
| 17–64 | full map, 2–4 `TBL`, if events are sparse; otherwise `k` candidates |
| 65–256 | `k` candidates, with a full-map redo when they overflow |
| > 256 | split it first: counters into scans (§2.1), then products and cascades (§2.2, §2.3) |
| patterns with a DFA too large to build | bit-parallel NFA (§2.5) |

A machine that will not fit any of these is a sign that the lexer is doing the parser's work.
Python's layout (INDENT and DEDENT) is an example: it is a stack of indentation widths, not a
lexer state. `02-python.md` §4 computes it with nearest-smaller-value scans instead.

## 5. Python 3.12 f-strings, as a test case

PEP 701 lets an interpolation reuse the f-string's own quote: `f"{x["a"]}"`. The `strings` machine
ends the f-string at the inner `"`, so `f1` sees `end` in an inner-string state and the file falls
back. That is 5 of 1,993 CPython files today, and the share will grow as Python 3.12 spreads.

Handling it means `strings` must know when it is inside an interpolation, so the two machines
merge. Built with the methods above:

- **Without nesting:** the merged machine holds a frame for the current f-string: its quote kind,
  its bracket depth, and literal text or code.
  - That is the 12 states of `strings`, plus the 4 quote kinds times about 10 `fstrings` states,
    about 52 in all. It fits a 64-entry map.
  - It runs over all events of the file, at one per 53 bytes, so 4 `TBL` per event cost about
    0.08 instructions per byte.
- **With nesting:** f-strings may nest without limit, since each level may reuse the quote. Each
  level multiplies the states, so nesting goes to a cascade (§2.3), one instance per level, as
  `f1` and `f2` are today.
  - Each level's input is the interpolation code of the level above.
  - The number of levels is bounded by a fallback. The corpora have at most 2.
- **With convergence:** of the 52 states, the live runs at a block boundary are, as in §3, code
  and the open long strings. So the `k`-candidate form costs about as much as today's
  `strings`.
