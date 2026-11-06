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

