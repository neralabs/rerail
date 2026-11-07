# Bitlex: a lexer language that lowers to branch-free SIMD

Status: language with a reference evaluator, 2026-11-06. The lexers in `lexers/` are written in
it, and `tools/bitlex` runs them. Their tokens equal tree-sitter's on every JSON file and every
data-layer Python file that tree-sitter parses (§5). No compiler to NEON or Metal exists yet.

## Intro

A lexer decides, for every byte, what it belongs to: a string, a comment, a number, an operator.
The usual way is a loop that looks at one byte and branches on it. simdjson shows the other way:
compute a bit per byte for each fact, and combine those bits with logic, shifts, and prefix
operations that work on 64 bytes at once.

Bitlex is a small language for writing lexers that way. Every construct in it has a known
branch-free lowering: to NEON on the CPU, and to SIMD-group and threadgroup operations on Metal.
A lexer written in it says what each byte is, never how to loop over the bytes. The compiler
chooses the loops, and they contain no branches that depend on the data.

The page answers a question first, about automata and branching, because the answer shapes the
language. It then gives the language, its lowering and its guarantees. `lexers/json.bitlex` and
`lexers/python.bitlex` are the two worked examples.

## 1. Automata, branches, and what the Python lexer really needs

### 1.1 Is an automaton branch-free?

Not as it is usually run. There are three ways to run a deterministic automaton:

1. **A switch per byte.** Code that branches on the state and on the byte. Every step is a
   data-dependent branch, and Python's quotes and comments make those branches unpredictable.
   This is what Bitlex rules out.
2. **A table walk.** `state = T[symbol][state]`. There is no conditional jump, but each step needs
   the previous state, so the steps form a serial chain of dependent loads. It is branch-free but
   not data-parallel, so "branch-free" alone overstates it.
3. **A data-parallel automaton** (Mytkowicz, Musuvathi and Schulte, "Data-Parallel Finite-State
   Machines", ASPLOS 2014).
   - Instead of one state, carry the whole map: "if this stretch started in state *s*, it ends in
     state *f(s)*", for every *s* at once.
   - With at most 16 states, the map is one 16-byte vector. Applying one more symbol is one table
     shuffle (`vqtbl1q_u8` on NEON): `f' = T[symbol] ∘ f`.
   - Maps of separate stretches compose the same way, so a file can be cut into chunks that are
     processed independently and combined by a prefix scan. That is the GPU form.
   - There are no branches and no serial dependency between chunks.

Bitlex's `machine` construct means the third way, and only it.

### 1.2 What the lexers need

A first version of the Python lexer had two machines: `strings` (13 states, for strings and
comments) and `fstrings` (25 states, for interpolations). Both are gone. `08-without-automata.md`
shows how each is replaced by bit operations:

- **Quotes are reflections.** On the three states code, in `'` and in `"`, a `'` sends `x` to
  `1 − x mod 3` and a `"` sends `x` to `2 − x`. So which string a byte is in is a toggle and a
  count mod 3: `reflect` (§2.7).
- **Everything but triple-quoted strings ends with its line.** Each line is lexed three times, as
  if it started in code, inside `'''` or inside `"""`. The line ends pick one: `hypotheses`.
- **A few roles depend on bytes after them in the same line:** a `'''` inside a short string, and
  a brace whose role depends on the depth. These are fixed points, unrolled a fixed number of
  times: `rounds`.
- **Depth is a count,** a prefix sum, and the field a byte is in comes from matching by depth.

The `machine` construct stays in the language, and the JSON lexer's comments still use one
(7 states). Where plain bit operations are enough, no machine is used: JSON's escapes come from
simdjson's carry trick (`after_odd_run`), and a JSON file without a `/` outside strings needs only
one prefix XOR (`toggle`).

## 2. The language

A Bitlex file defines one lexer. Every value is a **mask**: one bit per byte of the input. Masks
are defined once, never assigned, and may refer only to masks defined above them, so a lexer is a
directed acyclic graph of mask operations.

### 2.1 Statements

| Statement | Meaning |
|---|---|
| `lexer NAME` | Starts the file. |
| `class NAME = ITEMS` | A set of bytes, such as `'"'`, `'\n'`, `'a'..'z'` or `0x80..0xff`. Its mask has a bit at each byte in the set. |
| `mask NAME = EXPR` | A named mask. |
| `machine NAME { … }` | A data-parallel automaton over chosen event bytes (§2.3). |
| `scan NAME = reflect(A -> S, B -> T) [reset R] [start lane]` | The reflection scan: states `out`, `S`, `T` (§2.7). |
| `scan NAME = count(UP, DOWN) [reset R] max D` | A count that never goes below 0 and is bounded by `D` (§2.7). |
| `hypotheses NAME split E lanes L … { … }` | Statements evaluated once per lane on every segment; the segment ends pick the lane (§2.7). |
| `rounds N [until settled] { carry X = E … next X = E … }` | A fixed point, unrolled at most `N` times (§2.7). |
| `block NAME(P, …) { … }`, `block NAME = TEMPLATE(E, …)` | A family of statements with mask parameters, and an instance of it (§2.5). |
| `machine NAME(P, …) { … }`, `machine NAME = TEMPLATE(EXPR, …)` | A machine with mask parameters, and an instance of it (§2.5). |
| `ucode over EXPR { class NAME = … }` | Classes of Unicode characters, decoded only where `EXPR` is set (§2.6). |
| `token KIND = start EXPR [through EXPR]` | Tokens of one kind (§2.4). |
| `munch KIND over EXPR { "…" … }` | Longest-match operator tokens over a run of operator bytes (§2.4). |
| `keywords KIND { "…" … } [soft { "…" … }] [required]` | Retypes tokens of `KIND` whose text is a keyword. With `required`, a token of `KIND` that is none of them is an error. |
| `check KIND matches "REGEX"` | Every token of `KIND` (after `keywords`) must match `REGEX` in full, or the file goes to tree-sitter. Lowered as masks when it can be (§3.3); the lexers here write those masks out instead. |
| `skip EXPR` | Bytes that may belong to no token, such as whitespace. |
| `error "MESSAGE" when EXPR` | If any bit of `EXPR` is set, the file goes to tree-sitter. |
| `tree NAME { … }` | The tree section: where each node starts and ends, for the tape (`13-tape-on-the-gpu.md`). It holds the statements below, and masks and scans of its own. |
| `node KIND start S end E [level L]` | A node at each bit of `S`. It ends at the first bit of `E` at or after its start where count `L` after the byte equals `L` before the start; without a level, at the first bit. Listed order is rank: outermost first at one byte. |
| `field NAME of KIND … = EXPR`, `supertype NAME of KIND … = EXPR` | A node of those kinds starting at a bit of `EXPR` has the field, or is in the supertype. |
| `extra KIND …` | Nodes of these kinds carry the extra flag. |
| `untoken KIND …` | Tokens of these kinds are not nodes; a `node` statement makes those nodes instead. |

### 2.2 Mask expressions

Bit `i` is byte `i`. "Earlier" means a smaller `i`.

| Expression | Bit `i` is set when … |
|---|---|
| `a & b`, `a \| b`, `a ^ b`, `~a` | bitwise and, or, xor, not |
| `prev(a)`, `prev(a, k)` | `a` is set at `i − k`: the fact holds for the byte `k` before |
| `next(a)`, `next(a, k)` | `a` is set at `i + k` |
| `starts(a)` | `a` is set at `i` and not at `i − 1` |
| `lasts(a)` | `a` is set at `i` and not at `i + 1` |
| `after(a)` | `a` is clear at `i` and set at `i − 1` |
| `span(a, n)` | some bit of `a` is set at `i − n + 1 … i`: the `n` bytes from each bit |
| `toggle(a)` | an odd number of bits of `a` are set at `0 … i`: prefix XOR |
| `fill(s, e)` | some bit of `s` is set at `j ≤ i`, and no bit of `e` at `j < k ≤ i`: from each start up to, not including, the next end |
| `back_fill(s, e)` | the mirror of `fill`: some bit of `s` is set at `j ≥ i`, and no bit of `e` at `i ≤ k < j`. It carries a fact back to the start of the bytes it belongs to |
| `after_odd_run(a)` | `i − 1` ends a run of set bits of `a` whose length is odd: the byte after an odd run of backslashes |
| `run(a, n)`, `run(a, n+)` | `i` starts a run of exactly `n`, or at least `n`, set bits of `a` |
| `line_has(a, nl)` | `i` is a bit of `nl`, and some bit of `a` is set since the previous bit of `nl`: newlines of lines that hold an `a` |
| `aligned(a, n)` | `i` is `0, n, 2n, …` bytes from the start of its run of `a` |
| `head(a, k)`, `head(a, C)` | `i` is among the first `k` bytes of its run of `a`; with a count `C`, among the first `C` bytes, `C` read before the run's first byte |
| `text("…")` | the input at `i` starts with the given bytes |
| `M.in(S)` | machine or scan `M`'s state at byte `i` is `S` or a member of group `S` (§2.3) |
| `M.on(A -> B)` | `i` is an event of `M` that moves it from a state in `A` to a state in `B` |
| `M.final(S)`, `M.reached(S)` | (file-level) `M` ends in `S`; `M` is ever in `S` |
| `C.eq(k)`, `C.ge(k)`, `C.eq(D)` | count `C` after byte `i` is `k`, at least `k`, or equal to count `D` |
| `C.frame(a)` | the frame open before byte `i` (the innermost, by `C`) was opened by a byte of `a` |
| `C.since(a)` | that frame holds a byte of `a` at its own level, before `i` |
| `T*.in(S)`, `T*.on(A -> B)`, … | the OR of the same test over every instance of the machine template `T` (§2.5) |
| `at_end(a)` | (file-level) `a` is set at the last byte |

A byte that is both a start and an end of `fill(s, e)` starts a new fill: the end stops the fill
coming in, and the start begins another. So a fill meant to cover only the bytes of a token must
take its starts from those bytes: `fill(prev(dot) & num, ~num)`, not `fill(prev(dot), ~num)`.

### 2.3 Machines

```text
machine NAME {
  events { SYMBOL = EXPR … }      # event bytes; where several match, the first listed wins
  states S1 S2 i[0..4] …          # i[0..4] declares i0 … i4
  group G = S1 S2 i[*] …          # names for sets of states, for in() and on()
  start S
  final S …                       # states the input may end in; the default is the start state
  S on SYMBOL -> T, on SYMBOL -> T, …
  S on A | B -> T                 # the same transition for several symbols
  S on any -> T                   # every symbol not listed for S
  i[d] on SYMBOL -> i[d+1]        # one rule per index; a target out of range is `error`
  i0 on SYMBOL -> T               # a rule on a named state replaces the indexed one for that symbol
}
```

- **Size.** Up to 16 states is one table lookup per event; up to 64 is four. Larger machines, and
  ways to avoid them, are in `05-many-states.md`.

- **Event bytes.** Only bytes of the event masks are events. Every other byte keeps the state of
  the last event before it.
- **Transitions.** A (state, symbol) pair that is not listed keeps the state, except in a state
  named `error`, which keeps itself on every symbol.
- **What `in` sees.** `M.in(S)` at byte `i` is the state after the events at bytes `≤ i`. So an
  opening quote is already inside the string, and a closing quote is already outside it, the same
  convention as `toggle`.

### 2.4 Tokens

A token is a start byte and the bytes after it that belong to it.

- `token KIND = start S through B` makes one token at each bit of `S`. It covers that byte and the
  following bytes of `B`, up to the first byte outside `B` or the next token's start. Without
  `through`, a token is one byte.
- `munch` makes operator tokens by longest match, as Python's tokenizer does: `**=` is one token
  and `=-` is two.
- `keywords` retypes word tokens by their text. Soft keywords (`match`, `case`, `type`, `print`,
  `exec`, `__future__`, `_`) keep a soft bit for the grammar layer to decide (`02-python.md` §5.1).

The lexer's output is the token stream, in byte order: each token's kind, start and end.

**Implicit checks** become error bits, without being written:

- two tokens overlap;
- a byte is in no token and not in a `skip` mask;
- a machine ever reaches `error`, or does not end in one of its `final` states.

### 2.5 Families: machines and blocks

A `block` is a list of statements with mask parameters. Each instance evaluates them once, with
its own arguments, and its names are read as `INSTANCE.NAME`:

```text
block fstrings(body, end, raw, triple) { rounds 8 { … }  mask iopen = fopen & d0  … }
block f1 = fstrings(fbody, fend, raw, fbody & tstr)            # f-strings
block f2 = fstrings(f2_body, f2_end, f2_raw, empty)            # f-strings inside f1's interpolations
mask s_open = opened | fstrings*.iq_open
```

- **`fstrings*.NAME`** is the OR of `NAME` over the instances, so later masks need not name each
  one. Tests work the same way: `fstrings*.inner.in(s)`.
- **Cascades.** A later instance may take as input masks computed from an earlier one. This is
  the cascade of `05-many-states.md` §2.3: each nesting level is one more instance, and a fixed
  number of levels is a fallback bound.
- Machines take parameters the same way (`machine NAME(P, …) { … }`,
  `machine NAME = TEMPLATE(…)`). The reference evaluator runs only machines without them.

**Per-file variants.** A lexer may describe two ways to compute the same masks, and choose one
per file by a test that is exact for that file. `lexers/json.bitlex` does this: a file with no
`/` outside `toggle(q)`'s strings has no comments, so its strings are `toggle(q)` and the `lex`
machine does not run. The choice is one decision per file, like fallback.

### 2.6 Unicode classes

```text
ucode over code & high {
  class xid_start    = table "parser.c" sym_identifier_character_set_1
  class xid_continue = table "parser.c" sym_identifier_character_set_2
  class uspace       = U+FEFF U+2060 U+200B
}
```

- **The classes are masks.** Each class is set on every byte of each character in it, and is
  used like any `class` after that.
- **Decoding is gated.** Only characters whose lead byte is in the `over` mask are decoded. That
  mask is built from ASCII facts, which are exact on UTF-8.
- **Fidelity.** A class is a table from the grammar's generated `parser.c`, a list of code
  points, or a Unicode property.

`04-unicode-and-numbers.md` gives the design, the lowering and the measured cost.

### 2.7 Scans, hypotheses and rounds

```text
scan qq    = reflect(sq -> s, dq -> d) reset tstr      # states out, s, d
scan depth = count(up, down) reset ~body max 8         # +1, -1, never below 0, at most 8

hypotheses line split nl & ~icode lanes out s3 d3 {
  rounds 6 until settled {
    carry hidden = empty
    scan tq      = reflect(t1 & ~hidden -> s3, t2 & ~hidden -> d3) start lane
    …
    next hidden  = …
  }
  reject "a short string still open at the end of its line" when last & short
  exit tq
}
```

**`reflect(A -> S, B -> T)`** has three states, numbered `out` = 0, `S` = 1 and `T` = 2. A byte of
`A` sends `x` to `1 − x mod 3`; a byte of `B` sends it to `2 − x`. The other kind's state is the
reflection's fixed point, so a `"` inside `'…'` changes nothing.

- **`reset R`:** at each byte of `R` the state is `out`, and the scan goes on from there. A byte
  of `R` is never an event.
- **Tests:** `in()` and `on()`, as for a machine. `on(out -> s | d)` holds at an opening quote.

**`count(UP, DOWN)`** adds 1 at each byte of `UP` and subtracts 1 at each byte of `DOWN`, except at
0. With `reset R` it is 0 at the bytes of `R`.

- **`max D` is required.** A count above `D` is an error (inside `hypotheses`, a rejection), and
  the count stays at `D + 1` from there. The bound sets the width of the count's lanes, and
  `frame`, `since` and `head(a, C)` lower to one fill or shift per level up to `D`. Besides `eq` and `ge`, it answers two questions
about the bracket frames it counts:

- `C.frame(a)`: whether the frame around byte `i` was opened by a byte of `a`. The f-string lexer
  asks whether that frame is a replacement field or a bracket.
- `C.since(a)`: whether that frame already holds a byte of `a` at its own level. The lexer asks
  whether the field has passed its `:`, so that the byte is format-spec text.

**`hypotheses NAME split E lanes L1 L2 …`** cuts the input into segments; a byte of `E` ends its
segment. Its statements run once per lane, and in each run:

- every segment is treated as a whole input: `prev`, `next`, fills, scans and run operations do
  not cross a segment edge;
- `start lane` makes a reflect scan start each segment in the state named like the lane;
- `first` and `last` are the first and last byte of each segment;
- `reject "…" when E` marks the segments where `E` holds as impossible in that lane;
- `exit S` names a scan whose state at a segment's last byte is the lane the next segment starts
  in.

The real lane of the first segment is the first lane; each segment's exit gives the next one's.
After the block, every name defined in it holds, on each segment, its value in that segment's
real lane. A rejected real lane, or a last segment that exits in a lane other than the first, is
an error.

**`rounds N { … }`** evaluates its statements up to `N` times. A `carry` gives a mask its value
for the first round; each `next` gives it the value for the round after. When a round leaves every
carry unchanged, it has reached the fixed point, and the names keep that round's values.

- Inside `hypotheses`, the segments whose carries still change after `N` rounds reject their
  lane. Elsewhere, not settling is an error.
- Errors reported during a round count only in the last one.
- **`until settled`** lets a backend stop as soon as a round leaves every carry unchanged. That
  is a test per chunk of input per round, not per byte. Without it every round runs, so the cost
  is fixed. The values are the same either way: a settled round repeats itself.

The lexers use rounds only where the fixed point is unique: every role is decided from bytes
before it, so by induction from the left there is one self-consistent answer
(`08-without-automata.md` §3.3, §5.1).

## 3. Lowering

### 3.1 Masks

Masks are computed 64 bytes at a time, one `u64` per mask per block. A mask that looks across a
block boundary carries a small state from block to block:

| Operation | NEON | Metal | Carried between blocks |
|---|---|---|---|
| `class` | two nibble lookups (`vqtbl1q_u8`) and an AND, then a bit-pack. Up to 8 classes per lookup pair | one lookup per lane, then a ballot | nothing |
| `& \| ^ ~` | scalar or vector logic | logic | nothing |
| `prev(a, k)` | shift in from the previous block | read the lane `k` before, from threadgroup memory | the last `k` bits |
| `next(a, k)` | shift in from the next block (the pipeline runs one block ahead) | read the lane `k` after | the first `k` bits of the next block |
| `toggle` | carry-less multiply by all ones (`vmull_p64`) | the popcount of a ballot below the lane (Metal has no `simd_prefix_exclusive_xor`), then a scan between groups | one parity bit |
| `fill(s, e)` | `((e − s − borrow) & ~e) \| s`: the borrow runs from each start to the next end | the same, per 64-bit word, with the borrow passed by a scan | one borrow bit |
| `back_fill(s, e)` | `fill` on bit-reversed words (`rbit`), blocks taken last to first | the same, scanning the other way | one borrow bit, passed backwards |
| `after_odd_run` | simdjson's add-with-carry over odd and even run starts | the same per word, with the carry passed by a scan | one carry bit |
| `starts` `lasts` `after` `span` `run` | `prev`/`next` and logic | the same | as `prev`/`next` |

Every carry is a single bit, and every carry combines associatively:

- a parity, by XOR;
- a borrow or carry, as a generate and propagate pair, like a carry-lookahead adder.

So on Metal a block's carry comes from a prefix scan over the blocks before it, and every block
runs at once. On the CPU the carry passes from one block to the next in a register.

### 3.2 Machines

1. **Symbols.** Each event byte gets its symbol number by selects over the event masks
   (`vbslq_u8`), in their listed order.
2. **Compaction.** The event bytes and their symbols are compacted into a dense list:
   - per 8 bytes, one 256-entry shuffle table packs the event bytes;
   - the write pointer advances by the popcount.

   The work is the same for every block, whatever its contents.
3. **Composition.** Each symbol's transition map is a 16-byte vector. Composing one event is one
   `vqtbl1q_u8`. On the CPU, the list is cut into chunks whose maps compose independently. On
   Metal, each SIMD group composes a chunk, and a scan composes the chunks' maps.
4. **States per event.** Each event's state is its prefix map applied to the start state.
5. **Back to bytes.** For each state, or for each bit of the state number, a `fill` from the
   events that enter it to the events that leave it. That gives `M.in(…)` for every byte.

### 3.3 Tokens

- **Starts.** Each token kind's start mask is selected into a per-byte kind number. The union of
  start masks is compacted into the token list, as in §3.2.
- **Ends.** The last byte of each token is `B & (next(starts) | ~next(B))`. That mask is compacted
  too, and it aligns with the starts one for one.
- **`munch`** has a bounded fixed point. A candidate operator wins unless a winning operator that
  starts one or two bytes earlier covers it. The rule is applied a fixed three times. If the third
  and fourth applications differ, a run of chained candidates (such as `=====`) was longer than
  the bound, and the error bit is set.
- **`check`, flattened.** When the regex is a fixed sequence of parts (literals, classes, `?`,
  `+` and `*` on one class, and alternatives that differ only in a class), the compiler turns it
  into masks, as the number rules in both lexers do by hand. These are the masks:
  - a region per part, a `fill` from each separator to the token's end;
  - per region, the classes it allows;
  - `prev`/`next` rules for each pair of adjacent parts;
  - "at most once, in this order" rules for the separators.

  Every tree-sitter number, escape and comment pattern seen so far has this form
  (`04-unicode-and-numbers.md` §4.2).
- **`check`, otherwise,** is a machine whose events are the token's bytes, which a reset symbol at
  each token start returns to the start state. The token's last byte must leave it in an
  accepting state. With at most 16 states, it is one shuffle per byte of those tokens.
- **`keywords`** runs per token, not per byte. A word's first 8 bytes and its length index a small
  perfect hash, and one 8-byte compare confirms the match.

### 3.4 What the compiler guarantees

- **No data-dependent branch, anywhere.**
  - Loops run over blocks, over chunks of the event list and over chunks of the token list. Their
    trip counts are sizes.
  - Every per-byte decision is a mask, every per-event decision a shuffle, and every per-token
    decision a lookup.
- **Linear work.**
  - A constant amount per byte for masks;
  - a constant amount per event for machines;
  - a constant amount per token for the token list.
- **A complete error signal.** Every way a file can fall outside the lexer is an error bit,
  explicit or implicit (§2.4). One OR per file decides whether it goes to tree-sitter.

### 3.5 Scans, hypotheses and rounds

| construct | lowering | carried between blocks |
|---|---|---|
| `reflect` | `toggle` of the events for the sign, then a prefix sum mod 3 over two bit planes; a reset is a segmented scan | parity + a value mod 3: 3 bits |
| `count` | a prefix sum in bytes. The floor at 0 makes each step `x ↦ max(x + a, b)`, which composes, so it is still a scan | the count |
| `C.frame`, `C.since` | matching by depth, as brackets are matched in `01-branch-free-design.md` §3 | the open frames' flags |
| `hypotheses` | every lane computed for every segment; each segment's action is a map on the lanes (three 2-bit fields), composed by a scan of selects; each byte then selects its lane's masks | one map |
| `rounds` | unrolled `N` times, with a convergence bit per segment or per file | per round, as its body |

`08-without-automata.md` §6 and §7 work these through for a `"""` split by a block edge, and for the
cost of running every lane and every round.

## 4. The examples

- `lexers/json.bitlex`: tree-sitter-json's tokens, comments included.
  - Masks only, for a file without comments.
  - The 7-state `lex` machine otherwise.
- `lexers/python.bitlex`: tree-sitter-python's leaves, with no machine.
  - Strings and comments by `reflect`, per line under three `hypotheses`, with a `rounds` fixed
    point for hidden triple quotes;
  - f-string and t-string fields by `count`, `frame` and `since`, in two `block` instances,
    format specs at every depth;
  - Python 3.12 quote reuse, by an outer `rounds` that feeds interpolation code back to the
    string scan;
  - Unicode identifiers and the three non-ASCII whitespace characters, by `ucode`;
  - prefixes, raw and bytes strings, and escape sequences;
  - numbers, words and keywords;
  - operators by `munch`;
  - line continuations and comments.

  Layout (NEWLINE, INDENT, DEDENT) is not a token here. It is stage 3 of `02-python.md`, which
  reads this lexer's masks.

## 5. The reference evaluator, and what it found

`tools/bitlex` reads a `.bitlex` file and runs it on one input. Masks are word operations, with
their carries passed from word to word; scans, counts, machines and everything inside
`hypotheses` run byte by byte. It gives the semantics, not the speed.

```text
cargo run --release --offline --manifest-path tools/bitlex/Cargo.toml -- diff LEXER LIST
    tokens against tree-sitter's leaves, for every path in LIST
… -- strings LEXER LIST    Python: string and comment bytes against the old 13-state machine
… -- fstr LEXER LIST       Python: interpolations, format expressions and specs, against
                           tree-sitter and the f-string oracle
… -- tokens LEXER FILE     the tokens and errors of one file
… -- tree LEXER LIST       the tree section's nodes against tree-sitter's, per node in
                           preorder: kind, field, supertypes, extra, span and size
… -- nodes LEXER FILE      both node lists of one file, side by side
```

Tokens are compared with tree-sitter's leaves; a `string_content` with escape children counts as
the pieces between them, as the lexer emits it.

| corpus | files tree-sitter parses | tokens equal to tree-sitter's | lexer falls back |
|---|---:|---:|---:|
| data-layer JSON | 14,668 | 14,668 | 0 |
| data-layer Python | 13,042 | 13,042 | 0 |
| CPython 3.15 standard library | 1,993 | 1,990 | 3 |

- **The 3 CPython fallbacks:**
  - `test_fstring.py`, with a triple-quoted string inside an interpolation;
  - `syntax_warnings.py`, whose `1or 0` is a number run into a word;
  - `test_string_literals.py`, with a run of 21 quotes inside a short string whose grouping does
    not settle.
- **F-string fields** equal tree-sitter's in every file that does not fall back. That is 42,956
  f-strings in the data layer and 5,998 in CPython.

**What the differential test found.** Every one of these was a wrong belief about tree-sitter, or
a bug in a lexer as first written. The lexers had not been run before:

- in both lexers, an escape starts at an even offset in its run of backslashes (`aligned(backslash, 2)`);
  "not escaped" is not enough, because the second `\` of `\\` is not "after an odd run";
- a fill whose start may fall outside the token runs into the next token (the `fill` note in §2.2);
- in a non-raw string, a backslash that starts no escape is an anonymous `\` leaf;
- in a raw string, `\`, `\\` or `\` + quote just before the closing quote belong to `string_end`;
- `\N`, `\u` and `\U` are content in a bytes string;
- a Python comment keeps its `\r`, and ends at a newline even after a backslash;
- `t"…"` (PEP 750) has interpolations like `f"…"`;
- a run of `.` after `from` is `.` tokens, not an ellipsis;
- a `.` ends a number after an exponent or another `.`, or in `0x…`.

**Rounds taken** (files by the most rounds any block needed, the confirming round included):

| | 1 | 2 | 3 | 4–5 | 6–7 | not settled |
|---|---:|---:|---:|---:|---:|---:|
| data layer: triple quotes inside lines | 11,954 | 1,029 | 55 | 4 | | |
| data layer: f-string fields | 10,661 | 2,127 | 254 | | | |
| data layer: quote reuse (outer) | 8,304 | 4,737 | 1 | | | |
| CPython: triple quotes inside lines | 1,851 | 109 | 24 | 7 | 2 | |
| CPython: f-string fields | 1,714 | 237 | 38 | 3 | | 1 |
| CPython: quote reuse (outer) | 1,314 | 673 | | 3 | 1 | 1 at 8, 1 not settled |

A file without an f-string settles the outer round in 1. A file with f-strings needs 2: one, and
one to confirm that no interpolation ends open. A compiler can skip the confirming round in that
case (`lexers/python.bitlex` §4).

## 6. Next

`09-compile-and-lower.md` sets out how the IR, the interpreter and the compiled forms fit together.

1. **NEON and Metal lowerings** of the same programs, in the block-parallel form of
   `06-block-parallel.md`, checked against the evaluator.
2. **The tree stages** of `01-branch-free-design.md` and `02-python.md`, from the token masks to
   tapes, checked against `TapeImage::encode` the same way.
