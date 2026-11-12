# 0119. A rule file carries its own tests

Status: built (2026-11-12).

## Intro

A rule is tested today away from its `.rr` file. Its cases are a JSON envelope (`rerail rules test draft.json`,
`fixtures/rerail/<id>.cases.json`), a pack's upstream expected findings, or a Rust or TypeScript test that writes
files into a directory and checks what the engine reports. So the cases a rule was written against drift from the
rule. And a rule author has to write some code in another language to add a case.

This decision puts the cases in the rule file. A test is a block in the `.rr` file. It has:

- a name;
- **fixtures**: the files the test's run reads, inline or next to the rule file;
- optional **testing rules**: ordinary clauses and declarations that only the test sees;
- **assertions**: findings that must or must not be reported, or rows that a relation must or must not hold.

Outside testing mode, a test block is parsed, kept in the file's tape, and dropped by lowering. So a program with
tests is the same program as one without them, with the same key, plans and stored results. In testing mode, each
test is a run of the same engine over its fixtures, in a fact space of its own.

## A sample

```
.policy "bandit-b101-assert-used" revision 2
.language python
.scope include "**/*"
.scope exclude "**/tests/**"

.violation B101(at: node) code "B101"
    "Replace this assert with an if check that raises."
B101(a) :- kind(a, "assert_statement").

// A test: from `.test` to its `.end`.
.test asserts_are_reported
    // A fixture: a file of the test's run, by its path.
    // The fenced body is the file's text, less the
    // closing fence's indent.
    .fixture "app/views.py" ```
        def view(user):
            assert user.is_admin
            return render(user)
        ```
    // The finding (code, path, line) must be reported.
    .expect B101 at "app/views.py":2
    // And none on line 3.
    .expect no B101 at "app/views.py":3
.end

// `exactly`: the findings are the ones `.expect`
// lists, and no others.
.test scope_leaves_out_tests exactly
    .fixture "app/a.py" ```
        assert x
        ```
    // Outside the scope: must not be reported.
    .fixture "app/tests/test_a.py" ```
        assert x
        ```
    .expect B101 at "app/a.py":1
.end
```

## The decision

1. **Blocks.** Two blocks, each up to its `.end`; neither nests, and each name is unique in its file:
   - `.test <name> [exactly] [uses <set>, …]`: a test.
   - `.fixtures <name>`: a fixture set, which tests name with `uses`. What a set holds comes first in each test that
     uses it, in the order `uses` names the sets.

   A file may hold any number of them, after or between its rules, and a library module (no `.policy`, no
   `.violation`) as well as a policy.

2. **What a block holds.**
   - `.fixture "<path>" <text>`: a file of the test's run. `<text>` is a fenced literal (the lexer's backtick
     fence, which may span lines). The fence's body loses its first newline, and a final newline is added.
     The closing fence, on a line of its own, says the indent, as in Java's text blocks and Swift's multi-line
     strings: each line loses that much leading whitespace, and keeps any more. So a fixture is written at the
     test's indent, and a file whose lines are all indented is written one step further in than its fence:

     ````
     // Inside a test: the file is "    x = 1".
     .fixture "a.py" ```
             x = 1
         ```
     ````

     A closing fence after text on its line says no indent: the lines lose the indent they share.
   - `.fixture "<path>" from "<file>"`: the file's bytes. `<file>` is relative to the `.rr` file, so binary inputs,
     large inputs and upstream cases can sit next to the rule.
   - `.given <relation>, …`: every rule of the relation is dropped for the test, in the file and in what it
     includes, so the relation holds the test's facts and nothing else. A core relation (`kind`, `node`, …) is the
     engine's and is refused: its rows come from a `.fixture` file.
   - Declarations, clauses and facts (`seen("a.py").`): **testing rules**. They are the test's own and may read the
     file's relations. A fact for a `.given` relation is one of its rows.
   - In a test only, `.expect`:
     - `.expect <code> [at "<path>"[:<line>[:<column>]]]`: a finding. This is the findings contract: a set of
       finding keys, never a finding's shape or message. `_` is any code.
     - `.expect no <finding>`: no finding matches. Without a line, none in the file; without `at`, none anywhere.
     - `.expect <relation>(<value>, …)` and `.expect no <relation>(…)`: a row of any relation, the file's or the
       test's. `_` matches any value (`.expect no imported("os", _)`).
     - `.expect only <relation>(…)`: the relation's rows are the ones its `only` lines name, and no others.
     - `.expect count <code or relation> = <n>`.

   `exactly` on a test adds one assertion: its findings are exactly those its positive `.expect`s name. A
   `.expect` with no line then matches every finding in that file. `exactly` takes no `.expect no`.

3. **Unit tests.** A test with no fixture file reads no file: its run has an empty corpus, and its relations are
   its rules over its facts. With `.given`, a test checks the rules that read a relation from rows it names, with no
   parse and no tape. A test over files and a unit test are the same mechanism, so a test may mix them: a fixture
   file for what the tape gives, `.given` for the step under test.

4. **Parsing.** Each block is a tape node of its own (`test_block`, `fixtures_block`: the name under `name`,
   `exactly` under `modifier`, each set `uses` names under `argument`, the statements under `body`) and an item
   (`syntax::Item::Test`, `Item::Fixtures`), in the engine's parser (`syntax.rs`), the lean parser (`lean.rs`) and
   `from_tape.rs`. Without its own node, a block's clauses would parse as the file's. The compiled grammar
   (`rr_syntax/bitlex_ladder/22-test-blocks.bitlex`, `rerail-rr-parser`) reads them too, to the same tape: the
   setting opened at the `.` is the block, its statements a nested level read with a flag (`in_block`) under
   which no block opens, and `.end`'s `.` is a mark that is not hidden, so a `.end` outside a block refuses the
   file as the engine does.

5. **Never the program's.** `rr::lower` skips both items, always: no option exists that could leak a test into a
   program. So a test never reaches `front::Program`, its program facts or any key made from them (plans, choices,
   prepared and combined programs, memos). The guard is `a_rule_files_tests_are_no_part_of_its_program`: each file
   with tests lowers to the same program as with its blocks' lines blank. A block does change its file's text, and
   so its tape: that is the parse, and it is right that it changes.

6. **A test is an ordinary run** (`logic/rr/testing.rs`):
   - its fixture files are written into a directory of its own, the run's root;
   - its program is the file's statements, each `.given` relation's rules dropped, with the test's (and its sets')
     rules spliced in after the file's own, read as `read_file` reads a file (`rr::lower_files`);
   - its run is `run-rr`'s (`tools::run_program`), with the file's scope, suppressions and settings. The relations
     its `.expect`s name are made outputs of the run;
   - its assertions are checked against the run's findings and those relations' rows.

   A test's rules are in its run alone, so tests need no namespace. A test with no rules and no `.given` has the
   file's program, so it shares the file's plans with every such test and with real runs: testing mode is the same
   engine, with nothing evaluated outside it.

7. **A store of its own.** In the engine's tests each test has its own store (decision 0118). Elsewhere testing
   mode uses `$TMPDIR/rerail-test-rr-stores/<build>.sqlite`, one per engine build, kept between invocations, unless
   `RERAIL_TAPE_CACHE` names one (`corpus::use_store`, for the report's length). The project's store (decision
   0120) is its real runs': a test's run would read what they left there, and wait behind their writes
   (O-test-rr-busy).

8. **Where testing mode is.**
   - `rerail-engine test-rr <file or directory>... [--filter <text>] [--json]`: the tests of every `.rr` file named,
     one line each (`pass`, or `FAIL` with each failed `.expect`, its line and what the run had instead), exit
     status 1 on any failure. `--filter` keeps the tests whose `<file>::<name>` contains its text.
   - `rerail rules test <rule.rr>` (the TypeScript CLI): the same, through the Engine's `CodeRuleDslRuntime.testRr`,
     `--json` for the report. It does not reimplement anything.
   - The engine's tests: `rule_files_pass_their_own_tests` runs every test under `docs/rule-syntax/`.

   Nothing else runs tests. A scan, a library load (phase 0) and the product's engine read a file as they read one
   with no tests.

9. **Printing.** `flatten` keeps a file's text as written (`program_text`), so its blocks go with it. `rr::print`
   prints a lowered program, which holds no test, so what it prints has none.

10. **Today's cases move in by conversion.** `convert-rr` takes a draft (`{ kind, rule, cases }`) or, with
    `--cases <cases.json>`, a rule and its cases file (`fixtures/rerail/<id>.cases.json`). Each case becomes one
    test (`testing::tests_of_cases`): a caught case its file and `.expect _ at "<path>"`, a passed case its file and
    `.expect no _ at "<path>"`. A case names no code, hence `_`. The tests are in the report's `tests`, and after the
    program in its `program`; they are printed for a rule that needs a hand-written port too, ready for the port. A
    printed fixture's fence is longer than any run of backticks in it (`print_test`), and reads back byte for byte
    (`a_printed_test_reads_back_as_written`). A pack's upstream cases stay in their tool directory and its
    conformance gate; a rule may name some of them with `.fixture … from`.

## What is open

- **Typing facts in a test.** A test's run has the typing phase like any run. A test that needs a library's stubs
  names them as fixture files. Whether a test can name a library module instead is open.
- **Edits** (`.edit`, a fix's replacement): a `.expect fix <code> at …` that asserts the text after the fix is a
  natural next step. It is not designed here.

## Samples

- `packs/python/bandit-b101-assert-used.rr`: three tests of a pack rule.
- `spikes/rr-tests/print-calls.rr`: every form. Inline and `from` fixtures, a fixture set used by two tests, a
  testing rule, row, `only` and count assertions, `exactly`, and a unit test (`.given called`) with no files.

## Measured

- `rerail-engine test-rr docs/rule-syntax` (8 tests), six times in a row: all pass each time, with no busy writer.
  The first run made testing mode's store: 16.3 s of wall time, 3.2 s of CPU, 29.6 G instructions. The other five
  took 5.6 to 7.7 s of wall time, 1.7 to 2.5 s of CPU and 18.1 to 18.2 G instructions each (load 32 to 45). The
  store is 4.4 MB, against the machine's 4.4 GB.
- Wrong expectations fail, and each failure names its line and what the run had (`reported P001 at
  "app/main.py":2`, `printed_line has 2 rows: (2), (3)`, `counted 2`, `.expect only reach: also 2: (a, c), (b, c)`,
  `exactly: also reported …`).
- The engine's nine tests of testing mode take 8.2 s of wall time together.
- Under `RERAIL_SQL_STRICT=1`, `test-rr docs/rule-syntax` passes all 8: testing mode's store is small enough to
  read whole before the first wave. So does `run-rr` on its default store (decision 0120).
- The compiled grammar reads both files with tests as the engine does, as rules and as one machine
  (`rule_files_with_tests_are_the_engines`), and refuses what the engine refuses: a nested block, a `.end`
  outside a block, a block with no `.end` (`tape_is_the_engines`).
