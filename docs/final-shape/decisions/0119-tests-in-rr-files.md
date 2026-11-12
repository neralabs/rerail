# 0119. A rule file carries its own tests

Status: spike built (2026-11-12): items 1 to 5, but not `rerail rules test`, and not item 6. See "What the spike built" below.

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
    // indent its lines share.
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

1. **Syntax.** A test is `.test <name> [exactly]`, then items, then `.end`. Its items:
   - `.fixture "<path>" <text>`: a file of the test's run. `<text>` is a fenced literal (the lexer's backtick
     fence, which may span lines). The fence's body loses its first newline and the leading whitespace its
     lines share, and a final newline is added, so a fixture is written at the test's indent.
   - `.fixture "<path>" from "<file>"`: the file's bytes. `<file>` is relative to the `.rr` file, so binary inputs,
     large inputs and upstream cases can sit next to the rule.
   - `.expect <finding>`, where `<finding>` is a code, `at "<path>"` and an optional `:<line>` or
     `:<line>:<column>`. This is the findings contract: a set of finding keys, never a finding's exact shape or
     message.
   - `.expect no <finding>`: no finding matches. Without a line, none in the file; without `at`, none anywhere.
   - `.expect <atom>` and `.expect no <atom>`: a row of any relation, the policy's or the test's. `_` matches
     anything (`.expect no imported("os", _)`).
   - `.expect count <code or relation> = <n>`.
   - Declarations, clauses and facts (`seen("a.py").`): **testing rules**. They are the test's own and may read the
     policy's relations. A test can name what a finding alone would not show, e.g. an intermediate relation, or a
     fact the run should be given.

   `exactly` on the header adds one assertion: the test's findings are exactly those its positive `.expect`s
   name, keyed as they are written. A `.expect` with no line then matches every finding in that file.

   A file may hold any number of tests, after or between its rules. Tests do not nest. A test's name is unique in
   its file.

2. **Parsing.** `syntax.rs` parses a test into its own tape node (`Kind::TestBlock`, with its items under it) and
   `Item::Test { name, exactly, items }`. Without this, its clauses would parse as the policy's. Fenced fixture
   bodies are already one token. `from_tape.rs` rebuilds `Item::Test`. The `.rr` printer prints it back, so the
   converter and the formatter keep a file's tests.

3. **Dropped unless testing.** `rr::lower` skips `Item::Test` unless the reader asks for tests (a `tests: bool` on
   the read options; `false` everywhere but testing mode). So a test never reaches `front::Program`, its program
   facts (`program_facts.rs`) or any key made from them: plans, choices, prepared and combined programs, memos.
   Two guards hold this:
   - a test that lowers the library and the packs with and without their tests and asserts equal program facts;
   - `front::Program` has no field for tests, so no later step can read one by accident.

   A test block does change its file's text, and so its tape. That is the parse, and it is right that it changes.

4. **Testing mode runs each test as an ordinary run.** For each test:
   - its fixtures are written into a directory of its own (under the test's store in the engine's tests,
     decision 0118; a temporary directory from the CLI), and that directory is the run's root;
   - its program is the policy's program, plus the test's testing rules if it has any. The rules are combined as
     a library is (`combine_texts`), in the test's own namespace (`test.<name>.`), so two tests' relations never
     meet;
   - its run is `corpus::run_with_options` over that root, with the policy's scope, suppressions and settings;
   - its assertions are checked against the run's findings and its relations' rows.

   A test with no testing rules has the policy's program. So it shares the policy's plans with every other such
   test and with real runs. That keeps testing mode the same engine (phases, not new evaluators), with nothing
   evaluated outside it.

   The runs of one file's tests are independent and may go in parallel.

5. **Where testing mode is.**
   - `rerail-engine test-rr <file or directory>...`: runs the tests of every `.rr` file named, one line per test
     (`pass`, `fail` with the assertions that failed and what was reported instead), exit status 1 on any failure.
     `--filter <text>` keeps the tests whose `<file>::<name>` contains it.
   - The engine's Rust tests: `rule_syntax_files_pass_their_tests` runs every test of `docs/rule-syntax/` and the
     packs. Each `.rr` file is one test case, so a cargo filter selects files.
   - `rerail rules test` (the TypeScript CLI) calls the engine's entry point for a `.rr` draft. It does not
     reimplement it.

   Nothing else runs tests. A scan, a library load (phase 0) and the product's engine read the file as it reads
   one with no tests.

6. **Today's cases move in by conversion, not by hand.** A `{ caught, passed }` envelope converts into one test
   per case: a caught case becomes a fixture and `.expect <code> at "<path>"`; a passed case becomes a fixture and
   `.expect no <code> at "<path>"`. The converter that prints `.rr` (rules are Soufflé `.rr` only) prints them. A
   pack's upstream cases stay in their tool directory and its conformance gate. A rule may name some of them with
   `.fixture … from`.

## What is open

- **Typing facts in a test.** A test's run has the typing phase like any run. A test that needs a library's
  stubs names it as a fixture. Whether a test can name a library module instead is open.
- **Fixtures shared by a file's tests** (a `.fixtures` block that every test reads) are left out until a file
  needs them.
- **Edits** (`.edit`, a fix's replacement): a `.expect fix <code> at …` that asserts the text after the fix is a
  natural next step. It is not designed here.

## What the spike built

- **Parsing** (item 2): `Item::Test`, the tape kind `test_block` (its name, `exactly` as a modifier, its statements
  under `body`), in the engine's parser (`syntax.rs`), the lean parser (`lean.rs`) and `from_tape.rs`. The Bitlex
  grammar does not know the block yet (O-rr-tests-bitlex).
- **Dropped** (item 3), more simply than the read option above: `rr::lower` skips `Item::Test` always, so no
  option exists that could leak a test into a program. Testing mode splices a test's statements into the file's own,
  then lowers them as `read_file` does (`rr::lower_files`, split out of `read_file`). The guard is
  `a_rule_files_tests_are_no_part_of_its_program`: each file with tests lowers to the same program as with its
  tests' lines blank.
- **Running** (item 4, `logic/rr/testing.rs`): each test is `run-rr`'s run (`tools::run_program`, split out of
  `run_rr`) over a directory holding its fixtures. No namespace is needed: a test's rules are in its run alone. The
  relations an `.expect` names are made outputs of the run.
- **Where** (item 5): `rerail-engine test-rr <file or directory>... [--filter <text>]`, and the engine's tests
  `rule_files_pass_their_own_tests`, `failed_expectations_say_what_the_run_had` and `test_blocks_parse_only_whole`.
  `rerail rules test` does not call it yet.
- **Samples:** `packs/python/bandit-b101-assert-used.rr` carries three tests. `spikes/rr-tests/print-calls.rr` uses
  every form: inline and `from` fixtures, a testing rule, row and count assertions, and `exactly`.
- **Not kept by the printer:** `rr::print` prints a lowered program, so `convert-rr` and `flatten` leave tests out.
  A formatter that keeps them is still to be written.

## Measured

`rerail-engine test-rr` on the two sample files: 6 tests pass. The same tests with wrong expectations fail, and
each failure names its line and what the run had (`reported P001 at "app/main.py":2`,
`printed_line has 2 rows: (2), (3)`, `counted 2`, `exactly: also reported …`). The engine's four tests take 7.3 s
of wall time in all, with 4.3 s user and 4.2 s system CPU (62.9 G instructions). Under `RERAIL_SQL_STRICT=1`,
`test-rr` stops as `run-rr` does (O-run-rr-strict).
Once in four tries over `docs/rule-syntax/`, one test's run found the store's writer busy (O-test-rr-busy).

