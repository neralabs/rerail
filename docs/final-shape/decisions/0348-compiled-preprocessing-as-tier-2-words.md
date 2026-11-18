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
- **Digit separators and `u8'…'`, read since (Q35).** A separator's `'` (`1'000'000`, `0x'1`)
  must open no character constant, but the regions are found before the numbers. Both grammars
  (this one and the directives' `06-raw`) find it first, from a run of numbers made without the
  regions (`sep`). They check it after, against the code's numbers: an escape where they differ.
  That is a separator inside a character constant (`'.5'a`), or a `'` one side takes and the
  other does not. The old escape (a character constant after a digit) also refused `u8'a'`, and
  is gone. Corpus: tokens 3,073 the same, 62 refused, 0 differ; expansion 3,073 of 3,135 equal
  to the lean pass, the JIT equal to the interpreter on each.

## Stage 2, built: the words, on the interpreter

The words are `rerail-compiler`'s `pp_words` (opcodes `0x67`–`0x76` in `program.rs`), and the
program is `rerail-c-directives`' `expand/expand.rir` (84 words, one program for C and C++:
the dialect is the run's, `Env::cpp`; see "Built since").
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

## Stage 3, third part: built-ins, `#line`, `_Pragma`

- **Built-ins are definitions** in the macro table (`pp_words::BUILTINS`, the lean pass's
  list, checked by a test), so `defined`, `#undef` and `#define` treat them as the lean pass
  does. `MACRO m=2` tells one apart; `BUILTIN` makes its value, reading an operand raw to its
  `)` (a file's end held, as for arguments) and expanding it where it asks (`_Pragma`'s, a header
  name that is neither `"…"` nor `<…>`).
- **Expansions are recorded:** a token carries `exp` (its expansion + 1, as the lean pass's
  `Tk::exp`), and `Expanded::exps` each call's name and last token. `__LINE__` is the line of
  the outermost call's end. These records are also what origins (`c_pp_expansion`) need.
- **`#line` and line markers** (`LINEDIR`): the file source's presumed line and name.
  `File::lines` gives a token's physical line.
- **`Env`:** `__DATE__`, `__TIME__`, `__TIMESTAMP__` fixed (`c_fixed`); the `__has_feature`
  family answered from `c_feature` rows, and the tests not answered returned
  (`Expanded::unanswered`). `__has_include` and `__has_embed` look the name up in the file's
  `named` table: a name not there is wanted, as an include's is.
- **Corpus:** 3,059 of 3,135 files equal the lean pass, 0 differ (the 57 files with built-ins
  among them). Not run: 75 a grammar refuses, 1 `#embed`.
- **`#embed`** (`EMBED`): its name spelled (by mode (a)'s table) or made by expanding its first
  tokens, its parameters expanded and read (`limit`, `prefix`, `suffix`, `if_empty`), and its
  file's bytes pushed as numbers and commas, read again as a source. An embedded file is data:
  the host never lexes it unless it is also included (`Named::embed` keeps the two apart).

**Stage 3 is complete:** every directive and built-in the lean pass carries out. Corpus: 3,060
of 3,135 files equal the lean pass, 0 differ; the 75 a grammar refuses go to the lean pass.

## Stage 4a, built: small words, the algorithms in the program

Before Cranelift (and then Metal) lowers the words, each word is made small: its meaning is
straight code and at most one stated loop. Where a word held an algorithm (a recursion, a
nested walk), the algorithm moved into `expand.lab` as rules over smaller words. The
interpreter stays the meaning, and every backend lowers the same few dozen small words.

| Word before | The program now |
|---|---|
| `SUBST` (the body's substitution, `__VA_OPT__`, pastes, `#`) | A loop a body token a turn: `SUB` (modes 0–10: the token's parameter, `#`, `__VA_OPT__` walked in place, an argument, the output pushed), `STR` (`#`'s literal a token a turn), `GLUE` (one paste a step, in place, with read and write cursors). |
| `DEFINE` | Its name, its parameter list, its body (each token marked: a parameter, `__VA_OPT__`, `##`) and the walk of its argument uses, each a token a turn (`DEFINE` modes 0–8). |
| `IF` (`pp_expr`'s recursive descent) | Rules `cond`, `ternary`, `binary(min)` and `unary`, the evaluator's precedence climbing, over `EXPR` (modes 0–14) on the line's tokens in place. Values, pending operators, least precedences and liveness (`&&`, `||` and `?:` evaluate only what they take) are stacks. An error sets a flag, and the rules unwind with no more tokens read. A parenthesis costs four calls. |
| `EMBED`'s parameters and bytes | The parameters a token a turn (`limit` by the `#if` rules), the bytes a `,` and a number a turn. The numbers' spellings are a const table copied to scratch once per TU. |

The verifier's rule widened from "every cycle reads a token" to "every cycle steps a stated
cursor". The held side of these tests is a step: a body token taken, a `__VA_OPT__`'s content
closed, a token stringized, a paste, a `#define`'s token read or walked, an expression's token,
an `#embed`'s parameter, argument token or byte. Each is a loop head for a backend.

**No heap in the words.** The compiled code can't allocate, so the meaning doesn't either:
- A line's tokens are a range of `kept`, left in place (`line_end`).
- Header names, `_Pragma`'s and `#line`'s text, `#pragma`'s arguments, built-in strings and
  numbers, pastes and stringizing are *spellings*: a scratch range, or the input's bytes in
  place. Where they are written, they go to scratch in runs (between the escapes).
- The probe takes a token's spelling in place. Before, every identifier read copied its
  spelling to the heap.

**Primitives, shared.** The probe (`key` eight bytes a step, then a slot a step), a line's
number (a binary search of `File::lines`), the top file source and a spelling's copy are each
one stated loop. Several words use each, so they are lowered once, not split into program
loops: a dispatch per probed slot would cost more than the probe.

**Corpus** after each group: 3,060 of 3,135 files equal the lean pass, 0 differ. New examples
check `#if`'s edges (a division by zero not evaluated, unsigned arms, shifts past 63, the comma,
nested `?:`, 20 parentheses, malformed lines) and `#embed`'s (nested and unclosed arguments,
`limit`'s errors) against the lean pass.

**Left for 4b:**
- The records handed to the host on a miss (wanted files, unanswered features) are still
  `Vec`s. In compiled code they become (file, scratch range) rows.
- `_Pragma`'s text is scanned a byte at a time (`pragma_words`); the token grammar should lex
  it instead, as it lexes a file.

## Stage 4b–4d, built: the words on Cranelift

`compiler/src/native_cpu_pp.rs` lowers every word of `expand.rir` to Cranelift. `CompiledPp::run`
returns the same `Expanded` the interpreter does, and the host takes either as its runner
(`expand::expand_tu_with`).

**The shape of the code.**

- One function for the run, one block per word. A test word goes to the next word where it holds
  and the one after where it fails, as in the interpreter.
- The machine's state is a `#[repr(C)]` `Ctx`: the buffers `pp_words::run_native` reserves to the
  bounds of §3, their lengths and rooms, and the scalars (`T`, `D`, the flags). A store past a
  buffer's room refuses the TU, which then goes to the lean pass. A refusal is returned as
  `(pc + 1) | reason << 32`, one of the interpreter's reasons.
- The return stack holds a *rank*: which of the program's return points (`PUSH`'s operands) to go
  back to. Each `RETURN` is a jump table over the ranks, its own stated step (`Step::Return`).
  The plan had calls on the machine's stack, as tier 2's rules do. But a word's `RETURN` goes
  back to a return point it doesn't know, so a rank and a table are the direct form.
- Every function passes `native_cpu_shape::check`. It calls only its own functions, makes no
  indirect or tail call, and every cycle passes a stated step: `Token`, `Word`, `Pop`, `Probe`,
  `Item`, `Halve`, `Return`. No loop steps a byte: a spelling is read and written eight bytes a
  step. Its last bytes are one load, masked, where the eight lie within the spelling's page. They
  are one store where the buffer is a byte buffer with eight bytes of slack (scratch, the asked
  and wanted bytes).
- The records handed to the host on a miss are rows now: wanted files and unanswered features
  (`asked`) are each a row plus a range of bytes.

**`#if` flattened to a depth K (4c): built, measured, and removed.** This followed the user's
steer that a stack is fine but the first levels are flattened, the file deciding how many.

- The evaluator's words were lowered once per static depth `d` in `0..=K`, the return points
  below K in SSA variables, past K on the return stack. Each `EXPR` word was a function of its
  own, called from every depth.
- K came from the files: `expand::if_depth` counted the evaluator's calls on each `#if` line's
  tokens, rounded up to a power of two, at most 16, and `Compiled` compiled each K once.
- **Removed.** A build at depth K serves every TU, since a TU goes only as deep as its `#if`s
  do, so compiling per depth bought nothing that one build did not. And one build at depth 16
  was slower than one at 0 on both compilers, every corpus TU on each (M1 Max, quiet run):
  Cranelift 1,388 M cycles against 1,249 M (1.11×), rustc 861 M against 515 M (1.67×). The
  copies cost more in code size than the return stack's loads and tables saved. So the words
  are compiled once, the evaluator's calls on the return stack (`native_cpu_pp::compile`).

**Large words outlined (4d).** The run's function was 198–248 KB of machine code, larger than an
M1 core's instruction cache. `BUILTIN`'s three modes each held the whole of `builtin_value`: 19k
of its 47k CLIF instructions. `GLUE`, `PRAGMA`, `LINEDIR`, `DEFINED` and `EXPR`'s primary held
9k more. Every token passes through the run's function, but these words run rarely.

- Each word is now lowered alone first. One over 512 CLIF instructions (`OUTLINE`) becomes a
  function of its own, called by `call_word`. Its result is 0 for the next word, 1 for the one
  after, or a refusal.
- The run's function is now 56–68 KB. The shape test refuses any function over 128 KB, and
  reports each function's bytes and each word's instructions.

**Tests.**

- `the_compiled_words_are_the_interpreters`: the standard's examples and the TUs, in C and C++,
  each token's kind, flags and spelling and the expansions.
- `the_compiled_ifs_have_the_interpreters_values`: `#if`'s primaries and operators value by
  value, and deep `#if`s.
- `the_compiled_words_have_their_shape`.
- The corpus, ignored and run on the farm: 3,060 TUs the same, 75 refused by both (the token
  grammar's refusals, Q35), 0 differ.

**Time.** Over the corpus's 3,060 TUs, on an M1 Max in a quiet run, profiling build. The words
are timed alone: the TUs' files are lexed ahead, as stage 1 does once per file content. The lean
pass's time includes its lexing.

| Run | Instructions | Cycles |
|---|---|---|
| The words' interpreter | 5,124 M | 1,150–1,190 M |
| The compiled words | 2,290 M | 1,290 M |
| The lean pass (lexing too) | 8,040 M | 1,900–2,000 M |

- The compiled words retire 45% of the interpreter's instructions, at 1.8 instructions a cycle
  against its 4.4. Their cycles are 1.09× the interpreter's.
- By quarters of the TUs, the half that expand most run at 0.92×. The TUs with least expansion
  run at 1.28×, and the smallest at 1.48×.
- The steps that moved it: outlining the large words (1,410 M to 1,294 M cycles) and the masked
  byte words (to 1,290 M).
- Tried, measured and not kept:
  - A reservation per TU sized to its own work, not to the bounds: no change.
  - The run-invariant fields' loads marked `readonly`: no change. A word's block is a merge of
    many paths, so no load dominates the next word's.
  - `T` and `D` as SSA variables, spilled around calls (the plan's form), with a guard that no
    access reaches them in memory: 1.13×, and larger code. The patch is kept out of the tree.

So the cost is not in the instructions, which halved. Something stalls the compiled words that
does not stall the interpreter. The counters that would name it (branch misses, cache misses)
need a profiler run, which is open.

**Built since:**
- **The ahead-of-time Rust of the same IR.** `native_cpu_pp::rust_source` passes the JIT's
  functions through `clif_rust`, and `rerail-pp-aot` compiles them with rustc behind its
  `generated` feature. It is built without debug info: LLVM's debug info overflowed rustc's
  stack on the nested blocks. It equals the interpreter on the examples and TUs (90 runs). On
  the corpus at depth 0 it takes 533–556 M cycles against Cranelift's 1,240–1,258 M. So the
  stall above is Cranelift's code, not the words' IR. The plan that follows is note t44.
- **`_Pragma`'s text lexed by the token grammar,** as the lean pass lexes it: as a file of its
  own. The text is a named entry of the file on top, of a kind of its own (`Named::pragma`).
  Where the host has not lexed it yet, it is wanted, and the TU runs again. `#pragma` and
  `_Pragma` share `pragma_of`, and the byte scan (`pragma_words`) is gone.
- **One program for C and C++.** C's program had been the text with its five `; C++ only` lines
  left out, so every build was made twice. Those lines were table data, not words: three
  punctuators, the literal suffix paste, and `#if`'s alternative tokens and `true`. Now
  `ppop sN cpp` marks C++'s punctuators (`PpTables::cpp_ops`), `Env::cpp` gives the run's
  dialect, and the compiled words test it (`Ctx::cpp`) where the tables are C++'s. The example
  "pastes by dialect" and `the_dialects_examples_differ` check that the lean pass tells the
  dialects apart there, so words equal to it in both read the flag.
- **One build.** With the dialect a run's and no depth (4c, above), the words compile once:
  one JIT compile a process, one ahead-of-time build (`rerail_pp_aot::BUILD`, 3.4 MB of Rust).
  On the corpus (3,060 TUs, 0 differ): the interpreter 1,098 M cycles, Cranelift 1,235 M,
  rustc 487 M, the lean pass 1,876 M.

- **The stall found (Q36).** Cranelift's Arm jump tables carry a speculation barrier (`csdb`) on
  every dispatch, and every `RETURN` dispatched through one. Cranelift's code has its jump
  tables as trees of compares (`native_cpu_codegen::tables_as_compares`, every backend's, Q38),
  and the shape test refuses a `csdb` in the words' code; rustc's keeps the tables. With two smaller
  changes (extending loads, branches on each test of a combination), Cranelift's code went
  from 1,235 M cycles to 550 M: 0.50× the interpreter's, 1.14× rustc's (t44).
- **rustc's build is the CPU path** (the user's choice, t44 step 5). `rerail-c-directives`'
  `generated` feature is on by default, and `expand_tu` runs `expand::native`: the build
  where it was made from the words this process has (`Build::words`, `expand.rir`'s
  fingerprint, checked once), else Cranelift's code, compiled once a thread. Its cost is a
  rustc compile of 3.4 MB of generated Rust when the words or the compiler change.

## Open

- **Header units:** a header expanded once per entry macro state (0327, open).
- **Hash collisions:** a key match compares bytes, so a collision costs time, never a wrong
  answer.
