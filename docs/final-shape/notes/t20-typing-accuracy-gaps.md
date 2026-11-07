# t20: Typing support and accuracy across the main languages: gaps and plan

## Intro

The typing design (03 §13) is approximate inference written as per-language programs over the AST. Their
results are facts like any other: file-local first, then bounded cross-file rounds over interface summaries,
with interned type terms, `proven:`/`possible:` predicates, user declarations and dependency summaries.

This note surveys what each main language has against that design (2026-11-06, on `final-rerail-shape`
`69e98b01`). It ranks the gaps by how much rule accuracy closing each one would buy, and sets the order of
work. The short version: the machinery exists but no rule can reach it.

- **Calls.** The only typing-like answer a rule can read is the native call resolver's, through `calls:`.
  No pack rule uses it; only data-layer checks do.
- **Typing modules.** `typing::{terms, declare, deps, cross_rounds}` and the typing programs run only in tests
  and `rerail-engine logic`.
- **Types.** No rule can ask for a type. `value: {type: …}` is the type of a literal, and `proven:`/`possible:`
  have no compiler or matcher code (`typing/terms.rs:80` implements them, unused).

## What each language has

| | Python | Rust | PHP / Blade | TypeScript / JavaScript |
|---|---|---|---|---|
| Name resolution | Ruff's model (`scope/python.rs`) | lexical + `use` (`scope/rust.rs`) | PHP-Parser's `NameResolver` (`scope/php_names.rs`) | eslint-scope, value and type spaces (`scope/ecma.rs`) |
| Calls in production | native resolver: C3, metaclass `__call__`, multi-dot imports | native: paths, `use`, `impl`; `self.m()` only | native: `$this`, `static`, traits, `new C` | **refused** (`language/adapter.rs:175-187`) |
| Calls in a typing program | `calls.rr`: points-to; also `super()`, locals holding instances, instance `__call__` | `calls_rust.rr`: `etype` over annotations, `let`, returns, fields | none | none |
| Expression types | `workload.rr`'s `type_of` (flat symbols) | `etype` in `calls_rust.rr` | `$this` or all-`new C` only (`calls/php.rs:379-415`) | none beyond literal values |
| Declared types read | stub syntax (`declare.rs`), `declared_ret` only | parameter, `let` and return annotations in `etype` | none: typed parameters, properties, returns and PHPDoc all unread | annotations captured (`Site.annotation`), unread |
| Dependencies | `deps.rs`: `RECORD`, `uv.lock`, `poetry.lock` | none (no `Cargo.lock`) | none (no `composer.lock`) | none (no `node_modules`, `.d.ts`) |
| Narrowing | none | none | none | none |
| Accuracy oracle | CPython runtime trace; Jedi corpus | rust-analyzer SCIP | PHPStan reflection + runtime trace; Psalm | scope and values only |

Recorded accuracy:

- **Python.**
  - The native resolver agrees with the runtime oracle on about 76 calls, with 17 departures, all
    unknown, and 0 wrong (`calls/python_tests.rs:28-112`).
  - The spike's `calls.rdl` scored 84 agreements, 9 departures and 0 wrong. It also agreed with Jedi on
    99.18% of 33,309 calls.
  - **No test on this branch scores the typing program against either oracle.** The tests check only
    parity with the spike's outputs, and the spike's scorers were not copied.
- **Rust.**
  - Native resolver: 12,289 of 73,258 calls named over `native/src`.
  - Program: 18,119 named. 32,136 calls are method calls whose receiver type is unknown (44%).
  - The fixture: 37 agreements, 7 departures (4 trait dispatch, 2 iterator chain), 0 wrong.
- **PHP.** 0 disagreements on the fixture (135 PHPStan records, 113 runtime records). 101 entries in
  `packs/php/divergences.json` are rerail-unknown, about 100 of them for lack of types or callees.
- **TypeScript.** Every type-checked typescript-eslint rule is unported (about 20 are in
  `recommended-type-checked`, an approved configuration with no ports).

## Gaps, ranked by the rule accuracy they buy

1. **Rules cannot read types.** Without this, nothing below reaches a finding.
   - **Change:** typing programs export `type_of(n, t)` over interned terms.
   - `core.rr` declares the rule-facing relations: `proven_type(n, t)`, `possible_type(n, t)`, and `some`/`every`
     over union members, as 03 §13 states them.
   - A bare `type:` keeps today's meaning.
   - **Unlocks:**
     - the PHPStan candidates (`phpstan-if-always-*`, `array-duplicate-key`, `random-int-parameters`: about
       90 divergences);
     - typescript-eslint's type-checked rules;
     - a sound LEG-083 receiver test beside the parity one.
2. **Receiver types of method calls.** These are the largest unknown share in every language that has calls.
   - **Python:**
     - locals holding instances;
     - call results with a known return;
     - `super()`, annotations, and parameter types from annotations.
   - **Rust:**
     - `etype` through call results, `?`, `Option`/`Result` and iterator adapters, with
       `typing::terms` instantiation;
     - trait dispatch to the single impl for a known receiver.
   - **PHP:** declared parameter, property and return types first, then PHPDoc `@var`/`@param`/`@return`
     (`literal/phpdoc.rs` already parses them in full).
3. **TypeScript/JavaScript calls and types.**
   - **Calls:** an `impl CallRules for Ecma` built from what `lib/tsx/names.rr` already resolves (imports,
     exports, barrels, `tsconfig` `paths`), with `module_graph`'s resolver producing names.
   - **Types:** declared types from annotations, and summaries of `lib.d.ts` and package `.d.ts` files.
     The most asked questions are "is it a Promise", "is it any", "is it a string or number", "is it nullable".
4. **One call graph per language.** Python has three call implementations (native, `typing/programs/calls.rr`,
   `lib/python/calls.rr`) and Rust has two. Production uses the weaker one.
   - **Change:** port the native resolver's strengths into the program (C3 rather than a union over ancestors,
     metaclass `__call__`, relative imports of two dots or more).
   - Then `calls:` reads the program's facts, once the oracles show it is no worse.
5. **Narrowing.**
   - `isinstance`, `assert` and `typeof`/`instanceof` guards narrow for the rest of the block.
   - `cast`/`as` narrow the expression only.
   - Needed for PHP's `if-always-*` and for nullable tests everywhere.
6. **Dependency universe beyond Python.**
   - `Cargo.lock`, `composer.lock`, and `package.json` with `.d.ts`.
   - typeshed for Python: 130 of the Jedi divergences are `try: from _x import f` accelerator imports.
7. **O41:** the unconverged frontier after k = 2 is not marked pending, so a missing row is a silent false
   negative instead of an unknown.

**A caution on parity.** Most Python and data-layer rules are held to parity with linters that guess types by
name (`endswith("txn")`, `\w*(Intent|Step)`). Better types change their findings. A typed rule goes beside its
parity twin, as `LEG_083_SOUND` does, never in its place.

## Order of work

- **Step 0. Measure.**
  - Unblock the typing tests (O81).
  - Score the typing programs against the oracles, in Rust tests: Python against the runtime trace, Rust
    against rust-analyzer.
  - Gaps 2 and 4 are judged by these numbers.
- **Step 1.** The rule-facing type relations (gap 1) over the Python program's `type_of`, with one rule to
  prove the path end to end.
- **Step 2.** Receiver typing, one language at a time, each scored by step 0's tests: Python, then PHP
  (declared types), then Rust.
- **Step 3.** ECMAScript calls (gap 3), then its declared types.
- **Step 4.** Narrowing and the dependency universe.

## Progress

- **O81** (step 0): the planner gives a derived relation named as a base relation a private name
  (`plan::private_derived_inputs`), so T's programs' `docof` no longer reads named columns from a positional
  table.
- **Scoring** (step 0): `typing::oracle_tests` runs a typing program through the corpus runner over an oracle
  project and compares its `call_target`/`call_open` with the recording.

  | Program | Oracle | Agree | Depart | Wrong |
  |---|---|---|---|---|
  | `calls.rr` | CPython runtime (`fixtures/python/calls`) | 83 | 10 | 0 |
  | native resolver, for comparison | the same | about 76 | 17 | 0 |
  | `calls_rust.rr` | rust-analyzer 1.99.0 (`fixtures/rust/calls`) | 37 | 7 | 0 (1 is a convention: a local `fn` pointer, followed to its function) |

  Each departure is pinned with its reason. One Python departure is a silent no-edge caused by O41: a
  re-export three files deep, which the k = 2 rounds cut. With k = 6 the rounds converge and it agrees, so O41
  is an accuracy bug, not only a report.
- **Decisions** (0047, the user): rounds k = 4 with a pending round; `calls:` moves to the programs after parity;
  type relations drafted here first; Python first for receiver typing.
- **O41 narrowed:** at k = 4 calls.rr converges on the oracle (84 agree, 9 depart, 0 wrong). Cut short, the
  pending round makes the re-export unknown, never no edge.
- **Python parity** (decision 0047, point 2).
  - The runtime oracle gained the cases the native resolver models and the program did not: a metaclass
    `__call__`, a `__new__`, a C3 diamond, and relative imports two dots up (`pkg/hierarchy.py`,
    `pkg/sub/deep.py`).
  - The tracer now records what CPython runs for a class call: the metaclass's `__call__`, `__new__` and
    `__init__` where the workspace defines them, not `__init__` alone. It runs under 3.13.18, which reproduced
    the old recording exactly.
  - `calls.rr` now has:
    - `__new__` and metaclass `__call__` targets;
    - relative imports of any depth (`@pkg_up`);
    - method resolution per receiver class. A definition on a class another candidate subclasses is dropped at
      the call. Incomparable candidates make the call unknown, never both.

  | | Agree | Depart | Wrong |
  |---|---|---|---|
  | `calls.rr` | 94 | 9 | 0 |
  | native resolver | 84 | 19 | 0 |

  Every departure of the program is one the native resolver also has. The program departs nowhere the native
  resolver agrees. That is decision 0047's condition for the switch. The Jedi corpus comparison is a broader
  accuracy check beside it, not a gate (O88).

- **Rule-facing type relations, Python** (decision 0047, point 3; step 1). `calls.rr` exports them beside
  `call_target`/`call_open`. They are three-valued, as calls are, and read the program's points-to values:

  ```text
  type_of(n, t)        n may hold a value whose class is t ("pkg.models.Base", or a builtin's "str")
  type_open(n)         n may also hold a value of a class not listed
  proven_type(n, t)    isinstance(n, t) is true: every class n may hold is t or a subclass, none unlisted
  possible_type(n, t)  isinstance(n, t) may be true: some listed class is t or a subclass of t
  possible_open(n)     possible_type is incomplete
  ```

  A rule's `proven: t` is `proven_type(n, t)`. Its `possible: t` is `possible_type(n, t)` or `possible_open(n)`.
  `not` is the negation of either.
  - **New values in the program:**
    - **Builtin values** (`pv(n, "builtin", n)`): literals, and calls of builtin classes (`str(x)`, `dict()`).
      Their methods are no workspace definition, so `str(x).upper()` is now no edge rather than unknown.
    - **External values** (`pv(n, "ext", n)`): external imports, builtin names, their attributes, and
      comparisons. They mark a name that may hold something the program does not model. T's outputs on T's
      facts are unchanged (`product_programs_derive_t_outputs`). Without this marker, `json/encoder.py`'s
      `_iterencode` lost its unknown.
  - **Classes:**
    - an instance's class is the workspace class's qualified name;
    - a class object's class is `type`, unless a metaclass, an external base or an unknown base may change it;
    - a module's class is `module`, including an external module bound by `import`.
  - **Declared returns** (`declared_ret`, from `rerail.types/v1` and dependency summaries) type a call of a
    name the workspace does not define, as in `workload.rr`. The corpus runner interns `type_of` as one union
    term per node (`x.node_type`).
  - **Oracle:** the CPython tracer now records each method call's receiver: the span of `o` in `o.m(...)`, and
    each class it was, with its MRO. `typing::oracle_tests::python_type_relations_agree_with_the_runtime`
    checks three things at every receiver:
    - a closed `type_of` lists every class CPython saw;
    - every proven class is in each runtime class's MRO;
    - every class in an MRO is possible.

    A new fixture module (`pkg/typed.py`) adds builtin and local receivers.

  | | Receivers | Exact | Wider | Unknown | Wrong |
  |---|---|---|---|---|---|
  | `calls.rr` types | 44 | 44 | 0 | 0 | 0 |

  | Calls on the grown oracle | Agree | Depart | Wrong |
  |---|---|---|---|
  | `calls.rr` | 113 | 9 | 0 |
  | native resolver | 97 | 25 | 0 |

  A bytes literal is told from a str by `strprefix`, which T's shape of the syntax facts already makes.
  The receiver fixture is small and written for this, so 44 of 44 says the relations are sound where they
  answer, not how often they answer on a real corpus. What they leave out is O89.

  **For More-ergonomic-rule-syntax (information).** The relations above, with their columns, are what
  `core.rr` would declare for Python's `proven:`/`possible:`. They are in
  `native/src/code_rule_dsl/typing/programs/calls.rr`, section "types". Type names are dotted qualified names
  for workspace classes and bare names for builtins, as `declared_ret` and `workload.rr`'s `type_of` write them.

- **The switch, Python** (decision 0047, point 2). One implementation of Python call resolution remains for
  `.rr` rules: the typing program.
  - **The bridge** (`logic::plan::corpus::bridge`). A rule reads a typing program's output by declaring an input
    `typing_<program>__<output>`, with each node column as four: path, kind, start, end. When a rule reaching an
    output reads one, the corpus runner first runs `typing/programs/<program>.rr` over the same root, in its own
    bounded rounds and with the same typing inputs. The rows become the rule's facts, and `Report::typing_runs`
    has the program's report. The rule joins spans back to its own nodes, so T's numbering and `core.rr`'s
    need not agree.
  - **`lib/python/calls.rr` is retired** as a resolver. It now joins the program's `call_target`,
    `call_open` and type relations to the rule's nodes. It keeps the chain relations (`calls`, `calls_open`,
    `reach`, `owner`) and the syntax helpers the data-layer rules use (`call`, `callee`, `call_arg`, `posarg`,
    `awaited`). Its points-to copy is gone. No rule read it: the data-layer rules use only the syntax helpers.
  - **Check:** `typing::oracle_tests::rules_read_the_python_program_through_lib_calls`. A rule including the
    library, run on `core.rr`'s facts, gets exactly the program's rows on the oracle project: 93 call edges,
    358 proven types and 9 unknown calls.
  - **Left:** the JSON rule term `calls:` is not lowered to `.rr` by the converter (`logic::convert`
    refuses it, for every language, as before this work), so it still reaches only the native resolvers.
    No pack rule uses it (the Python pack exempts the capability). Lowering it onto the library's `reach` is
    what removes the native Python resolver.

- **Cost of `calls.rr` on django** (2026-11-06). The first Jedi scoring run used 1,126 s of CPU and did not
  finish. Profiling slices of django (`RERAIL_PROFILE`, debug build, CPU = user + sys, machine load 30-97)
  showed where the time went. Each fix below keeps every output the same, `pv`, `tv` and `vopen` included.

  | Fix | Slice | Cross-file CPU before → after |
  |---|---|---|
  | `alook(n, c, a)`: an attribute's lookup class, joined once | utils (49 files) | 17.5 → 8.7 s |
  | `pass(n, c)`: pass-through values made per file | utils + template (75) | 32.4 → 16.2 s |
  | A join through same-document atoms is not a hop (`cross_rounds::bound`) | db/models (24) | 36.4 → 18.0 s |
  | `fv(c, a, k, v)`, and proven types from one type's supertypes | db/models (24) | 18.0 → 9.3 s |
  | `type_first` by `min`, not by pairs compared as text | utils + db (172) | 73 s, of which 25 s was this |
  | `fret(f, k, v)` | utils + db (172) | → 40.9 s |
  | Node types written in one transaction | utils + template (75) | write 2.7 → 0.07 s |
  | Previous round's rows bound from memory | utils + template (75) | 5.63 → 3.86 s |

  At the end: 75 files take 3.9 s, 172 files 37 s and 393 files (django without `contrib` and `conf`) 51.5 s,
  with a peak RSS of 552 MB. Before, 172 files did not finish in 110 s.

  The program-side fixes work around engine costs, which are listed in O90.

- **`calls.rr` against Jedi on django** (O88; `typing::oracle_tests::benchmark_python_calls_program_corpus`,
  ignored). Jedi 0.19.2 `goto` recorded over django's 907 files. The run took 112 s of CPU (debug, the test with
  its scoring) and a peak RSS of 815 MB.

  | Calls | Count |
  |---|---|
  | All | 37,141 |
  | Jedi undecided | 9,491 |
  | Both decided | 20,364 |
  | Agree | 20,205 (99.2% of both decided) |
  | Program: another workspace definition | 142 |
  | Program: a workspace definition, Jedi: external | 17 |
  | Program unknown, Jedi decided | 7,286 |

  The spike's `calls.rdl` scored 99.18% of 33,309 (t9 §7.4). Most of the 142 resolve through a decorator. For
  example, `mark_safe(...)` is decorated with `keep_lazy`, and the program gives the wrapper in
  `utils/functional.py`, where Jedi gives the decorated `def`. Most of the 17 are `super().__init__` calls, where
  Jedi stops at `object.__init__` and the program gives a workspace base's. Neither group has been judged one by
  one. The 7,286 unknowns are where the program says `call_open` instead of guessing.

- **Declared types, Python** (2026-11-07; O89, step 2). `calls.rr` reads parameter, return and class-attribute
  annotations, and decorators declared `(fn: F) -> F`; what it trusts and what stays open is in O89. Django is
  barely annotated, so the score was taken on the data layer (commit 41ab36f): Jedi 0.19.2 `goto` recorded over
  its l1–l20 packages (489 files, 246 s of CPU), and both programs scored on 84 of those files
  (`_internal`, `l1100_exceptions`, `l1200_json`, `l1400_types`, `l2050_transaction`, `l2055_lazy_result`):

  | Calls of the 84 files | Before | Annotations read |
  |---|---|---|
  | Agree | 4,917 | 5,454 |
  | Program unknown, Jedi decided | 2,108 | 1,571 |
  | Program another definition | 0 | 0 |
  | Program no edge, Jedi a definition | 412 | 412 |
  | CPU (debug, load about 80) | 13.0 s | 15.3 s |

  Every newly decided call agrees with Jedi. The 412 calls with no edge are calls into packages outside the 84
  files: Jedi saw all 489 files, the program saw only these. Scoring all 489 files is O94. The runtime and
  receiver-type oracles are unchanged.

- **The first rule that reads types** (step 1's end-to-end rule): `docs/rule-syntax/data-layer/leg-083-typed.rr`.
  - **What it does.** LEG-083 counts every `x.resolve()` as an eager lazy terminal, by name; at the data layer's
    October commit all its findings are pathlib paths or lookups (`data-layer/upstream/LEG-083.md`). This rule
    counts a `resolve()` or `resolve_async()` with no arguments only when its receiver may be a `LazyResult` or a
    `LazySeq` (`possible_type`). It reports a function that reaches one through calls in its own file.
  - **Fixture.** `typing::oracle_tests::leg_083_typed_counts_resolve_by_the_receivers_type` pins it on
    `fixtures/python/leg083`: a declared return, a decorated method, a class attribute and an attribute chain are
    counted; a path, a quoted annotation and an untyped parameter are unknown; a call with an argument is
    not counted.
  - **Data layer.** On the 84 files, the 16 calls it counts are all lazy terminals, and none of the 17 pathlib
    calls is counted. Those are unknown until external classes have types (O89).
  - **What it took in the engine.** The corpus runner now drops rules no output reads: including
    `python/calls.rr` had run all of `names`, `imports` and `flow`. The bridge hands the typing rows over as
    values, not as `.rr` text. On 39 files the rule went from over 100 s (killed) to 9.8 s, most of it the
    typing program's own run.
  - **Not yet.** Legacy surfaces (the rest of the LEG port) and LEG-083's file scope are not applied, so the 17
    findings on the 84 files are all in test files.

  **For More-ergonomic-rule-syntax (information).** The rule is the pattern for a `possible:` test on a receiver:
  `possible_type(o, t), lazy_class(t)`, with `terminal_unknown` for the receivers no type decides.

