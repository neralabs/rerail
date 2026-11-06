# Bitlex: a lexer language that lowers to branch-free SIMD

Status: language proposal, 2026-11-06. The lexers in `lexers/` are written in it. No compiler
or evaluator exists yet, and neither lexer has been run.

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

### 1.2 Over what does the automaton run?

Over the whole file, but only over the bytes that can change the mode. Whether a `"` opens a
string depends on everything before it: an earlier `#` may have made it part of a comment, and an
earlier `'''` may have made it part of a string. So no stretch of the file can be lexed without
knowing the state at its start. But most bytes cannot change the state.

One automaton covering strings, comments and f-string interpolations together needs about 50
states. Two automata with 16 states or fewer are better, because each one's map then fits one
NEON register:

| Automaton | Runs over | States | Events (data-layer Python) |
|---|---|---|---|
| `strings` | every unescaped quote, every `#`, and each newline that ends a line containing a quote or `#` | 13 | one per 53 bytes (1.87% of bytes) |
| `fstrings` | braces, brackets, `:`, quotes and `#` inside f-string bodies, and each f-string's closing quote | 14 | only inside f-strings |

- **`strings` treats an f-string as a plain string.** Before Python 3.12, an interpolation could
  not contain the f-string's own quote character, so the string still ends at the first unescaped
  matching quote. Raw and byte strings need no special case either: in Python, a backslash keeps
  a quote from ending even a raw string.
- **`fstrings` runs only inside f-strings.** It finds the interpolations. It also detects what
  `strings` gets wrong. A Python 3.12 f-string that reuses its own quote inside an interpolation
  (`f"{f"{x}"}"`, or `f"{'"'}"`) makes `strings` end the string while an interpolation is still
  open. `fstrings` then sees the closing quote in a state other than literal text and sets the
  error bit. The file goes to tree-sitter.
- **Nothing is lost.** Every file that `strings` mislexes is caught by `fstrings`. In the corpus,
  strings inside interpolations occur in 5.17% of files, and almost all of them use the other
  quote character. How many files reuse their own quote has not been measured yet.

Where plain bit operations are enough, no automaton is used at all. JSON needs none. Its escapes
come from simdjson's carry trick (`after_odd_run`), and its strings from one prefix XOR
(`toggle`).

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
| `token KIND = start EXPR [through EXPR]` | Tokens of one kind (§2.4). |
| `munch KIND over EXPR { "…" … }` | Longest-match operator tokens over a run of operator bytes (§2.4). |
| `keywords KIND { "…" … } [soft { "…" … }]` | Retypes tokens of `KIND` whose text is a keyword. |
| `check KIND matches "REGEX"` | Every token of `KIND` (after `keywords`) must match `REGEX` in full, or the file goes to tree-sitter. |
| `skip EXPR` | Bytes that may belong to no token, such as whitespace. |
| `error "MESSAGE" when EXPR` | If any bit of `EXPR` is set, the file goes to tree-sitter. |

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
| `M.in(S)` | machine `M`'s state at byte `i` is `S` or a member of group `S` (§2.3) |
| `M.on(A -> B)` | `i` is an event of `M` that moves it from a state in `A` to a state in `B` |
| `M.final(S)`, `M.reached(S)` | (file-level) `M` ends in `S`; `M` is ever in `S` |
| `at_end(a)` | (file-level) `a` is set at the last byte |

Overlap between a start and an end in `fill(s, e)`: a start that is also an end covers only its
own byte. The lexers below never write one.

### 2.3 Machines

```text
machine NAME {
  events { SYMBOL = EXPR … }      # event bytes; where several match, the first listed wins
  states S1 S2 …                  # at most 16 (one register); up to 64 allowed (four)
  group G = S1 S2 …               # names for sets of states, for in() and on()
  start S
  S on SYMBOL -> T, on SYMBOL -> T, …
  S on any -> T                   # every symbol not listed for S
}
```

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
  `exec`) keep a soft bit for the grammar layer to decide (`02-python.md` §5.1).

The lexer's output is the token stream, in byte order: each token's kind, start and end.

**Implicit checks** become error bits, without being written:

- two tokens overlap;
- a byte is in no token and not in a `skip` mask;
- a machine ever reaches `error`, or does not end in its start state.

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
| `toggle` | carry-less multiply by all ones (`vmull_p64`) | `simd_prefix_exclusive_xor`, then a threadgroup scan | one parity bit |
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
- **`check`** is a machine whose events are the token's bytes, which a reset symbol at each token
  start returns to the start state. The token's last byte must leave it in an accepting state.
  The regex compiles to a DFA. With at most 16 states, it is one shuffle per byte of those tokens.
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

## 4. The examples

- `lexers/json.bitlex`: tree-sitter-json's tokens.
  - Masks only, no machine.
  - Comments set the error bit for now (`01-branch-free-design.md` §5).
- `lexers/python.bitlex`: tree-sitter-python's leaves.
  - The `strings` and `fstrings` automata;
  - prefixes, raw strings and escape sequences;
  - numbers, words and keywords;
  - operators by `munch`;
  - line continuations and comments.

  Layout (NEWLINE, INDENT, DEDENT) is not a token here. It is stage 3 of `02-python.md`, which
  reads this lexer's masks.

## 5. Next

1. **A reference evaluator.** It reads `.bitlex` and evaluates the masks a block at a time with
   plain `u64` operations. Each machine is evaluated as a table walk, so the evaluator stays simple
   and gives the semantics.
2. **The differential check** of each lexer's tokens against tree-sitter's leaves:
   - all 14,669 JSON files;
   - the 13,042 Python files;
   - CPython's standard library.
3. **NEON and Metal lowerings** of the same mask graph, checked against the evaluator.
