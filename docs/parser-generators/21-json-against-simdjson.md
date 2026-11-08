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
| pass 1 per 64 bytes | about 172 instructions, measured | 790 instructions, measured | 960 |
| pass 1 body, in Cranelift IR | (hand-written) | 225 on an ASCII block, 508 on others; 490 more where a number is, 156 where a `\u` is | 293 and 561; comments 59 more |
| pass 1 branches | the loop, the flattening, a UTF-8 fast path | the loop, the flattening, an ASCII test, one per gated region | the same |
| pass 1 writes | structural positions | token starts and last bytes, fallback totals | the same |
| pass 1 machine code | (hand-written) | 4,263 instructions | 5,007 |
| pass 2 runs | hand-written C++, a state machine with a jump per state | 429 generated words, one function | 564 words |
| pass 2 machine code | (hand-written) | 4,914 instructions | 6,458 |
| pass 2 loops | one per token; numbers parsed in scalar code | one per token | the same |
| numbers | converted to doubles and integers, in pass 2 | checked against the grammar, in pass 1, as masks, on blocks that hold one | the same |
| output | simdjson's tape | a node table: kind, field, start, end, size | the same, with comments |

tree-sitter-json is a different shape. It is a table-driven LR parser with a lexer that steps
one byte at a time. It builds a tree of allocated nodes, and it can recover from errors.

Pass 1's 205 operations, by kind: 44 `and`, 29 `and not`, 35 `or`, 1 `xor`, 48 `prev`,
23 byte classes, 13 single-byte tests, 4 `fill`, 1 `escaped`, 1 `toggle`, 5 `within`, 1
constant. 44 of them check numbers (doc 20 §3.4). Of the classes and tests, 13 check
UTF-8.

On Cranelift (doc 20 §6):

- The values that depend only on bytes stay in 16-byte vectors until something else reads
  them. Of the 48 `prev`s, 18 are byte shifts there.
- In full blocks, 7 more are constants: the byte-order mark's chain and the check for UTF-8
  cut short at the end.
- An all-ASCII block runs a second body, with the UTF-8 values folded away.
- The number check and the `\u` check are gated regions. Each runs behind one branch, only
  on a block where its gate has a bit or its carried bits came in nonzero. Step 14's string
  files pay for neither.

Pass 2's 429 words: 17 `TOKEN`, 21 `SWITCH`, 85 `SPAN`, 48 `WORD`, 85 `LEAF`, 23 `OPEN`,
12 `CLOSE`, 28 `NODE_SUPER`, 9 `NODE_FIELD`, 8 `PUSH`, 2 `RETURN`, 4 `EMPTY`, 85 `GOTO`,
2 `ACCEPT`.

## 2. Speed

simdjson's 17 benchmark files, 15.5 MB, on an Apple M-series core. Each GB/s figure is the
median CPU time of 20 runs, ours and simdjson's taken one after the other at a load average
of about 60. Instructions and cycles were counted at a load of 13 to 30 for ours and
simdjson's, and earlier, at about 70, for tree-sitter's. Treat the figures as approximate;
the instruction counts do not depend on the load.

| | GB/s, whole corpus | instructions per byte | cycles per byte |
|---|---|---|---|
| simdjson, stage 1 | 7.81 | 2.7 | 0.66 |
| simdjson, whole parse | 2.40 | 10.0 | 1.93 |
| ours 14, pass 1 | 2.24 | 12.4 | 1.86 |
| ours 14, pass 2 | 3.78 | 9.1 | 1.11 |
| ours 14, both | 1.41 | 21.4 | 2.97 |
| ours 15, pass 1 | 1.97 | 15.0 | 2.10 |
| ours 15, both | 1.29 | 24.2 | 3.12 |
| tree-sitter-json | 0.038 | 550 | 121 |

In ratios:

- simdjson's whole parse takes about 0.6 of our step 14's time (0.59; by cycles, 0.65).
- Our pass 1 takes about 3.5 times stage 1's time (by cycles 2.8, by instructions 4.6).
  Our pass 2 is a little faster than simdjson's stage 2 (1.11 cycles a byte against 1.27),
  but it does less: no number conversion.
- tree-sitter-json takes 37 times ours, and 63 times simdjson's.
- Comments cost step 15 about 9% more than step 14.

The work is not quite the same:

- simdjson converts every number; we only check its syntax.
- We write a node per token, in tree-sitter-json's form (every `[` and `,` is a node).
  simdjson's tape has an entry per value.

Pass 1 by the kind of file, in instructions per byte for step 14:

| file | pass 1 |
|---|---|
| strings only (`["abc…", …]`, 16 MB) | 7.3 |
| numbers only (`[12345.678e-9, …]`, 14 MB) | 14.3 |
| simdjson's corpus | 12.4 |

On strings the gated regions are skipped. On numbers, every block runs the number check:
about 490 more instructions a block, 7.7 a byte. simdjson's stage 1 does not look at
numbers there, and its stage 2 parses each one in scalar code.

Before the gated regions, pass 1 took 16.2 instructions a byte over the corpus, and 15.7
on strings only. Before the number check moved into pass 1, the values were kept in vectors
and the ASCII test was added, step 14 ran at 0.38 GB/s, under a similar load.

Per file, in GB/s:

| file | KB | simdjson stage 1 | simdjson all | 14 pass 1 | 14 pass 2 | 14 both | 15 both | tree-sitter |
|---|---|---|---|---|---|---|---|---|
| apache_builds | 127 | 10.99 | 5.02 | 2.42 | 3.60 | 1.45 | 1.36 | 0.048 |
| canada | 2,251 | 6.84 | 1.50 | 2.07 | 4.21 | 1.39 | 1.27 | 0.037 |
| citm_catalog | 1,727 | 10.12 | 4.76 | 2.27 | 5.64 | 1.62 | 1.43 | 0.066 |
| github_events | 65 | 10.93 | 4.80 | 2.77 | 4.52 | 1.72 | 1.53 | 0.056 |
| gsoc-2018 | 3,328 | 9.38 | 4.76 | 4.14 | 11.74 | 3.06 | 2.48 | 0.099 |
| marine_ik | 2,983 | 5.71 | 1.55 | 1.76 | 2.24 | 0.99 | 0.97 | 0.022 |
| mesh | 724 | 6.75 | 1.49 | 2.06 | 3.01 | 1.22 | 0.89 | 0.020 |
| numbers | 150 | 7.54 | 1.93 | 1.36 | 3.99 | 1.02 | 1.39 | 0.033 |
| twitter | 632 | 9.28 | 4.42 | 2.32 | 4.10 | 1.48 | 1.26 | 0.050 |
| twitterescaped | 562 | 8.33 | 2.88 | 1.89 | 1.82 | 0.93 | 0.88 | 0.024 |

On files of numbers (canada, marine_ik, mesh, numbers) simdjson's whole parse takes 0.5 to
0.9 of our time: it converts each number there, and we check each block that holds one. On
files of strings (gsoc-2018, citm_catalog) it takes 0.35 to 0.65 of ours. Pass 2 is slowest on
twitterescaped and marine_ik, which have the most tokens per byte. Single small files vary
by 10% or more from run to run.

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

**Pass 1: about 790 instructions a block over the corpus.** simdjson's stage 1 is about 172.
What is left, in the compiler and not in a language:

- **The number check, where there are numbers.** It is gated, so a block without a number
  skips it. A block with one runs all of it: about 490 instructions, 11 masks and 12 `prev`s
  of masks, for every byte of the block. A check per number, as simdjson's stage 2 does, would
  need a loop over numbers. Tier 2 has one, over tokens, but the check was moved out of it
  to be masks.
- **The rest of the body.** On an ASCII block, 225 instructions in Cranelift's IR, before
  code generation: 6 masks (quote, backslash, `u` and three byte classes), the string and
  escape carries, and the token starts. simdjson makes 4 to 5 masks.
- **Carried `prev`s of masks.** 23 are left outside full-block constants: 4 outside the
  regions, 19 inside them. Each is a mask shifted with its field in the carried state, about
  4 to 7 instructions. A `prev` of a value that is not byte-local has no byte form.
- **Blocks with UTF-8.** They make 7 masks outside the regions, against 6 on an ASCII block;
  the 13 UTF-8 tests stay in vectors but cost about 280 instructions more than the ASCII body.
- **The sinks and the flattening:** about 70 instructions a block, and about 45 for each
  eight positions written, as in simdjson.

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
