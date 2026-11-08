# Parser generators: JSON in Bitlex against simdjson and tree-sitter

## Intro

Two Bitlex files describe JSON. Each is compiled to a transducer program and run with no Rust
between the bytes and the nodes:

- **`14-json.bitlex`** accepts what simdjson accepts. It writes tree-sitter-json's nodes.
- **`15-jsonc.bitlex`** is the same with `//` and `/* */` comments. It writes tree-sitter-json's
  nodes, comments included.

Both are the last two steps of the ladder (doc 20 §9), after thirteen made-up languages. This
page compares them with simdjson 5.0.5 and tree-sitter-json 0.24.8:

- how each one is built (§1);
- how fast it runs (§2);
- what it accepts (§3);
- where the time goes, and what would close the gap (§4).

Files, tools and how to rerun every number are in §5.

## 1. Shapes

Both our parsers and simdjson run in two passes: one over the bytes, one over the tokens.

| | simdjson | ours, step 14 | ours, step 15 |
|---|---|---|---|
| pass 1 runs | hand-written NEON, per 64 bytes | a generated block program, per 64 bytes | the same |
| pass 1 operations | (hand-written) | 205 operations | 259 operations |
| pass 1 per 64 bytes | about 173 instructions, measured | 864 Cranelift instructions on an ASCII block, 1,186 on others | 1,069 and 1,376 |
| pass 1 branches | the loop, the flattening, a UTF-8 fast path | the loop, the flattening, an ASCII test | the same |
| pass 1 writes | structural positions | token starts and last bytes, fallback totals | the same |
| pass 1 machine code | (hand-written) | 4,296 instructions | 4,896 |
| pass 2 runs | hand-written C++, a state machine with a jump per state | 429 generated words, one function | 564 words |
| pass 2 machine code | (hand-written) | 4,914 instructions | 6,458 |
| pass 2 loops | one per token; numbers parsed in scalar code | one per token | the same |
| numbers | converted to doubles and integers, in pass 2 | checked against the grammar, in pass 1, as masks | the same |
| output | simdjson's tape | a node table: kind, field, start, end, size | the same, with comments |

tree-sitter-json is a different shape. It is a table-driven LR parser with a lexer that steps
one byte at a time. It builds a tree of allocated nodes, and it can recover from errors.

Pass 1's 205 operations, by kind: 43 `and`, 29 `and not`, 36 `or`, 1 `xor`, 47 `prev`,
24 byte classes, 13 single-byte tests, 4 `fill`, 1 `escaped`, 1 `toggle`, 5 `within`, 1
constant. About 60 of them check numbers (doc 20 §3.4). Of the classes and tests, 13 check
UTF-8.

On Cranelift, the values that depend only on bytes stay in 16-byte vectors until something
else reads them (doc 20 §6). So a `prev` of a class is a byte shift, and most UTF-8 values
never become masks. An all-ASCII block runs a second body, with the UTF-8 values folded away.

Pass 2's 429 words: 17 `TOKEN`, 21 `SWITCH`, 85 `SPAN`, 48 `WORD`, 85 `LEAF`, 23 `OPEN`,
12 `CLOSE`, 28 `NODE_SUPER`, 9 `NODE_FIELD`, 8 `PUSH`, 2 `RETURN`, 4 `EMPTY`, 85 `GOTO`,
2 `ACCEPT`.

## 2. Speed

simdjson's 17 benchmark files, 15.5 MB, on an Apple M-series core. Each figure is the median
CPU time of 20 runs or more. The machine was loaded: a load average of about 70 for simdjson's
and tree-sitter's runs, and 75 to 80 for ours. Treat the figures as approximate.

| | GB/s, whole corpus | instructions per byte | cycles per byte |
|---|---|---|---|
| simdjson, stage 1 | 5.23 | 2.7 | 0.72 |
| simdjson, whole parse | 1.64 | 10.1 | 2.05 |
| ours 14, pass 1 | 1.21 | 16.2 | 2.64 |
| ours 14, pass 2 | 2.88 | 9.1 | 1.14 |
| ours 14, both | 0.85 | 25.3 | 3.78 |
| ours 15, pass 1 | 1.07 | 19.3 | 2.88 |
| ours 15, both | 0.78 | 28.5 | 4.00 |
| tree-sitter-json | 0.028 | 550 | 121 |

In ratios:

- simdjson's whole parse takes about half of our step 14's time (0.52; by cycles, 0.54).
- Our pass 1 takes about 4 times stage 1's. Our pass 2 is a little faster than simdjson's
  stage 2 (1.14 cycles a byte against 1.33), but it does less: no number conversion.
- tree-sitter-json takes 30 times ours, and 58 times simdjson's.
- Comments cost step 15 about 9% more than step 14.

The work is not quite the same:

- simdjson converts every number; we only check its syntax.
- We write a node per token, in tree-sitter-json's form (every `[` and `,` is a node).
  simdjson's tape has an entry per value.

Before the last changes (the number check moved into pass 1, values kept in vectors, the
ASCII test), step 14 ran at 0.38 GB/s: pass 1 at 0.98, pass 2 at 0.63.

Per file, in GB/s (simdjson's columns are from the earlier runs):

| file | KB | simdjson stage 1 | simdjson all | 14 pass 1 | 14 pass 2 | 14 both | 15 both | tree-sitter |
|---|---|---|---|---|---|---|---|---|
| apache_builds | 127 | 7.14 | 3.29 | 1.25 | 2.28 | 0.81 | 0.73 | 0.031 |
| canada | 2,251 | 4.73 | 0.99 | 1.19 | 2.97 | 0.85 | 0.79 | 0.026 |
| citm_catalog | 1,727 | 6.92 | 3.23 | 1.44 | 4.02 | 1.06 | 0.83 | 0.047 |
| github_events | 65 | 7.11 | 3.15 | 1.20 | 3.36 | 0.89 | 0.77 | 0.037 |
| gsoc-2018 | 3,328 | 6.52 | 3.50 | 1.22 | 7.66 | 1.05 | 0.99 | 0.074 |
| marine_ik | 2,983 | 3.74 | 1.06 | 1.10 | 1.78 | 0.68 | 0.63 | 0.018 |
| mesh | 724 | 4.63 | 1.03 | 1.13 | 2.27 | 0.76 | 0.74 | 0.015 |
| numbers | 150 | 4.91 | 1.27 | 1.24 | 3.85 | 0.94 | 0.82 | 0.024 |
| twitter | 632 | 6.01 | 2.90 | 1.20 | 3.06 | 0.86 | 0.79 | 0.037 |
| twitterescaped | 562 | 6.17 | 2.07 | 1.23 | 1.43 | 0.66 | 0.58 | 0.018 |

On files of numbers (canada, marine_ik, mesh, numbers) we are now within 0.7 to 0.9 of
simdjson's whole parse: simdjson converts each number there. Pass 2 is slowest on
twitterescaped and marine_ik, which have the most tokens per byte.

## 3. Accuracy

**JSONTestSuite**, 318 files. `y_` files are valid JSON, `n_` files are not, and `i_` files
are left to the implementation. Each cell is the number of files accepted:

| class | files | ours 14 | ours 15 | simdjson | tree-sitter-json |
|---|---|---|---|---|---|
| `y_` valid | 95 | 95 | 95 | 95 | 91 |
| `n_` invalid | 188 | 0 | 3 | 0 | 22 |
| `i_` either | 35 | 12 | 12 | 4 | 30 |

- **Ours 14 and simdjson** agree on every `y_` and `n_` file. They differ on 8 `i_` files, each a
  number too large for a double, such as `1e400`, or an integer past 64 bits. These are valid
  JSON. We accept them; simdjson refuses them, because it converts each number.
- **Ours 15** also accepts the 3 `n_` files that are valid once comments are allowed.
- **tree-sitter-json** refuses 4 valid files, each with an exponent signed `+`, such as `1e+5`.
  Its grammar has no `+` there. It accepts 22 invalid files, among them `1.`, `[1] [2]`, an
  unescaped tab, lone surrogates and a form feed.

On every suite file, our compiled parser gives the same nodes as the Bitlex reference
evaluator, which runs Bitlex without the compiler.

**simdjson's 17 benchmark files.** All four parsers accept every file. On every file our nodes
equal tree-sitter-json's, node for node: kind, field, start, end and size.

**With comments.** We took 99 of these files (the suite's valid ones and the corpus's smaller
ones) and put comments between tokens, where tree-sitter-json's own tree says tokens end. On 475
such files, step 15's nodes equal tree-sitter-json's, each comment in the same place.

One case is refused by design: a comment that holds a quote, such as `// "x`. Step 15 finds the
string state without looking at comments, as simdjson does, so it cannot tell a quote in a
comment from a string's. It sends those files to tree-sitter: 95 of 99 such files in the test
above. A comment holding the other kind's opener (`/* // */`) is sent too. No file without
comments is affected.

## 4. Where the time goes

**Pass 1: about 1,186 instructions for a block with UTF-8, 864 without.** simdjson's stage 1 is
about 173. What is left, in the compiler and not in a language:

- **Carried `prev`s of masks.** Of the 47 `prev`s, a few shift bytes; the rest shift masks:
  the number check's states (one `prev` by one per group of arms), and values brought to a
  later delay. Each costs about 4 to 7 scalar instructions, with its field in the packed
  state.
- **Masks still made.** 26 values become masks on the full body, 19 on the ASCII one, each
  about 10 instructions on NEON. They are read by something that needs a mask: a `fill`,
  `toggle` or `escaped`, a value that is not byte-local, or several `prev`s of one value.
- **The number check.** It is about 60 of the 205 operations, run on every byte. simdjson
  checks only the bytes of numbers, in stage 2. A file without numbers pays for it too.

**Pass 2: no check left.** It runs at 2.9 GB/s over the corpus, 7.7 GB/s on gsoc-2018. Its
time is the token loop: a switch, the arm's nodes, and the jump to the next state, per token.

## 5. Rerunning it

The data is not in the repository:

- simdjson's benchmark files, from `simdjson/simdjson-data`, `jsonexamples/`;
- JSONTestSuite, from `nst/JSONTestSuite`, `test_parsing/`.

The tools:

- `tools/json-compare/build.sh` builds `simdjson_check` against Homebrew's simdjson. It gives
  simdjson's verdict on each file (`verdict`), its timings (`bench`), or a loop for counting
  instructions (`run`).
- `bitlex_json_tests`, in `native/src/code_rule_dsl/transducer/packed/`. Each test runs by name,
  with the variables it needs:
  - `json_suite_accuracy`: `RERAIL_JSON_SUITE`, and `RERAIL_SIMDJSON` for simdjson's column;
  - `json_corpus_accuracy`: `RERAIL_JSON_CORPUS`;
  - `jsonc_comments_equal_tree_sitter`: both directories;
  - `json_bench`: `RERAIL_JSON_CORPUS`, `RERAIL_ITERS`; `RERAIL_FORMS` for other Bitlex files;
  - `json_shapes`: no data.

Instructions and cycles come from `/usr/bin/time -l`. Each parser runs in its own process,
20 times over the corpus; a run that only loads the files is subtracted. For our parsers that
is `json_bench` with `RERAIL_BENCH_RUN` set to `none`, `tier1` or `both`.
