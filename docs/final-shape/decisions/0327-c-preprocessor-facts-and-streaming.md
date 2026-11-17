# 0327: A C and C++ preprocessor: directives recorded as facts, and applied while streaming

Status: decided (user, 2026-11-16): "a C preprocessor, in two modes: a) recording the directives as
facts and b) applying, streaming mode. On both CPU and Metal. We will need it for some of the
header/library analyses". Building. Numbered past the highest number in any worktree (0326).

## Intro

A C or C++ file is not its own program. What it means depends on the directives it runs before
the compiler sees it: the files it includes, the macros it defines, and the branches of `#if` that a
configuration keeps. An analysis of headers and libraries has to ask two kinds of question.

- **What does a file say, whatever the configuration?** Which headers it names, which macros it
  defines, and how its conditionals nest. This is **recording** (mode a): every directive of a file,
  as facts, read from the file alone.
- **What does a translation unit become under one configuration?** The tokens that survive, where
  each came from, which macros expanded and which files were entered. This is **applying** (mode b):
  the translation unit read in one forward pass, its output streamed as it is made.

Both modes run as native steps inside the engine's fixpoint, like its other reads of the world
(listing a directory, reading a file, loading a document). Their configuration is facts that rules
make and change. The C and C++ frontends read mode (b)'s tokens directly, with the same lexer.

## Decided with the user

| question | answer |
|---|---|
| What applying produces | All of these, as each mode asks: a token tape with origins, which the C and C++ frontends read directly (one shared lexer); `-E`-style text; facts. |
| Where the configuration comes from | Facts, which rules can change. |
| How it is built | Lean first: a Rust crate on the CPU, proved against `clang -E`. Then compiled in the transducer, on the interpreter, Cranelift and Metal (page 12 of `docs/parser-generators`). |
| Which dialects | All of them: C89 to C23 and C++98 to C++26, with GNU and clang extensions. |

## The two modes as requests answered in the fixpoint

The engine already answers requests inside its fixpoint (0065 §2a). A rule that reads `dir_entry(d,
…)` with `d` bound asks for `d` to be listed; the runner lists it, and the rows come back as the next
wave's (`logic/plan/corpus/world.rs`, 0145). A kept result is two rules: a key from what decides it,
and the result from the key by the native step (0104). The preprocessor takes the same shape.

- **Mode (a).** The request is `c_pp_file_req(path)`, and its answer is the file's directive rows.
  The key is the file's content (`file_digest`), so an unchanged file is not read again, and an
  edited one retracts only its own rows (ledger row 14).
- **Mode (b).** The request is `c_pp_tu_req(tu, cfg)`, and its answer is the translation unit's
  outputs. The key is a rule joining the unit, its configuration rows and the digests of the files
  its include closure reaches.

**Include resolution is rules** (`c/includes.rr`), as Python's import resolution is
(`python/modules.rr`). The rules cover:
- quote and angle-bracket search orders, `#include_next`;
- `-iquote`, `-I`, `-isystem` and `-idirafter`, and framework directories;
- all of them read through `dir_entry`.

The closure starts from mode (a)'s rows, taking every branch. That over-approximates the files a
unit reads, so the key names every file the result may depend on. An `#include` whose operand is a
macro can only be known while applying. Mode (b) reports it as `c_pp_include_wanted(tu, spelled)`,
the rules resolve it, and the key grows by it. That is phase 1's import fixpoint (0083, 0094) for C.

## Configuration is facts (`c/env.rr`)

| relation | what it holds |
|---|---|
| `c_tu(tu, lang, std)` | a translation unit, its language (`c`, `c++`, `objective-c`) and standard |
| `c_predefine(cfg, name, body)`, `c_config_undef(cfg, name)` | the predefined macros, `-D` and `-U`, in order (`c_config_undef`: mode (a) has `c_undef`) |
| `c_include_dir(cfg, order, dir, kind)` | the search path, by kind (`quote`, `angle`, `system`, `after`, `framework`) |
| `c_feature(cfg, kind, name, value)` | what `__has_attribute`, `__has_builtin`, `__has_feature`, `__has_extension`, `__has_cpp_attribute`, `__has_c_attribute` and `__has_warning` answer |
| `c_fixed(cfg, name, value)` | `__DATE__`, `__TIME__` and `__TIMESTAMP__`, fixed so that a run is deterministic |

Where the rows come from, every source through rules, so a rule can add to them or override them:
- the run's settings (`env("c.…", v)`);
- `compile_commands.json`, loaded as a JSON document and read from its tape. An entry's
  `arguments` are read as they are; its `command` is split by a new `@shell_words` functor;
- a compiler profile: `clang -dM -E` and `clang -v -E` output, written once by a dev script
  (`scripts/dev/c-profile`) and read with `file_line`.

A configuration is a key, not a setting of the engine. Two configurations of one unit are two
requests with their own rows.

## The relations

**Mode (a), per file:**

| relation | a row |
|---|---|
| `c_directive(path, i, kind, start, end, line)` | directive `i` of the file, its kind (`define`, `include`, `if`, …, `null`, `unknown`), its bytes and line |
| `c_define(path, i, name, fnlike, params, variadic, body_start, body_end)` | a macro's definition |
| `c_undef(path, i, name)` | |
| `c_include(path, i, kind, form, spelled)` | `kind`: `include`, `include_next`, `import`, `embed`; `form`: `quote`, `angle`, `macro` (tokens that make a header name once expanded) |
| `c_cond(path, i, kind, expr_start, expr_end)` | `if`, `ifdef`, `ifndef`, `elif`, `elifdef`, `elifndef`, `else`, `endif` |
| `c_cond_next(path, i, j)`, `c_cond_parent(path, i, p)` | the chain of a conditional's groups, and the group a directive sits in |
| `c_text(path, i, text)` | the text of `#error`, `#warning`, `#ident`, `#sccs`, `#assert`, `#unassert` and an unknown directive |
| `c_pragma(path, i, first, second, text)`, `c_line(path, i, n, file)` | a pragma's first two words (`once`; `GCC system_header`; `push_macro`), so rules join on words, not text (ledger row 18) |
| `c_guard(path, i, name)` | the file's include guard, by clang's multiple-include rule: `#ifndef X` or `#if !defined X` first, its `#endif` last, no token outside |
| `c_pp_problem(path, i, kind)` | an unmatched `#elif`/`#else`/`#endif`, one after `#else`, an unclosed `#if`, a malformed operand, an unclosed comment |

**Mode (b), per translation unit and configuration:**

| relation | a row |
|---|---|
| `c_pp_included(tu, from, i, to, depth)` | directive `i` of `from` entered `to` |
| `c_pp_active(tu, path, start, end)` | a byte range of a file that survives the conditionals |
| `c_pp_cond_taken(tu, path, i, branch)` | which group of a conditional was taken |
| `c_pp_macro_def_used(tu, name, path, i)` | a definition that was expanded |
| `c_pp_expansion(tu, id, name, path, off, parent)` | an expansion, where it was written, and the expansion it came from |
| `c_pp_diag(tu, path, off, kind, text)` | `#error`, `#warning`, and what the preprocessor refused |
| `c_pp_tape(tu, t)`, `c_pp_text(tu, x)` | the token tape and the `-E` text, stored objects named by a fact's column (0104, rule 3) |

An answer produces only the relations the program reads (ledger row 13). Expansions in system
headers run to millions of rows, and a program that reads only `c_pp_included` makes none of them.
A token's origin is not a row: the tape holds origins as run-length segments (an expanded range → a
file and offset, or an expansion id), read as `c_pp_origin` rows only where asked. A per-file cost
must not become a per-token one (history index, lesson 6).

## The code

- **One lexer.** `rerail-c-lean`'s lexer gains a preprocessing-token mode for the preprocessor's
  reading:
  - pp-numbers, header names, `#` and `##`, the digraphs `%:` and `%:%:`;
  - trigraphs where the standard reads them;
  - no keyword codes.

  The parser's mode is unchanged. Both modes write the same `Tok`.
- **`rerail-c-pp`** (a new crate, `native/crates/c_pp`), the lean preprocessor:
  - `directives.rs`, mode (a). It finds logical lines by stage 1's masks and the micro-lexers
    (`rerail_simd8::blocks`, `rerail_simd8::micro`), names directives by `keyword_switch!`, and
    matches conditionals by nesting.
  - `macros.rs`, the macro table and its expansion:
    - hide sets, as clang paints tokens;
    - function-like arguments, `#`, `##`, `__VA_ARGS__`, `__VA_OPT__`;
    - GNU named variadics and `, ## __VA_ARGS__`;
    - `push_macro` and `pop_macro`, `#pragma poison`;
    - the built-in macros (`__LINE__`, `__FILE__`, `__COUNTER__`, `__INCLUDE_LEVEL__`,
      `__FILE_NAME__`, `__BASE_FILE__`).
  - `expr.rs`, the `#if` expressions:
    - `intmax_t` and `uintmax_t` arithmetic with the usual conversions;
    - short-circuit evaluation, with no error from an operand not evaluated;
    - `defined`, the `__has_*` family (from `c_feature`), `__has_include` and
      `__has_include_next`, `__has_embed`.
  - `stream.rs`, the forward pass:
    - the include and conditional stacks;
    - `#pragma once`, `#import`, `#line`, `#embed`;
    - `#error` and `#warning` recorded, `#ident`, `#assert`, `_Pragma`;
    - skipped groups read only for their directives.
  - `sink.rs`, the outputs, any of them at once:
    - the token sink: `Tok`s over an expanded buffer, with the origin segments;
    - the text sink: `-E` text with line markers;
    - the fact sink: rows.
- **The frontends.** The lean C and C++ parser takes its tokens through a source:
  - today's `Lexer::run` over a file's bytes;
  - or the preprocessor's token sink.

  So a preprocessed unit is parsed with no second lexing. Its tape is a C or C++ document whose text
  is the expanded buffer.
- **The engine.**
  - A language-generic registry of answerers: the core defines it, and C registers its two in
    `install_languages` (language-generic interfaces, as the crate split has them).
  - The registry is wired into `world.rs` beside `dir_entry`, `file_line` and `load`.
  - The answerer is called once per request, never per row (ledger row 17).
- **The rules:**
  - `docs/rule-syntax/lib/c/env.rr`, `includes.rr` and `preprocess.rr`;
  - C's include closure in phase 1 (`typing/programs/sources.rr`), beside Python's imports.

## Correctness

- **The standard's examples:**
  - C17 §6.10.3.3 to §6.10.3.5: the `hash_hash` and `xstr` examples, and the rescanning examples 3
    to 7;
  - C++'s [cpp.subst] examples for `__VA_OPT__`.
- **Corner cases:**
  - `#if`: unsigned promotion, division by zero in an operand not evaluated, `defined` made by a
    macro;
  - skipped groups holding invalid directives;
  - `#include_next`, `#pragma once`, `push_macro`;
  - GNU's comma elision.
- **`clang -E` as the oracle,** over the C and C++ corpora (`~/orca/rerail-shared/lean-corpus`),
  under one compiler profile. Both outputs are read by our lexer, and their token spellings and
  lines are compared. Each difference is fixed or named.
- **Mode (a) tests:**
  - against a byte-at-a-time reading of the phase-2 text (directives' starts, kinds, lines), over
    random texts of directives, continuations, trigraphs, comments and strings, and over every
    file of the C and C++ corpora (3,135 files, 58,234 directives);
  - operands, chains, nesting and guards by unit tests.

  Where a reading was in doubt, clang's is the one kept. A `#` after a comment that spans lines
  starts a directive when its line started before the comment (`/*\n*/ #define W`), and not when a
  token came first (`x /*\n*/ #define Z` is text); a line end inside a comment ends no directive.
  tree-sitter's preproc nodes were not used as the oracle: tree-sitter reads a directive only where
  its grammar can, and refuses most of what the preprocessor must still read.
- **Measured (lean, mode b):** every file of both corpora, against `clang -E -P` (Apple clang 21,
  `gnu17` and `gnu++20`), under clang's own profile. The profile is its predefined macros, its
  search path, and its answers to every `__has_*` test the pass asked (probed). A project header
  not here is an empty stub, after every other directory, for both. `__DATE__` and `__TIME__` are
  the epoch's on both sides.

  | corpus | files | same tokens | differ |
  |---|---:|---:|---:|
  | C | 1,574 | 1,574 | 0 |
  | C++ | 1,561 | 1,561 | 0 |

  The run is `c_pp/examples/pp_oracle.rs`. Where the two read differently, clang's reading was the
  one kept:
  - `__LINE__` in a call over several lines is the line of the call's `)`;
  - a call with the wrong count of arguments is its name, its arguments dropped;
  - `__has_include` and `__has_embed` outside `#if` are identifiers;
  - a UTF-8 byte-order mark at a file's start is passed;
  - `__building_module` is one of the built-ins.

  Not compared: clang's error recovery. Where `"../x/y.h"` is missing, clang includes the
  `x/y.h` it suggests.
- **The frontends (built):**
  - `rerail_c_pp::tokens::TokenSink` writes a unit's expanded text and the lean parser's tokens
    over it;
  - `rerail_c_lean::tape_of_tokens` parses those tokens with no lexing.

  Both corpora (3,135 files) give the same tape from the tokens as from lexing the text. A
  pp-number is split where the parser's lexer ends a number, so `100ms` is a number and a name.
  Digraphs are written in their plain spelling. Origins are run-length segments: a segment holds
  while the text follows its file byte for byte, or one expansion.
- **Mode (a) in the engine (built).**
  - A language registers answers to a file's request, `language::FileAnswerer`: a request
    relation, the relations it gives, and the rows for a file's bytes. C's is
    `language/adapter/c_pp.rs`, registered in `install_languages`.
  - `rr::WORLD` lists the file system's answers (`FILE_SYSTEM`) and then each registered one,
    one row a relation. Lowering, the cone's rounds, the journal's carry and the consulted
    record treat them alike. A file a language's answer reads is consulted by its digest, as a
    file's lines are.
  - The answer gives only the relations the program declares (`world::answers`).
  - The rules: `docs/rule-syntax/lib/c/preprocess.rr`. Test:
    `c_directives_are_answered_by_file`. A header no document names is asked for in the next
    wave, once an include names it.
  - **Fixed on the way.** A cone that loads nothing left the exports its rules read unbound.
    A request keyed by a document's path failed on missing statistics, in any language. Such a
    cone now binds the corpus's rows of them (test
    `a_request_keyed_by_an_export_needs_no_load`).
- **The compiled forms** are held to the lean preprocessor: the same rows and the same tokens, on
  every backend.

## The compiled forms (later steps)

- **Mode (a)** is a transducer program:
  - tier 1, a block program: logical lines, a line's first non-blank `#`, `\`-newline
    continuations, and comment and string regions by page 36's region walk;
  - tier 2, the stream words: a directive's kind and operands;
  - the interpreter, Cranelift and Metal, as `.rr`'s and JSON's grammars run.
- **Mode (b)** needs what tier 2 does not have:
  - a macro table;
  - token buffers that replay a macro's body;
  - argument frames;
  - the conditional and include stacks.

  These primitives are decided in 0328 before they are built. On Metal, a unit is one thread's
  work over a bounded work queue.

## Prior work consulted

- **Ledger row 14** (each file owns its rows). **Agrees.** Mode (a)'s rows are a file's, keyed by its
  content. Mode (b)'s are a unit's, keyed by the files it reads.
- **Ledger row 13** (extract on demand). **Agrees.** An answer makes only the relations the program
  reads.
- **Ledger row 17** (no per-row host callbacks). **Agrees.** The native step answers a request
  whole.
- **History index, lesson 6** (a per-file cost turned into a per-row cost). **Agrees.** Origins are
  segments in the tape, not rows.
- **The spike** built no preprocessor, and its linter survey needed no import resolution (design
  index, "Linter needs"). Nothing to agree or depart from there.
- **Decisions:**
  - 0065 §2a, 0104 and 0145: requests answered inside the fixpoint, and keys that are rules;
  - 0083 and 0094: phase 1's import fixpoint;
  - 0307 and 0318: the lean C and C++ parser and its lexer;
  - page 12 of `docs/parser-generators`: the compiled forms are side programs of the transducer's
    `Program`, never a module beside it.

## Open

- **Header units.** A header could be preprocessed once per entry macro state and shared by every
  unit that enters it in that state, as clang's modules are. To be measured first.
- **Expansion facts at system-header scale.** They are produced only where read; a budget, as
  `load` has, may be needed.
