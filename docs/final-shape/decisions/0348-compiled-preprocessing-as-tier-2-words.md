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
`rir` (`rerail-c-directives`' `expand/expand.rir`, page 12 §2.4's route), assembled and lowered
by the same compiler.
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

## Stage 1, built: the whole file's tokens

`rerail-c-directives`'s `bitlex_tokens/tokens.bitlex` is a grammar of its own beside the
directives' ladder, so mode (a) keeps its lean tape.
- **One text, two dialects.** It is C++'s grammar. C's leaves out the lines marked `C++ only`:
  `->*`, `<=>` and `.*`, a literal's suffix, and C++'s `<::` rule. Each is compiled once a
  process (`tokens::compiled`).
- **The tape:** a `line` node for each line's tokens, which gives the lean lexer's `BOL`, and a
  leaf a token: an identifier, a pp-number, a literal (its prefix and C++ suffix included),
  `punct`, `digraph`, `hash`, `hashhash` or `other`. A token's `SPACE` is the byte before it,
  which the reader tests (`tokens::read`). A mark takes no node on a tape, so it cannot carry a
  flag.
- **Operators by `munch`** (03-bitlex §3.3). Its masks were new to the compiler; they are now the
  Bitlex parser's expansion of the statement, so the evaluator and every backend read one
  definition. The fixed point runs four times, and a chain longer than that is refused.
- **Refused, to the lean lexer:** what the directives' grammar refuses, plus a byte past ASCII in
  code (an identifier's by its XID class), a `\` in code that splices no line (`\u` names), a
  splice between two tokens' bytes, and C++'s `<::`. A configuration other than C23 or C++26
  with GNU goes to the lean lexer as well.
- **Corpus:** 3,058 files' tokens equal the lean lexer's, kinds, bytes and flags; 77 refused;
  0 differ. The interpreter, Cranelift and Metal agree on every file.
- **A backend difference found here.** A start of a token's own kind inside its run (`"a""b"`,
  the second `"`) is a malformed grammar: Bitlex's evaluator says "token starts inside the one
  before". The backends then differ. Metal's last-byte mask (`TokenRule::last`) stops before
  that start, while the CPU's and the interpreter's run rule reads through it. Making the rule
  `R & ~S` gave every backend Metal's answer, but it cost JSON's tier 2 its shape (30 calls of
  its own, where it has none), so it was not kept. The grammar keeps its starts out of its runs
  (`run lit_bytes & ~lit_start`). Work queue Q32 has the guard this needs.

## Stage 2, built: the words, on the interpreter

The words are `rerail-compiler`'s `pp_words` (opcodes `0x67`–`0x76` in `program.rs`), and the
program is `rerail-c-directives`' `expand/expand.rir` (84 words; C's leaves out its `C++ only`
lines).
- **The words.** Each acts on one store, and a test runs the next word or skips it, as tier 2's
  tests do:
  - `READ` and `LINE` read: the next token unexpanded, the directive line's next token;
  - `IS`, `PEEK` (the next token, unread), `LOOKUP` and `MACRO` test;
  - `PAINT` and `PUT` act on `T`;
  - `FRAME`, `ARG` (its modes: append, `(`, `)`, `,`, the count, drop), `PRE`/`KEEP` and
    `SUBST` make a call;
  - `DEFINE`, `UNDEF` and `SKIPLINE` are the directives.
- **The control is the program's.** `one` (lean `Pp::next`), `raw` (`lex_raw`), the argument
  loop and the pre-expansion loop are `rir`, with `PUSH`/`RETURN` calls. A call's arguments are
  expanded by calling `one` again, over a stop source.
- **A run of its own.** `pp_words::run` beside `stream::run`: the parser's words act on nodes
  and marks over tier 1's records, these on sources, frames and a table. One program has one
  family, and `rir_asm::validate` sends it to its verifier: every cycle reads a token, a call
  counting as a read where every way through its callee reads.
- **Exact where the order shows.** An argument is expanded where the substitution first asks
  for it, in the lean pass's order, and one asked for only inside a `__VA_OPT__` only where the
  variadic argument is present (`Def::pre`, made once per `#define`): `__COUNTER__` and the
  diagnostics will see the same order.
- **A paste is a table.** The kinds of the two tokens and the punctuators (`ppop`, the token
  grammar's `munch` list, `%:` and `%:%:`) decide it, and `ppsuffix` (C++) lets a literal take an
  identifier. A raw string's prefix pasted refuses the TU.
- **Spelling.** A file token is its bytes; a literal a splice is inside is spelled by phase 2,
  in a second buffer of the TU (`expand::tape`).
- **Found and fixed on the way:**
  - The lean pass dropped a pasted digraph's `DIGRAPH` flag (`<` `##` `:`), so it read unlike the
    same `<:` from a file.
  - Stage 1 refused every `\` in code. Only `\u` and `\U` need refusing; a lone `\` is now an
    `other` token, as the lean lexer reads it.
- **Tests:** the standard's examples (C17 §6.10.3.3–5, C++20 `__VA_OPT__`), GNU's comma and
  named variadics, painting, calls across lines and with the wrong count, every kind of paste,
  and stringizing. Each is run in C and in C++, and equals the lean pass's tokens, kinds and
  flags. Corpus: of the 3,135 files, 1,681 are in stage 2's reach (no conditionals, `#embed` or
  built-ins), and all 1,681 expand as the lean pass does.

## Stage 3, first part: conditionals and `#if`

- **A group not taken is never read.** Mode (a)'s chains (`c_cond_next`) are mapped onto the
  token tape once per file (`expand::chains`: each conditional's `#` and its chain's next one's).
  `SKIP` moves the file's cursor from the `#` it read last to the next of its chain. The lean
  pass's `skip_group` lexes every skipped line; the words jump.
- **The words:** `COND` (the file's conditional stack: open taken or not, the chain taken, its
  `#else` seen, closed, and two tests), `IF` (an `#if`'s line as a stop source and a level, then
  its value), `DEFINED` (the operator, live only while an `#if`'s line is expanded, `IS m=4`).
- **`EVAL`'s meaning is `pp_expr`,** the lean pass's evaluator over the words' tokens (kinds and
  spellings, C++'s alternative tokens and `true` with `ppcpp`). It is recursive descent in the
  interpreter; the generated code needs its explicit stack (Cranelift, stage 4).
- **The program** is `expand/expand.lab`, labeled; `gen.py` numbers it into `expand.rir` (208
  words).
- **Refused:** a file mode (a) refuses (60 of the corpus: a group open at the file's end among
  them) goes to the lean pass.
- **Corpus:** 3,002 of 3,135 files expand as the lean pass does, 0 differ. Not run: 57 use a
  built-in macro, 1 a directive not read yet (`#embed`), 75 a grammar refuses.

## Stage 3, second part: includes and pragmas

- **Resolution is the host's, before the words run.** An include whose header name mode (a)
  read (`c_include`, quote or angle form) is resolved by the host for each file it reaches, and
  the file's table (`File::includes`) maps its `#` to the file. `INCLUDE m=0` looks the `#` up
  and passes the line unread.
- **A name the expansion makes** (`#include MACRO`): `IF m=2` expands the line as `#if`'s is
  expanded (no `defined`), and `INCLUDE m=1` forms the header name (the lean pass's
  `header_of`) and looks it up in the file's `named` table. A name not there is returned in
  `Expanded::wanted`; the host resolves it and runs the TU again. This is the import fixpoint
  of the plan, here in the host (`expand_tu`, 16 rounds at most). In the engine, the answerer
  returns `c_pp_include_wanted` and the fixpoint re-asks.
- **`INCLUDE m=2`** enters the file: not when it is not found, when `#pragma once` or `#import`
  has entered it, or past 200 files deep. A file's end pops it, except while a call's arguments
  are read (they end with the file, clang's reading and the lean pass's).
- **`PRAGMA`**: `once`, `push_macro("X")` and `pop_macro("X")` (a stack per macro slot).
- **Tests:** six TUs of several files (guards, not found, once and import, names made by
  macros, `#include_next` over two system dirs, macros and conditionals across files, a call
  ended by its file, push and pop) equal the lean pass in C and C++. The corpus is unchanged.
- **Also fixed:** the verifier counted `DEFINED` as a read on both sides; where it does not
  hold it reads nothing.

## Open

- **Header units:** a header expanded once per entry macro state (0327, open).
- **Hash collisions:** a key match compares bytes, so a collision costs time, never a wrong
  answer.
