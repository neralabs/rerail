# 0327: A C and C++ preprocessor: directives recorded as facts, and applied while streaming

Status: partly built (audited 2026-11-23). Left: c_pp_tape/c_pp_text as stored objects; header units; system headers
as roots; compiled mode (b) moved to 0436/0448/0482. Decided (user, 2026-11-16): "a C preprocessor, in two modes: a)
recording the directives as facts and b) applying, streaming mode. On both CPU and Metal. We will need it for some of
the header/library analyses". Building. Numbered past the highest number in any worktree (0326).

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
- **Mode (b).** The request is `c_pp_tu_req(tu, spec)`, and its answer is the translation unit's
  outputs. `spec` is the unit's configuration as one text, which rules make from the configuration's
  rows (`c_cfg_spec`), so a changed row is another request. The answer names every file the pass
  looked at, found or not (`c_pp_read`), and the journal keeps it while none of them changed: an
  edit to a header asks again for the units that read it, and for no other. (A key joining the
  digests of the include closure was the first design. It would recurse through an aggregate: a
  computed include found while applying grows the closure, which changes the key.)

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
| `c_tu(tu, cfg)` | a translation unit and the configuration it is preprocessed under |
| `c_lang(cfg, lang, std)` | the language (`c`, `c++`, `objective-c`, `objective-c++`) and standard as a driver spells it (`gnu17`, `c++2c`) |
| `c_predefine(cfg, order, name, body)`, `c_config_undef(cfg, order, name)` | the predefined macros, `-D` and `-U`, in one order (`c_config_undef`: mode (a) has `c_undef`) |
| `c_include_dir(cfg, order, dir, kind)` | the search path, by kind (`quote`, `angle`, `system`, `framework`, `after`) |
| `c_feature(cfg, test, name, value)` | what `__has_attribute`, `__has_builtin`, `__has_feature`, `__has_extension`, `__has_cpp_attribute`, `__has_c_attribute` and `__has_warning` answer |
| `c_fixed(cfg, name, value)` | `__DATE__`, `__TIME__` and `__TIMESTAMP__`, fixed so that a run is deterministic |

Where the rows come from, every source through rules, so a rule can add to them or override them:
- the run's settings (`env("c.…", v)`);
- `compile_commands.json`, answered by file as mode (a)'s directives are (`c_cc_file_req(path)`:
  `c_compile_command`, `c_compile_arg`). An entry's `arguments` are read as they are; its `command`
  is split as a shell splits it (`shell_words`; `@shell_unquote` gives rules a word's value). A load
  is in the run's grammar, so a C run cannot load the file as a JSON document, as first planned;
- a compiler profile: `clang -dM -E` and `clang -v -E` output, written once by a dev script
  (`scripts/dev/c-profile`) and read with `file_line`.

`lib/c/compile-commands.rr` makes each entry a unit under a configuration of its own (`-D`, `-U`,
`-I`, `-iquote`, `-isystem`, `-idirafter`, `-F`, `-x`, `-std=`, joined or apart), the profile its
first macros and its compiler-own search path (settings `c.compile_commands`, `c.profile.<lang>`).

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

**Mode (b), per translation unit and configuration** (each row led by `tu` and `spec`; `at` is a
directive's `#` or a call's name, in its file's bytes):

| relation | a row |
|---|---|
| `c_pp_read(tu, spec, path)` | a file the pass looked at, found or not (always made: the journal keeps the answer by it) |
| `c_pp_included(tu, spec, from, at, to, depth)` | the directive at `at` of `from` entered `to` |
| `c_pp_include_wanted(tu, spec, from, at, spelled, form)` | a header not found, as spelled once expanded |
| `c_pp_active(tu, spec, path, start, end)` | a byte range of a file that survives the conditionals |
| `c_pp_cond_taken(tu, spec, path, at, taken)` | whether a conditional directive's group was taken |
| `c_pp_expansion(tu, spec, id, name, def_path, def_at, path, at, parent)` | an expansion, its macro's definition, where it was called, and the expansion it came from (a definition used is a rule over it) |
| `c_pp_diag(tu, spec, path, at, kind, text)` | `#error`, `#warning`, and what the preprocessor refused |
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
  - A language registers answers to a file's request (`language::Answerer`, then `FileAnswerer`): a request
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
- **Mode (b) in the engine (built).**
  - The answer interface is `language::Answerer`: a request of one column (a path, asked by the
    readers of its relations, as `file_line` is) or of several (made by rules). An answer reads
    files through the runner (`Reader`), under the run's roots, so they are the run's consulted
    record. A request of several columns names the files it read (`read_set`, `c_pp_read`), and
    the cone's rounds keep it from the journal while none of them is named (`cone_rounds`).
  - C's `Units` reads the configuration back from `spec` (`read_spec`), searches as a driver does
    (`rerail_c_pp::files::Search`, every path it looks at recorded) and runs `rerail_c_pp::Pp`
    with the fact sink.
  - The rules: `lib/c/env.rr` (the configuration, `c_cfg_item`, `c_cfg_spec` by `join`) and
    `lib/c/preprocess.rr` (`c_pp_tu_req(tu, s) :- c_tu(tu, c), c_cfg_spec(c, s)`).
  - Tests: `c_units_are_preprocessed_under_their_configurations` (two configurations take other
    branches and enter another header); `a_header_edit_asks_again_only_for_the_units_that_read_it`
    (with the journal on: an edit asks for one unit of three, a file no unit looked at for none, and
    the outputs are a fresh run's).
  - **Fixed on the way**, each in any language:
    - pruning unreached clauses dropped a request made by a rule, which nothing reads by name: an
      answer's relation read now reaches its request (`rr::reached_statements`);
    - the run's constants took every answer relation as given, so a rule reading only a unit's
      answer was folded over no documents, empty: a language's answer is a constant only where its
      request is (`constants::foldable`);
    - the journal's carry compared a relative key with absolute roots, so no relative path was ever
      kept: it is the run root's, as the world reads it;
    - `rr::WORLD` was made once a process, before the languages installed where no load hook runs
      (a test runner), and then answered no language's request: it is made once they are;
    - a request keyed by a `match` group (`dir_entry(d, …)` with `d` from
      `match("(?<d>.*)/[^/]*", p)`) was made by no clause, as the group's equality is written
      later in the lowering: a positive `match` binds its groups (`rr::lower`'s world demand).
  - `lib/c/includes.rr`: the closure as rules, for analyses (`c_reach`, `c_resolved`,
    `c_unresolved`), by the driver's search order over `dir_entry` and mode (a)'s rows, every
    branch taken; `#include_next` from the entry after the includer's. Test
    `c_includes_resolve_by_the_search_order` (the closure is the files mode (b) enters).
  - Configurations from a build: `c_configurations_come_from_compile_commands_and_a_profile`.
  - Phase 1 (`sources.rr`): the closure's headers are loaded as documents in the run's grammar
    and handed on as `library` rows, as Python's imports are; a program that reads
    `sources__library` has them as its documents (test `a_c_runs_included_headers_are_its_documents`).
    A `.h` file is C++'s by its suffix, so a library document is the grammar phase 1 loaded it in,
    not its suffix's (`corpus::run_observed`'s `text_document`). The units: a build's record where
    `c.compile_commands` names one, else each source file that is no header, under a configuration
    `default` (the run's language, GNU's 17, the profile `c.profile.<lang>` where named). The
    configuration relations are `extensible`, so a program adds to them.
  - Not yet: `c_pp_tape` and `c_pp_text` as stored objects; the store's families still give a
    stored document its dialect by suffix (`facts::families::dialect`), which a persisted C run's
    header tape would need to follow.
- **The compiled forms** are held to the lean preprocessor: the same rows and the same tokens, on
  every backend.

## The compiled forms (later steps)

- **Mode (a)** is a Bitlex grammar compiled by the Bitlex compiler, as `.rr`'s (`rr_parser`) and
  Kotlin's (0332, `kotlin_parser`) are: a crate of its own, `c_directives`
  (`rerail-c-directives`), with a ladder of grammar steps (`bitlex_ladder/`) and the last step
  compiled once a process.
  - Tier 1, the block program: logical lines (`\`-newline continuations), a line's first
    non-blank `#` outside comments and strings, the comment and string regions (page 36's walk on
    the CPU, the rounds on Metal). Comments are hidden bytes, never tokens: no directive row
    names one, so the scan needs none of 0332's comment kinds.
  - Tier 2, the stream words: each directive a node of a directive tape (its kind by its word,
    its operands' spans as fields), conditionals nested as brackets.
  - The rows are read from that tape in place (0310's reads), the same relations as the lean
    scan's (`c_directive`, `c_define`, `c_include`, `c_cond`, …).
  - The reference is the lean scan (`rerail_c_pp::directives`). A step refuses what it does not
    read yet, and a refused file goes to the lean scan, so a step never gives a wrong row: the
    crate's corpus test counts the files that read the same, are refused, or differ, and
    "differ" stays 0 (0332's rule).
  - The interpreter, Cranelift and Metal, as `.rr`'s and JSON's grammars run.
- **Mode (b)** needs what tier 2 does not have:
  - a macro table;
  - token buffers that replay a macro's body;
  - argument frames;
  - the conditional and include stacks.

  These primitives are decided in a decision of their own before they are built, numbered past
  the highest in any worktree (0339 on 2026-11-17: the "0328" first planned is another
  branch's). On Metal, a unit is one thread's work over a bounded work queue.

### Mode (a) compiled, built: step 1 (`01-lines.bitlex`)

Each directive's kind and bytes, the rows of `c_directive` but its line and group. The grammar
finds a directive's `#` as the first byte of a logical line that is no blank (comments, splices
and a byte order mark are blanks), gives each directive line's tokens, skips the rest of the
file, and makes a node a directive, retagged by its word (`null`, `line_marker`, `unknown`
otherwise). The tape's rows are read in place (`rerail_c_directives::rows`).

- **Corpus** (3,135 C and C++ files): 3,016 the same as the lean scan, 119 refused, 0 differ; the
  interpreter's rows and Metal's (tier 1 on the device, tier 2 over its buffers) are Cranelift's on
  every file. Refused: carriage returns, raw strings, digit separators, `'` or `"` a line break
  closes (`#error don't`), trigraphs and `%:`.
- **Metal takes a token's last from its rule's mask over the whole file.** A `run` over a class,
  or a `read` of a region, gave a last wherever the class or the region was, the code between
  directives too, so the device's tier 1 sent the file to its fallback. A directive line's tokens
  run over masks of the line (`d_word`, `s_bytes`).
- **A tape past 2-byte columns** (a file's last directive far from its end: a root tail past
  64 KiB) was refused by every compiled grammar: the compiler's `Parser` compiled tier 2 at the
  narrow widths only. It now reads such a file again (`Parser::again`, on the CPU and over
  Metal's buffers): at 4 bytes, then at the narrowest widths its values fit, so the image is the
  writer's byte for byte (work queue Q31). Found with it and fixed: an offset was flagged wide at
  any width (`native_cpu_tape::blocks_from`, `rust_stream`).

### Mode (a) compiled, step 2 (`02-operands.bitlex`): the operands

Each operand is a field's node below its directive, read in place (`rerail_c_directives::operands`):
a `#define`'s `name`, its `params` where a `(` touches the name (each `param`, a `variadic`
`...`, GNU's `name...` as the `...` right after a parameter) and its `body`; an `#undef`'s
`name`; an include's `spelled`, its form by the node (a string, a `header` `<…>` to its first
`>`, or the tokens a macro makes one of; `#embed`'s first token only); a conditional's `expr`;
a `#pragma`'s `text`, with `first` and `second` where they are words; a `#line`'s or a line
marker's `rest`, its `n` where its first token is a number and its `file` where the second is a
string; any other directive's `text` after its word.

- **Corpus**: 3,016 files the same as the lean scan, rows and every operand; 119 refused (none
  new); 0 differ; the interpreter's and Metal's images are Cranelift's on every file.
- **The lean scan ended a directive at its last operand**, not at its line's last token as its
  rows say (`#undef X junk`, an include's tokens after its header, `#embed`'s parameters). It
  now reads the line's remaining tokens; the grammar's node ends there too.
- **The grammar's own ids stay below 0x400**: a tape's kind is 10 bits, and an id past them
  (`0x4000` on, as `.rr`'s parser numbers its own) read as another kind on Cranelift and lost
  its field in the interpreter's image. Unseen in step 1, whose nodes had no fields.

### Mode (a) compiled, step 3 (`03-groups.bitlex`): conditional groups

A group is a node from its `#if`, `#ifdef` or `#ifndef` to its `#endif`, nested as the file
nests them. Token marks on a `#` and its word (`tokens hash "if" | "ifdef" | "ifndef"`, and the
same for a chain's `#elif…`/`#else` and for `#endif`) open a group, go on with its chain, and end
it. A group's own directives are its chain (`c_cond_next`); a directive's parent is the chain's
directive before it, an opener's or a chain directive's the enclosing group's
(`c_cond_parent`); an `#elif…`, `#else` or `#endif` with no group open is a directive of its
level, as the lean scan records it. A group the file does not close is refused (the lean scan's
unterminated `#if`). Every `#` is a `hash` token: a token mark tells kinds by first byte.

- **Corpus**: 3,016 the same (rows with parents, operands, chains), 119 refused, 0 differ, every
  backend's images equal.
- **The compiler, fixed on the way** (the first grammar with token marks and no extras):
  - token marks needed one extra kind (`bitlex_masks`, the verifier); with none, the executors
    took mark bit 0 for the extra's and would skip tokens with it. Each executor (the
    interpreter, Cranelift's tier 2, the Rust AOT's) now takes the extra's bit as an `Option`:
    with none, no record is an extra.
  - Cranelift's tier 1 made no start mask where its token loop walks the starts
    (`walked_starts`), but a mark the loop does not set read its bits from it: every token
    mark's bit was 0, so no group opened. The starts stay made where such a mark reads them
    (`BlockNative::records_made`).

### Mode (a) compiled, step 4 (`04-facts.bitlex`): the engine's answer

`c_pp_file_req` is answered from the compiled grammar's tape where it reads the file, else from
the lean scan (`rerail_c_directives::facts`). Both make one `Facts` value (`Facts::of` a tape's
reading, `Facts::lean`), which the answerer writes as rows: one path for the rows whichever
source.

- **The guard** needs code outside the directives: a `code` leaf where it starts after each
  directive (or the file's start), at most one a directive. A guard is a top level of one group
  and nothing else, its chain its opener and its `#endif`, its opener `#ifndef X`,
  `#if !defined X` or `#if !defined(X)` (the opener's expression's leaves).
- **Problems**: an unmatched `#elif…`/`#else`/`#endif` (a chain directive at the top level), one
  after its chain's `#else`, a `#define` or `#undef` with no name, an include with no header. An
  unterminated group or an unclosed comment is refused, so the lean scan gives those.
- **Lines** from the file's line breaks (`rerail_simd8::micro::find`), not the tape; texts with
  their `\`-newlines removed. A `\`, blanks and a line break (GCC's and clang's splice) is
  refused.
- **Corpus**: `Facts` equal to the lean scan's on all 3,016 files read (119 refused).

### Mode (a) compiled, step 5 (`05-crlf.bitlex`): `\r\n` line ends

A `\r` before a `\n` is a blank and `\` before `\r\n` a splice, as phase 2 reads them; a `\r`
alone is still refused. Corpus: 3,040 the same, 95 refused, 0 differ (the 24 CRLF files now
read). Of the refusals left, raw strings are most (59 files have one); then digit separators,
quotes a line break closes, trigraphs and `%:`.

### Mode (a) compiled, step 6 (`06-raw.bitlex`): raw strings

A raw string with no delimiter (`R"(…)"`, prefixed by `u8`, `u`, `U` or `L` too) is a region
from its `"(` to the first `)"`, line breaks inside it too (a directive's line runs on past
them). One with a delimiter, one open at the file's end, and a splice inside one (phase 2 undoes
it there) are refused. A configuration without raw strings (C, not GNU) has the lean scan read a
file with an `R"`. Corpus: 3,075 the same, 60 refused, 0 differ, on every backend.

- **A compiler fix found here.** Two region kinds' openers on one byte (a raw string's `"` is a
  string's opener too): the walk, the regions' meaning, opens the first-listed kind, but the
  rounds Metal runs opened both. Each kind's rounds opener now excludes the earlier kinds'
  (`bitlex_masks::regions`; `regions_tests`).

### Prior art in the other worktrees (surveyed 2026-11-17)

| Where | What | For mode (a) compiled |
|---|---|---|
| Swift-Kotlin-Scala-support, 0332 | A new language's Bitlex ladder in its own crate, held to its lean parser; refusals to the lean parser; "differ" 0 on the corpus | The crate's shape, its tests and its fall-back |
| the same, 0332's compiler additions | several comment kinds, readers merged at one first byte (`/` for `//` and `/*`), `text("…")` in masks, a part to a `last`, quiet regions | `text("…")` and a part to a `last` for the directive word and its operands; the comment kinds are not needed (comments are hidden) |
| Parser-generators-final-shape, 0323 | A region read in parts (`read str parts { … }`) | an `#include`'s `<…>` and `"…"`, a `#define`'s parameters |
| the same, 0335, 0337, 0338, 0339 | a prefix outside the stream; a check waits for its token; a one-byte string as a toggle (`prefix_xor`); a class as one lookup | C's `"…"` and `'…'` are toggles' shape but two kinds: the regions walk until they are merged |
| final-rerail-shape and others, 0310 | the tape's reads written once in CLIF, for Rust, the JIT and MSL | the rows read from the directive tape on every backend |
| serverplan-quality, 0336 | the stored tape read as stored on the CPU and GPU | the directive tape is read in place, never decoded |

The step 7 build starts on this branch's compiler, which has the regions, the walk and the token
loop the first steps need. The two compiler branches above come in by the roll-up that merges
them, not by a merge here (Swift-Kotlin-Scala-support's language crates collide with this
branch's in 29 files; the grammar is a crate of its own, so it rebases on the compiler they
leave).

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
- **Headers outside the run's roots.** A unit's system headers are read only under a root (the
  world's rule), so a compiler's own include directories need to be roots, or a library root, as
  Python's typeshed is.
- **Expansion facts at system-header scale.** They are produced only where read; a budget, as
  `load` has, may be needed.
