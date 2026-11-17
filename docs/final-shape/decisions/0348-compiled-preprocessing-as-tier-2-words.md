# 0348: Compiled preprocessing (mode (b)) is tier 2 words over a re-entrant token source

Status: draft (2026-11-17). Step 8 of 0327's plan.

## Intro

Decision 0327 has two modes:
- **(a)** records each file's directives as facts;
- **(b)** applies them to a translation unit (TU) in one forward pass, under a configuration.

Mode (a) is compiled now (0327, steps 1–6). It is a Bitlex grammar on the interpreter, Cranelift
and Metal, and it reads 3,075 of the corpus's 3,135 files as the lean scan does.

Mode (b) is the lean preprocessor today (`rerail-c-pp`, `stream.rs`, 1,616 lines). It is checked
against `clang -E`. This decision says how mode (b) runs in the transducer. It is a different kind
of work from mode (a):

- **Mode (a)** is a function of one file's bytes. It is data-parallel, and every byte is read once.
- **Mode (b)** is sequential per TU. A macro defined on line 10 changes how line 11 reads, and
  an `#include` switches files midway. Expansion reads tokens again: a macro's body, then its
  arguments, then the result once more.

So the question is which primitives the sequential part needs, where they live in the
transducer, and what is done ahead of time, data-parallel.

## Prior work consulted

- **Page 12 (`docs/parser-generators/12`):** one format. Everything goes into the transducer's
  `Program`, its digest, `validate`, `rir` and its backends. There is no module beside it.
  Bit-parallel primitives are a side program. Primitives over the token stream are tier 2 words,
  as 0087 added `TOKEN`, `SWITCH`, `PUSH` and the rest.
- **0087 and doc 20 §6:** tier 2's words are a loop over the token stream with a call stack
  (`PUSH`/`RETURN`). The shape guard allows a loop whose turn steps a token, and refuses calls
  and byte loops. Doc 20 lists tier 2 on Metal as open: "the words lower the same way … nothing
  generates it yet".
- **0027 (device work queue):** device-decided loops run on one persistent dispatch with an
  atomic queue and no unbounded wait.
- **0034 (no thread memory):** a Metal thread's stacks and arrays live in device pools whose
  size the plan sets, never in private arrays.
- **The memory notes:** "No Rust calls; transducer executes" (no bespoke Rust or MSL on the
  parse path). "Tapes never generate implied tokens". "No per-byte loops" (word ops over a
  window).
- **The lean preprocessor:** clang's painting rather than Prosser's hide sets. A token is marked
  "no expand" when its macro is disabled, and a macro is enabled again when the reader passes
  the end of its list (`Src::Toks { def }`). There is no set algebra.

## Decision

### 1. Ahead, data-parallel, once per file content (shared by every TU and configuration)

- **The file's pp-token tape:** every preprocessing token of the file, not only the directive
  lines' tokens. It gives each token's kind, span, and the `BOL` and `SPACE` flags. It is Bitlex
  ladder step 7 of `rerail-c-directives`, so it runs on the interpreter, Cranelift and Metal. It is
  checked to equal `PpLexer`'s tokens.
- **Mode (a)'s directive tree** (0327). It gives each conditional chain's links, so a group not
  taken is skipped by a jump to its chain's next directive, with no lexing. It also gives each
  `#define`'s name, parameters and body as token ranges of the tape. A body is never copied or
  re-lexed. A definition is `(file, first token, last token)` plus its parameter count and
  variadic kind.
- **An identifier's key:** a 64-bit hash of its bytes and its length, computed by word ops (eight
  bytes a step). It is a column of the tape. A table probe compares keys, and the bytes only on a
  key match.

### 2. Per TU, sequential: tier 2 words over a re-entrant token source

Expansion is a tier 2 program: words in the transducer's `Program`, written once by hand in
`rir` (`c_pp/expand.rir`, page 12 §2.4's route), assembled and lowered by the same compiler.
The configuration is data the words read (tables in `Program` and the TU's inputs), not code.

The new words and the state they act on:

- **Sources.** Tier 2's token input becomes a stack of cursors:
  - a file's tape, resident and read-only;
  - a range of the TU's token arena.

  `SOURCE` pushes a cursor. Reading past a cursor's end pops it. If the cursor carries a
  definition, popping it enables that definition again (clang's painting).
- **The token arena.** It holds a TU's made tokens (words of kind, flags and origin): the
  arguments collected, the bodies substituted, and the tokens made by pasting and stringizing.
  It is append-only, with a byte arena for made spellings. `APPEND` and `RANGE` are its words.
- **The macro table.** Open addressing, by identifier key, to a definition slot. `DEFINE`,
  `UNDEF` and `LOOKUP` are its words. Slots also hold `push_macro`'s saved definitions. A
  definition's "disabled" bit is the painting.
- **Frames.** A call's arguments are arena ranges. An argument is pre-expanded only when its
  parameter is used outside `#`/`##`, and then once (lean `pre`).
- **Integer words** for `#if`: 64-bit signed and unsigned values with the usual conversions,
  evaluated by precedence climbing over an explicit operand stack. Short-circuit is a "live"
  bit, so an unevaluated operand raises no error.
- **The conditional stack**, per file source: whether a group was taken, and whether `#else`
  was seen.
- **Includes.** Rules resolve them ahead (`c/includes.rr`). The TU's input is a table from
  (file, directive) to a file, plus a table for macro-spelled headers by spelling.
  An include whose spelling is not in either table stops the TU with `c_pp_include_wanted`, and
  the fixpoint asks again, as the CPU path does today.
- **Outputs.** The expanded token tape and its origin segments (`Expanded`), and an event arena
  that the fact rows (`c_pp_*`) are read from. The `-E` text is a reader of the token tape.

### 3. Bounds

Every loop's turn steps a token, so the shape guard holds. Each store has a bound that comes from
the work:
- the table: the TU's defines, counted from mode (a)'s rows over its closure, plus the
  predefines;
- the arena: a multiple of the closure's token count;
- the nesting limits: clang's.

Running past a bound refuses the TU, and the CPU's lean pass runs it. Expansion's worst case is
exponential, so a refusal is the answer to it, not a larger budget.

### 4. Backends

- **Interpreter:** `stream::run` with the new words. This is the meaning, checked against the lean
  preprocessor.
- **Cranelift:** the words lowered as tier 2's are, one block of code per word, in a loop over
  tokens. The arena, the table and the stacks are buffers the caller sizes from §3.
- **Metal:** one TU per thread, taken from a work queue (0027), largest first. Its stores live in
  device pools sized per TU (0034). This is tier 2 on Metal, which nothing generates yet (doc 20,
  open). Expansion is its first user, and the generator serves every grammar's tier 2.

### Rejected

- **A Rust routine on the CPU and a hand-written MSL kernel.** It breaks "the transducer executes"
  and page 12's one format.
- **A side program with its own op set** (as block programs are). Expansion reads a token stream
  with a call stack, which is what tier 2's words already are. A second program kind would
  duplicate that loop.
- **Expansion as rules in the fixpoint.** Its state depends on order, and it would be a row per
  token. Rules keep the configuration and the include resolution, where order does not matter.

## Build stages (each tested, then committed)

1. **Ladder step 7:** the whole file's pp-tokens, equal to `PpLexer`'s on the corpus, on every
   backend.
2. **The words, on the interpreter:** sources, the arena, the table and frames, enough for
   object-like and function-like macros, `#`, `##`, `__VA_ARGS__` and `__VA_OPT__`. The
   standard's examples must match the lean preprocessor's tokens.
3. **Conditionals and `#if`** by the directive tree's chains. Includes from the resolution table.
   The built-ins. The corpus's TUs must match the lean pass's `Expanded`.
4. **Cranelift.**
5. **Tier 2 on Metal,** then expansion on the work queue.

## Open

- **Header units:** a header expanded once per entry macro state (0327, open).
- **Hash collisions:** a key match compares bytes, so a collision costs time, never a wrong
  answer.
