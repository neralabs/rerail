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
| pass 1 operations | (hand-written) | 143 operations | 197 operations |
| pass 1 per 64 bytes | about 173 instructions, measured | 1,085 Cranelift instructions | 1,298 |
| pass 1 branches | the loop, the flattening, a UTF-8 fast path | the loop and the flattening only | the same |
| state carried per block | a few words | 26 words (47 bits) | 42 words (61 bits) |
| pass 1 writes | structural positions | token starts and last bytes, fallback totals | the same |
| pass 2 runs | hand-written C++, a state machine with a jump per state | 433 generated words, one function | 569 words |
| pass 2 machine code | (hand-written) | 7,459 instructions | 10,200 |
| pass 2 loops | one per token; numbers parsed in scalar code | one per token; inside a number's check, one per 64 bytes | the same |
| numbers | converted to doubles and integers | checked against the grammar | the same |
| output | simdjson's tape | a node table: kind, field, start, end, size | the same, with comments |

tree-sitter-json is a different shape. It is a table-driven LR parser with a lexer that steps
one byte at a time. It builds a tree of allocated nodes, and it can recover from errors.

Pass 1's 143 operations, by kind: 32 `and`, 21 `and not`, 26 `or`, 28 `prev`, 17 byte classes,
10 single-byte tests, 1 `escaped`, 1 `toggle`, 5 `within`. Of its 27 classes and tests, 13 check
UTF-8.

Pass 2's 433 words: 17 `TOKEN`, 21 `SWITCH`, 85 `SPAN`, 48 `WORD`, 85 `LEAF`, 23 `OPEN`,
12 `CLOSE`, 28 `NODE_SUPER`, 9 `NODE_FIELD`, 8 `PUSH`, 2 `RETURN`, 4 `CHECK`, 4 `EMPTY`,
85 `GOTO`, 2 `ACCEPT`.

## 2. Speed

simdjson's 17 benchmark files, 15.5 MB, on an Apple M-series core. Each figure is the median
CPU time of 20 runs or more. The machine had a load average of about 70 during the runs, so
treat the figures as approximate.

| | GB/s, whole corpus | instructions per byte | cycles per byte |
|---|---|---|---|
| simdjson, stage 1 | 5.23 | 2.7 | 0.72 |
| simdjson, whole parse | 1.64 | 10.1 | 2.05 |
| ours 14, pass 1 | 0.98 | 15.3 | 2.98 |
| ours 14, pass 2 | 0.63 | 29.5 | 4.23 |
| ours 14, both | 0.38 | 44.9 | 7.21 |
| ours 15, pass 1 | 0.82 | 18.7 | 3.16 |
| ours 15, both | 0.35 | 48.3 | 8.35 |
| tree-sitter-json | 0.028 | 550 | 121 |

In ratios:

- Our step 14 takes 4.3 times simdjson's time. Pass 1 takes 5.3 times stage 1's; pass 2 takes
  3.8 times stage 2's.
- tree-sitter-json takes 14 times ours, and 58 times simdjson's.
- Comments cost step 15 about 8% more than step 14.

The work is not quite the same:

- simdjson converts every number; we only check its syntax.
- We write a node per token, in tree-sitter-json's form (every `[` and `,` is a node).
  simdjson's tape has an entry per value.

Per file, in GB/s:

| file | KB | simdjson stage 1 | simdjson all | 14 pass 1 | 14 pass 2 | 14 both | 15 both | tree-sitter |
|---|---|---|---|---|---|---|---|---|
| apache_builds | 127 | 7.14 | 3.29 | 0.96 | 2.10 | 0.66 | 0.57 | 0.031 |
| canada | 2,251 | 4.73 | 0.99 | 0.99 | 0.48 | 0.32 | 0.31 | 0.026 |
| citm_catalog | 1,727 | 6.92 | 3.23 | 1.05 | 1.70 | 0.65 | 0.56 | 0.047 |
| github_events | 65 | 7.11 | 3.15 | 1.14 | 2.35 | 0.77 | 0.58 | 0.037 |
| gsoc-2018 | 3,328 | 6.52 | 3.50 | 1.01 | 6.70 | 0.88 | 0.75 | 0.074 |
| marine_ik | 2,983 | 3.74 | 1.06 | 0.92 | 0.29 | 0.22 | 0.21 | 0.018 |
| mesh | 724 | 4.63 | 1.03 | 0.95 | 0.25 | 0.20 | 0.19 | 0.015 |
| numbers | 150 | 4.91 | 1.27 | 1.00 | 0.37 | 0.27 | 0.26 | 0.024 |
| twitter | 632 | 6.01 | 2.90 | 1.00 | 1.97 | 0.66 | 0.56 | 0.037 |
| twitterescaped | 562 | 6.17 | 2.07 | 0.91 | 1.03 | 0.48 | 0.43 | 0.018 |

Pass 1 runs at about 1 GB/s on every file: it does the same work for every byte. simdjson's
stage 1 is faster on files that are mostly ASCII, since its UTF-8 check has a fast path.

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

**Pass 1: classes become masks one at a time.** simdjson classifies each 16-byte vector with
two table lookups, and makes only a few 64-bit masks. Our block program makes a 64-bit mask for
each of its 27 classes and tests, then combines masks. On NEON, each mask costs a compare and a
reduction of about ten instructions. The UTF-8 check alone is 13 classes, and its `prev`s are
done on masks. simdjson checks UTF-8 in vector registers, and skips the work when a block is
all ASCII.

What would close it, in the compiler and not in a language:

- lower classes that share a table to one lookup, and keep the byte vectors until a mask is
  needed;
- do `prev` of a class on the byte vectors (a vector extract) before making the mask;
- an all-ASCII test per block, so the UTF-8 masks are made only when a byte is 0x80 or above.

The last is a branch. It would be the second kind of branch in pass 1's body.

**Pass 2: the number check is 80% of it on files of numbers.** Without the check:

| file | pass 2 with the check | pass 2 without it |
|---|---|---|
| canada | 4.56 ms | 0.87 ms |
| marine_ik | 10.41 ms | 1.83 ms |
| twitter | 0.33 ms | 0.23 ms |
| gsoc-2018 | 0.51 ms | 0.47 ms |

The check is a block program, run over a 64-byte block for every number. Most numbers are a few
bytes long. Two ways to make it cheaper:

- run the check over an 8-byte word, as the words' `WORD` test reads one, when the number is
  short;
- or check numbers in pass 1, over every byte at once, as masks.

Without the check, pass 2 runs at 2.6 GB/s on `canada.json` and 7.1 GB/s on `gsoc-2018.json`.

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
