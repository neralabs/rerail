# Language packs

Each language family's engine code sits behind one adapter (`native/src/code_rule_dsl/language/adapter.rs`). The
host's schema checks shapes only; the engine alone refuses what a rule's language lacks. The registry, the
not-supported-yet behaviour and the engine adapter seam are implemented.

## What a language pack is

A language is three parts, each owned by one place.

| Part | Where | Holds |
|---|---|---|
| Registry entry | `languages/<id>.json` | Id, family, dialects (file suffixes, parser, grammar, tape tag), supported capabilities, suppression comments, embedded languages, status |
| Engine adapter | `native/src/code_rule_dsl/language/adapter.rs` and its concern files, one adapter per family | How the engine reads it (scope, values, call arguments, call graph, format strings, suppressions), and the DSL features it refuses |
| Rule pack | `packs/<id>/` ([pack structure](pack-format.md)) | Code rules written in it, with evidence that they behave |

## The layering rule

Language-specific code lives only in a language's adapter and registry entry; nothing else branches on a language.

Core, the host and packs ask the registry or the adapter. Both sides are generated from `languages/`:

- **Engine:** language tables generated at build time (`native/build.rs`, `native/src/build_languages.rs`).
- **Host:** `src/engine/code-rule/code-rule-language-registry.generated.ts`, written by `npm run generate:languages`.
  A test fails if it is stale, and a native test checks it agrees with the engine's tables.

Only the engine decides whether a rule uses something its language lacks.

<details>
<summary>How the engine refuses a rule</summary>

"Something its language lacks" is a key, option, format style, suppression comment or another family's grammar. The
host's schema checks a rule's shape only. Every write, and `validateCodeRule` from `@neralabs/rerail/rules`, compiles
the rule in the engine. The engine refuses it as invalid, or as `language-not-supported` when the registry plans the
language or capability.

</details>

## The engine adapter

Each language family has one adapter type, and every engine call site asks it instead of matching on a language.

- **Adapter:** the trait each family's type implements: `Python`, `Ecma`, `Php`, `Blade`, `Rust`, `Vue`, `Sql`, `Rr` or `Json`.
- **Concern traits:** one per capability (`ScopeRules`, `ValueRules`, ...); a concern the family lacks is `Refused`.
- **Embedding family:** Blade and Vue read the embedded language's semantics over an embedded program.

[Adding a language](#adding-a-language) lists the steps.

| Family | Languages | Capabilities | Suppressions |
|---|---|---|---|
| `python` | `python` | all five | `noqa`, `nosec` |
| `ecma` | `typescript`, `javascript` | `scope`, `value`, `arguments`; `calls` and `formatArgs` `Refused` | `eslint`, `oxlint`, `biome` |
| `php` | `php` | all five | `phpstan`, `psalm`, `phpcs` |
| `blade` | `blade` | all five | none |
| `rust` | `rust` | all five | `clippy` |
| `vue` | `vue` | `scope`, `value`, `arguments`; `calls` and `formatArgs` `Refused` | `eslint` |
| `sql` | `sql` | `scope`, `value`, `arguments`; `calls` and `formatArgs` `Refused` | `sqlfluff` |
| `rr` | `rr` (`.rr` rule files, the engine's own grammar `native:rr`) | none: all five `Refused`; rules match its kinds and text | none |
| `json` | `json` | none: all five `Refused` (rules read its tree) | none |

<details>
<summary>What each concern trait holds</summary>

| Part | Holds |
|---|---|
| `Adapter` | Diagnostic label, line terminators, name normalisation (Python's NFKC, SQL's PostgreSQL case folding), module paths from package markers (Python), inline suppression comments (an embedding family's read every document of its files) |
| `ScopeRules` (`adapter/scope.rs`) | Scope tables: building, resolving, bindings, usage, PHP's flow, SQL's query blocks; refused binding and resolution features |
| `ValueRules` (`adapter/value.rs`) | Value decoding, kinds, structures, names, keys and references for comparisons, type names; refused compare modes |
| `ArgumentRules` (`adapter/arguments.rs`) | Call kinds, a call's arguments; refused signature markers and modes |
| `CallRules` (`adapter/calls.rs`) | Call and function kinds, per-file call summaries, call-graph links and labels |
| `FormatArgRules` (`adapter/format_args.rs`) | The format styles a family reads |
| `SyntaxRules` (`adapter/syntax.rs`) | Literal grammars a family hosts (`parse`), `languageOptions` and `when` keys |

</details>

<details>
<summary>How dispatch, refusals and embedding work</summary>

- **Adapter files:** `Python` (`adapter/scope.rs`, `value.rs`, ...), `Rust` (`adapter/rust.rs`), `Vue`
  (`adapter/vue.rs`), `Sql` (`adapter/sql.rs`), `Rr` (`adapter/rr.rs`) and `Json` (`adapter/json.rs`).
- **`with_adapter!`** is the one exhaustive match on a family. It, `Family`, `GrammarDialect`, `Family::of` and
  `honours` are generated from the registry; no enum is edited by hand.
- **Every other call site** asks the adapter. One that needs a concern's associated type calls a function generic over
  `A: Adapter` from inside the macro.
- **Call names** carry their family (`CallRules::NAMES`). **Value semantics** (`truthy`, `type_name`, `equals`,
  `order`, `relation`, `numeric`, `integer`, `to_str`) belong to `ValueRules`.
- **Refusals.** A concern the registry does not mark `supported` for a family is `Refused`. One gate (`gate`) reports
  a refused feature as invalid and a planned one as `language-not-supported`.
- **Embedding families** take the embedded language's semantics, over the embedded program: a document of its own,
  read with that family. `Blade` delegates to `Php` over the compiled view; `Vue` to `Ecma` over the script-block
  program. Their own concerns name the embedded language's for the compile's checks. A semantic key over the file's
  own tree is refused (`compile::documents::check_sides`).
- **The adapter kit** (`adapter/kit.rs`) runs one generic suite over every adapter, on the document each family's
  semantics read. It checks each adapter reads exactly the capabilities its registry entries support.

</details>

### Vue

`adapter/vue.rs` reads a `.vue` file as two documents (`embed`): the component's tree-sitter-vue tree, and one program
built from its `<script>` blocks and template expressions. A clause reads the program with
`ast.embedded: script-block`; a template rule reaches it with `embedded`.

<details>
<summary>How the program is built and read</summary>

The program comes from `vue::view`, via `script-block` (`embed/script_block.rs`): TypeScript for `lang="ts"`, TSX for
`lang="tsx"`, else JavaScript. Another language (`coffee`) embeds none. The program has its own grammar and text, with
segments mapping that text to the file's bytes.

| Concern | Vue |
|---|---|
| Documents | The component's tree; the program, in the dialect `ScriptBlock` built it in. Each has its own syntax errors: template errors leave a clause over a clean program complete |
| Scope, value, arguments | Ecma's, over the program. Template references resolve in the scopes Vue's compiler gives them; tokens the view writes (an interpolation's `(` and `);`) are not spelled (`embed::rewritten`) |
| Calls, format arguments | Refused, as for ECMAScript: vue-eslint-parser hands scripts to ESLint, which has neither |
| Suppressions | `eslint`: ESLint directives in `<script>` comments (`suppress::vue::script_directives`, the program's comments at the file bytes they copy), or eslint-plugin-vue's template comment directives (`vue/comment-directive`, `VueDirectives`) |
| Patterns | Vue's over the component's tree; the dialect's over the program |
| `languageOptions`, `when` | Ecma's keys |

</details>

### The Rust adapter

`adapter/rust.rs` reads Rust from tree-sitter-rust's syntax alone (`ast-grep:Rust`, tape tag 9), without types or
macro expansion. The Clippy pack (`packs/rust/`) ships the syntax ports that pass the
[conformance check](../conformance/README.md#the-gate); the rest stay candidates.

<details>
<summary>What each capability reads, and its oracle</summary>

The adapter does not read names inside macro token trees and `macro_rules!` bodies. Each concern is proven against a
recording of the Rust 1.99.0 toolchain under `fixtures/rust/` (`generate.py` beside each; run by hand, never in CI).

| Capability | Module | Reads | Oracle |
|---|---|---|---|
| `scope` | `scope/rust.rs`, `scope/usage/rust.rs` | rustc's lexical name resolution. Items are visible in their whole block or module (`extern` blocks too); `let` and pattern bindings from where they are made; generic and `for<'a>` lifetimes in their item or type; no local of an enclosing `fn`. Types, values, macros, lifetimes and labels are separate namespaces. A macro invoked in item position may define items, so a name nothing else binds there is not unresolved. `resolves.import` lists `::` paths, a read's head through `use` and `extern crate`, `self::`/`super::` normalised to `crate::` | rust-analyzer's SCIP occurrences, rustc's unused-name diagnostics (`fixtures/rust/scope`) |
| `value` | `value/rust.rs` | Literals and constant expressions as rustc evaluates them (any-width integers typed by suffix, floats, `char`, strings, byte and C strings), within the decode's depth cap. `follow` reads a `const` item of the file | rustc's values for every literal and expression case (`fixtures/rust/literal`, `fixtures/rust/value`) |
| `arguments` | `matcher/arguments/rust.rs` | Positional arguments | rustc's argument binding per case (`fixtures/rust/matcher/arguments`) |
| `calls` | `calls/rust.rs`, `Graph::link_rust` | A crate's functions by path, `Self::f`/`Type::f` through its `impl` blocks, `self.m()` in an inherent `impl`. Open: method calls on other values, trait dispatch, closures, function pointers, macros whose expansion may call a crate function | rust-analyzer's resolution of every call in a test crate (`fixtures/rust/calls`) |
| `formatArgs` | `literal/rust_format.rs`, `matcher/format_args/rust.rs` | `rustFormat` templates and style `rust`: the `format_args!` family as rustc binds it | rustc's diagnostics per format macro case (`fixtures/rust/matcher/format_args`) |

Rust refuses `languageOptions`, `resolves.imported`, and other families' grammars and format styles.

</details>

<details>
<summary>Clippy suppression</summary>

Suppression is `clippy` (`suppress/clippy.rs`), checked against `clippy-driver` over UI fixtures and the ripgrep and
tokio corpora (`fixtures/rust/suppress`).

- **Reads** rustc's and Clippy's lint-level attributes (`allow`, `expect`, `warn`, `deny`, `forbid`), outer and
  inner, including those a `cfg_attr` applies when its predicate holds. With `suppress.clippy.msrv`, it also reads
  the `#[clippy::msrv]` in force.
- **Per file only.** Levels set in another file (a crate root's `#![allow(..)]` over its modules) or on the command
  line are not seen.
- **`cfg` predicates** other than `all`, `any` and `not` over `clippy` are false.

</details>

### The SQL adapter

`adapter/sql.rs` reads SQL as SQLFluff 4.4.0 does, with one generic dialect for PostgreSQL, SQLite, MySQL and ANSI
files. It refuses `calls` and `formatArgs`.

<details>
<summary>The dialect, and what each concern reads</summary>

That dialect is `GrammarDialect::Sql`, the vendored tree-sitter-sql with Rerail's patches
(`native/grammars/tree-sitter-sql/PATCHES.md`). Where dialects disagree it reads PostgreSQL's; the pack declares the
rest as divergences.

| Concern | Reads | Oracle |
|---|---|---|
| `Adapter` | Names as PostgreSQL looks them up: naked identifiers fold to lower case, quoted ones are exact (`value::sql::identifier`). SQLFluff's `-- noqa` (`suppress/sqlfluff.rs`) | `fixtures/sql/suppress/sqlfluff.expected.json`: SQLFluff's reports with `noqa` ignored and honoured |
| `ScopeRules` | Query blocks (`ScopeKind::Query`, scope `query`) from `scope/sql.rs`, below. `resolves.unresolved` is RF01's report; `binds: { used: false }` is AL05's | SQLFluff's expected RF01 and AL05 findings (`packs/sql/upstream/sqlfluff/fixtures.expected.json`, pack test data: `npm run fixtures:pull`), with the declared ANSI-versus-PostgreSQL mismatches |
| `ValueRules` | Literals and names (`value/sql.rs`) only where PostgreSQL and SQLite agree. `type` and the key, member, reference and structural compare modes are refused | `fixtures/sql/scope/postgres.expected.json` (PostgreSQL 18.6 via libpg_query), `fixtures/sql/value/sqlite.expected.json` (SQLite 3.50.4) |
| `ArgumentRules` | An `invocation`'s arguments as PostgreSQL 18.6's parser binds them (`matcher/arguments_sql.rs`): by position, then by name. Signature markers, `syntactic` and `estree` are refused | `fixtures/sql/matcher/arguments/postgres.expected.json`: 50 calls, 49 matched by position, 1 asserted unreadable (`substring(s FOR n)`, whose start PostgreSQL supplies) |
| `SyntaxRules` | No literal grammar (`parse`), no `languageOptions`; `when` reads the document's options | |

A query block binds:

- its `FROM`/`JOIN` relations (`Form::Relation`);
- its `UPDATE`/`DELETE`/`INSERT` target (`Form::Target`);
- around `CREATE POLICY` or `ALTER POLICY`, the policy's table (`Form::Target`), seen by the queries in its `USING`
  and `WITH CHECK`, as SQLFluff's RF01 reads it on PostgreSQL;
- around a `WITH` statement, its CTE names (`Form::Cte`).

A qualified column's table part resolves to a relation or target of its block or an enclosing one; a CTE only through
a relation over it. The pack's ports read `syntax`, `value`, `compare`, `scope` and `suppression`. None reads
`arguments`; that capability's own oracle proves it.

</details>

<details>
<summary>Why calls and formatArgs are refused</summary>

`calls` and `formatArgs` are `refused` in `languages/sql.json`: SQL gives the engine nothing to read for them. A rule
reading either is invalid for `sql`, never `language-not-supported`. The module doc of `adapter/sql.rs` has the
evidence:

- **`calls`:** no SQLFluff rule follows a call into a routine. A routine body is a string literal of another language
  (`$$ ... $$`, PL/pgSQL), not parsed SQL. A `.sql` file has no workspace-resolvable call graph: no import or module
  links one file's routines to another's.
- **`formatArgs`:** no SQLFluff rule reads a format string, and ISO SQL has no format function. PostgreSQL's
  `format()` and SQLite's `printf()` are dialect extensions.

</details>

## The registry entry

Each language is one file, `languages/<id>.json`, so changing one language touches no other's file.

```json
{
  "format": "rerail.language/v1",
  "id": "blade",
  "status": "supported",
  "family": "blade",
  "dialects": [
    { "label": "blade", "variant": "Blade", "suffixes": [".blade.php"], "parser": "forte", "grammar": "native:forte", "tapeTag": 8 }
  ],
  "capabilities": { "scope": "supported", "value": "supported", "arguments": "supported", "calls": "supported", "formatArgs": "supported" },
  "suppressions": [],
  "embeds": [{ "language": "php", "via": "compiled-view" }]
}
```

`id` equals the file name and is what a code rule's `language` names. `capabilities` gives each of `scope`, `value`,
`arguments`, `calls` and `formatArgs` as `supported`, `refused` or `not-supported-yet`.

<details>
<summary>Field reference</summary>

| Field | Required | Meaning |
|---|---|---|
| `format` | yes | `rerail.language/v1` |
| `id` | yes | What a code rule's `language` names. Equals the file name |
| `status` | yes | `supported` or `planned` (see the lifecycle) |
| `family` | yes | The semantic family that reads it. Languages sharing one (`typescript` and `javascript` share `ecma`) share its suppressions |
| `dialects` | yes | The grammars its files are parsed with (below) |
| `runsOn` | no | Dialect labels its rules run on, its own first; default its own. A `typescript` rule also runs on JavaScript files |
| `capabilities` | yes | Per capability (`scope`, `value`, `arguments`, `calls`, `formatArgs`): `supported`, `refused` (no such thing or no plan; a rule using it is invalid) or `not-supported-yet` |
| `suppressions` | yes | Linter suppression comments its files honour (`noqa`, `nosec`, `eslint`, `oxlint`, `biome`, `phpstan`, `psalm`, `phpcs`, `clippy`, `sqlfluff`) |
| `embeds` | no | Embedded languages and how they are reached (`via`): Blade's PHP through the compiled view, Vue's script blocks. A rule runs on the program with `ast.embedded: "<via>"` and reaches it from the file's tree with `embedded` ([DSL reference](code-rule-dsl-reference.html#embedded)) |
| `fragments` | no | Grammars of a part of one of its dialects, which `.rr` rules name in `embed`, `sub_tape` and `parsed`: `{ "name": "annotation", "dialect": "python", "start": "expression" }` is a Python expression, parsed inside a written `( … )`. A name is unique among every dialect label and fragment ([decision 0082](final-shape/decisions/0082-embedded-grammar-names-sql-and-global-guests.md)) |

Each dialect has:

- **`label`**: the file language hosts and the store use.
- **`suffixes`**: lowercase, with the dot. Compound ones such as `.blade.php` win over shorter ones.
- **`variant`**, supported only: the engine's `GrammarDialect::<variant>`.
- **`parser`**, supported only: `tree-sitter`; `forte`, Rerail's port of Forte ([docs/blade-parser.md](blade-parser.md));
  `rr`, the rule engine's own parser of `.rr` files (decision 0061); or `bitlex`, a Bitlex grammar the transducer
  compiles, written in its tree-sitter grammar's ids, with tree-sitter for a file the grammar refuses. Validated; the
  engine picks the parser by dialect, not from this field (JSON's Bitlex parser: `tape::TapeImage::parse`).
- **`grammar`**, supported only: `vendored:<name>` (built by `build.rs`), `native:<name>` (a parser Rerail implements)
  or `ast-grep:<Language>`.
- **`tapeTag`**: required when supported; a `planned` dialect may declare one to reserve it. Persisted, unique across
  entries, never reused. Taken: Rust 9, SQL 10, Vue 12, rr 13 and JSON 14 (11 is free for a second SQL dialect).

</details>

<details>
<summary>Embeds, versioning and validation</summary>

**Embeds.** A supported entry may embed only supported languages. An `Embedder` builds at most one program per file
and `via`: a document derived from the file's (`tape::DerivedImage`), whose own tape the store keeps as a row beside
the file's (`file_documents`).

**Versioning.** A new optional field or value is a `rerail.language/v1` change. Renaming or removing a field, or
changing a value's meaning, needs `rerail.language/v2`.

**Validation.** The build and `test/engine/code-rule/language-registry.test.ts` fail when:

- ids, dialect labels, suffixes or tape tags repeat;
- a supported dialect lacks a variant, parser, grammar or tape tag;
- a planned language supports a capability or names `runsOn`;
- entries name different capabilities;
- `runsOn` names another family's dialect;
- `embeds` names an unknown language.

</details>

## Status lifecycle

A language moves from planned, through capabilities one at a time, to supported.

1. **Planned.** Dialects have suffixes but no grammar; every capability is `not-supported-yet`. Rules may name the
   language, so its packs can land first, but the engine reads none of it.
2. **Capabilities, one at a time.** A capability becomes `supported` only when its oracle suite passes: its
   `fixtures/` oracles and the conformance scores of code rules that read it.
3. **Supported.** The entry flips once its dialects have a grammar, parser and tape tag and its adapter exists. The
   build fails until each does.

<details>
<summary>What happens to a rule the engine cannot read yet</summary>

A rule of a planned language, or one reading a `not-supported-yet` capability, is refused as
`language-not-supported` and kept inactive:

- Compiling the rule fails with the stable code `language-not-supported` (the message starts with it and names the
  language), never a schema error.
- The catalog loads the rule but keeps it inactive. Audits skip it, never fail on it, and do not skip a pack that
  holds it.
- The rule APIs (`GET /api/repository/rules`, `GET /api/rerail-rules`, `GET /api/library-rules`) and activity API
  (`GET /api/rules/activity`, `measured: false`) return it with `inactive: { code, reason }`.
- Audits parse none of a planned language's files: its suffixes select no dialect, so regex clauses read them as
  `text`.
- Pack tests report its rules as `not supported yet` without failing
  ([validation contract](pack-format.md#the-validation-contract)).

</details>

## The test-only language

`native/test-languages/exemplar.json` is a planned language only tests see, so supporting a real language never
touches a test.

<details>
<summary>How tests use it</summary>

A test that needs "a planned language" names it. It has family `exemplar`, one dialect `exemplar` (suffix
`.exemplar`), every capability `not-supported-yet`, no suppressions and no tape tag. It is in no package and never in
the product:

- **Not in the registry.** `languages/` and the generated host module omit it; `npm run generate:languages` reads
  `languages/` alone.
- **Test builds only.** `native/build.rs` validates it like any entry, and against the registry for repeated ids,
  labels, suffixes and tape tags. It emits it only as `#[cfg(test)]` items: `PLANNED_LANGUAGES` lists it only in test
  builds (`TEST_PLANNED_LANGUAGES` names languages only tests plan). It is no part of the engine's identity.
- **Mocked in host tests.** The product-build native binding does not know it. Host tests mock the generated module
  with `exemplarRegistryModule()` and the native module with `nativeRefusingExemplar`
  (`test/engine/support/exemplar-registry.ts`, `exemplar-engine.ts`), which refuses only exemplar rules and passes
  the rest to the real binding.
- **Real refusal.** A native test checks the engine's `language-not-supported` refusal. So does
  `test/engine/code-rule/language-not-supported-engine.test.ts`, unmocked, by compiling a rule of a product language
  the registry still plans.

</details>

<details>
<summary>Test-only embedding</summary>

The test-only HTML grammar embeds TypeScript, TSX and JavaScript through `Via::TestScript` (`language::test_embeds`,
`native/src/code_rule_dsl/embed/test_script.rs`). Its program is an HTML file's scripts, then its `on*` handlers. Its
tests (`embed/tests.rs`) cover the embedder loop, derived documents and their dialects stored apart from the file's
tape, and scope and usage over a program mapped to the file out of order. The generated `Via` and `with_embedder!`
carry it only under `#[cfg(test)]`.

</details>

## Adding a language

Register the language as planned, add its grammar and adapter, then flip capabilities as each is proven.

1. **Register it.** Add `languages/<id>.json` with `status: "planned"`: family, dialect labels and suffixes, every
   capability `not-supported-yet`, suppressions and embeds. Give each dialect the `tapeTag` it reserves (Rust 9, SQL
   10, Vue 12, rr 13 and JSON 14 are taken; 11 is free for a second SQL dialect). Run `npm run generate:languages` and commit the
   regenerated `src/engine/code-rule/code-rule-language-registry.generated.ts`. Rules and packs can land now.
2. **Add its grammar.** Vendor or pick one (add a vendored one to `GRAMMARS` in `native/build.rs`). Flip the entry to
   `supported` with each dialect's `variant`, `parser` and `grammar`. The build fails until `adapter::<Family>` exists.
   Name the language under `ast.languages` in `native/src/code_rule_dsl/capabilities.json` with its grammar (a
   version, or `native:<id>` for a grammar the engine builds in, as `rr` has), or policies cannot target it; a test
   holds those keys equal to the registry's supported languages.
3. **Write its adapter** in its own file, registered by one `mod` and one `pub(crate) use` line in
   `native/src/code_rule_dsl/language/adapter.rs`. A new family's type implements `Adapter` and each concern trait,
   or `Refused` for a concern the language will not have.
4. **Add missing suppression kinds** to the engine, the host schema, the entry and the adapter (below).
5. **Add an embedding** (`embeds` with a new `via`): `native/src/code_rule_dsl/embed/<via>.rs` implementing
   `Embedder`, plus one `mod` and one `pub(crate) use` line in `embed.rs`. Nothing else in the engine changes.
6. **Add a kit test:** one `#[test]` function in `native/src/code_rule_dsl/language/adapter/kit.rs` calling
   `kit::<Family>(&Expect { .. })` with the family's expectations.
7. **Flip capabilities** to `supported` only when implemented and proven by their oracle suite; mark ones the language
   will never have `refused`. Flip `status` last.

<details>
<summary>Steps 2 and 3: generated code and concern traits</summary>

For step 2, the build generates `GrammarDialect::<Variant>`, `Family::<Family>`, `Family::of`, `honours` and the
`with_adapter!` arm; no enum is edited by hand. For step 3, a new family implements:

- `ScopeRules`;
- `ValueRules`, with `const LANG: Family = Family::<Family>` and the hooks `truthy`, `type_name`, `equals`,
  `order`, `relation`, `numeric`, `integer` and `to_str`;
- `ArgumentRules`;
- `CallRules`, with `const NAMES: Option<Family>`, the family whose namespace its call names live in;
- `FormatArgRules`;
- `SyntaxRules`.

A new dialect in an existing family needs no per-dialect table: ECMAScript's scope reads each dialect's grammar
(`scope::ecma::script`).

</details>

<details>
<summary>Step 4: where a suppression kind goes</summary>

Each missing suppression kind needs:

- one row in `suppression_kinds!` in `native/src/code_rule_dsl/suppress.rs`
  (`Variant key: Spec = linter::spec, "detail";`), with its parser in `suppress/<linter>.rs`;
- one entry in `codeRuleSuppressSchema` in `src/engine/code-rule/code-rule-schema-semantic.ts`;
- its name in the registry entry's `suppressions`;
- its use in the adapter's `suppresses`.

</details>

<details>
<summary>Step 5: what an embedding provides</summary>

`build` returns one `embed::Program` or none: its dialect, its tree over its own text, the `SegmentMap` mapping that
text to the host's bytes (copied or written over them, in any order), and the host syntax errors it reads through.
The engine handles the rest:

- **Storage.** The program is a derived document (`tape::DerivedImage`): text, source map, host errors and its own
  tape, in its grammar's codes, checked like every tape. The store keeps that tape as its own row, related to the
  file's by `file_documents` ([store](content-addressed-workspace-store.md)).
- **Access.** A clause reads it with `ast.embedded: <via>`; a host rule reaches it with `embedded`; a program node
  reaches the host with `host` (`matcher::embedded`).
- **Positions.** Witnesses, captures and fixes land at the file bytes they stand for. Each document is an
  `origin::Origin` into its source, and `origin::InFile` places nodes and ranges in the file (fixes only over
  copied bytes). Nothing else maps a position.
- **Ranges.** A node covers its pieces' file ranges (`embed::segments::FileRanges`), except where the program is
  out of file order and joined only over white space. A single span reports the range it starts in. Positions are
  half-open: a boundary belongs to the range starting there.
- **Syntax errors.** Each document has its own, as file ranges (`soundness::SyntaxErrors::of_doc`). A program with
  empty `host_errors` is complete whatever the host's errors.
- **Calls** read the program whose dialect's calls join the graph (`calls::calls_document`).

The embedding family's adapter (step 3) names the embedded family's concerns, as Blade's does PHP's.

</details>

## Start a pack

A pack is a directory `packs/<id>/` that passes the [validation contract](pack-format.md#the-validation-contract).

Models to copy:

- **Ported language packs:** `packs/python/`, `packs/typescript/`.
- **Framework packs over a supported language:** `packs/node/`, `packs/next/`, `packs/react/`. New ports start in
  `candidates/rules/` and ship once they pass the [conformance check's gate](../conformance/README.md#the-gate).
- **Smallest pack:** `fixtures/packs/valid/`, with one example-proven rule and one ported rule.
- **Planned-language pack:** `fixtures/packs/exemplar/`.

### A pack for a supported language

A Laravel pack with one code rule proven by examples. The skeleton passes as written.

```sh
mkdir -p packs/laravel/rules
cp LICENSE packs/laravel/LICENSE
```

`packs/laravel/package.json`:

```json
{
  "name": "@neralabs/rerail-pack-laravel",
  "version": "0.1.0",
  "license": "Apache-2.0",
  "files": ["pack.json", "rules", "LICENSE", "README.md"],
  "peerDependencies": { "@neralabs/rerail": ">=0.1.0-0 <0.2.0" },
  "peerDependenciesMeta": { "@neralabs/rerail": { "optional": true } },
  "publishConfig": { "access": "public" }
}
```

`packs/laravel/pack.json`:

```json
{
  "format": "rerail.rule-pack/v1",
  "id": "laravel",
  "name": "Laravel",
  "category": "framework",
  "covers": "Laravel conventions: no debugging helpers left in application code.",
  "reason": "matches Laravel",
  "match": { "any": [{ "files": ["artisan"], "label": "an artisan file" }] },
  "rules": [
    {
      "id": "laravel-no-dd",
      "document": "rules/laravel-no-dd.json",
      "title": "No dd() or dump() calls",
      "group": "Laravel",
      "examples": {
        "caught": { "path": "app/Http/Controllers/UserController.php", "text": "<?php\nfunction show($user) {\n    dd($user);\n}\n" },
        "passes": { "path": "app/Http/Controllers/UserController.php", "text": "<?php\nfunction show($user) {\n    return view('user', ['user' => $user]);\n}\n" }
      }
    }
  ]
}
```

`packs/laravel/rules/laravel-no-dd.json` is a code rule (`rerail.policy/v1`, see the
[code-rule DSL reference](code-rule-dsl-reference.html)). Its `id` matches `pack.json` and its file name. Rule ids
are lowercase letters, digits and dashes, in one style for every pack ([rule ids](pack-format.md#packjson)).

```json
{
  "format": "rerail.policy/v1",
  "id": "laravel-no-dd",
  "revision": 1,
  "mode": "steer",
  "description": "A call to Laravel's dd() or dump() debugging helper.",
  "scope": { "include": ["app/**/*.php", "routes/**/*.php"], "exclude": [] },
  "clauses": [
    {
      "id": "no-dd",
      "forbid": { "ast": { "language": "php", "rule": { "kind": "function_call_expression", "has": { "field": "function", "regex": "^(dd|dump)$" } } } },
      "violation": { "anchor": "witness.span", "code": "LARAVEL-NO-DD", "message": "Remove dd()/dump() before committing" }
    }
  ]
}
```

Add a `README.md` saying what the pack covers. Then record the new workspace and run the contract:

```sh
npm install --package-lock-only --ignore-scripts   # packs/* are npm workspaces: package-lock.json lists each
npx vitest run test/engine/catalog/packs.test.ts -t "pack laravel"
```

Add a changeset (`patch`, "Add the Laravel pack: …"), unless the pack starts private (`"private": true` in its
`package.json`, no `publishConfig`): a private pack is never published and takes no changeset until it goes public
([package.json](pack-format.md#packagejson)). Every later change to a published pack adds one too;
[versioning](pack-format.md#versioning-and-release) says which bump (`patch` until 1.0.0):

```sh
npx changeset   # pick @neralabs/rerail-pack-laravel, choose the bump, write the summary; then rename it added-laravel-pack.md
```

Every rule needs proof: `examples` (one caught, one that passes, each `path` inside the rule's scope) or a port entry
(below). Each check and rule is one test. A failure names the rule and file, for example
`laravel-no-dd: the caught example (app/Http/Controllers/UserController.php) produces no LARAVEL-NO-DD violation`.

### A pack for a planned language

Use the same files, with rules whose `language` is `planned` or that read a `not-supported-yet` capability.

The pack lands: its checks pass, and each such rule's test is skipped as `not supported yet` with the engine's reason,
for example ``/clauses/0/forbid/ast/language: `exemplar` is not supported yet``. Write `examples` now: checks 3 and 4
run them once core reads the language. Any other compile error fails check 1 today.

`fixtures/packs/exemplar/` is a complete example for the test-only planned language.

### A ported pack

A pack that reproduces another tool's rules adds validation material, which `files` never publishes. Each ported tool
is a tool-pack, `upstream/<tool>/`, with its own pack data ([pack data](pack-format.md#pack-data)):

1. **`conformance.json`** ([schema](pack-format.md#conformancejson-divergencesjson-upstream)): the family, declared
   capabilities, pinned `tools`, and one entry per ported rule.
2. **`upstream/<tool>/`**: a generator kept beside the data, a recipe under `scripts/pack-data/recipes/`, and
   `gates.yaml`, the upstream manifest ([conformance/README.md](../conformance/README.md#declared-upstream-facts-gatesyaml)).
3. **`licenses/<project>/`**, one per upstream repository: its licence verbatim, and `UPSTREAM.md`
   ([UPSTREAM.md](pack-format.md#upstreammd)).
4. **`divergences.json`**: each line where Rerail and the tool disagree, with its kind and checked evidence
   ([divergences](../conformance/README.md#divergences)). Start with `[]`; add only disagreements you have checked.
5. **Optional samples:** `upstream/<tool>/samples.lock` and `SAMPLES.md`, real-code samples made with the sample
   builders and published like test data ([docs/upstream.md](upstream.md#samples)).

<details>
<summary>What each file holds</summary>

**`conformance.json`**, per ported rule: its `upstream` tool and rule, `id` (the rule's own), `policy` (its
document), how findings compare (`line` or `line+column`) and the capabilities it exercises.

- A port's clauses report the upstream rule's code.
- Every declared capability needs a ported rule or an `exemptCapabilities` entry with evidence.
- New ports start as candidates under `candidates/rules/`
  ([candidates and promotion](../conformance/README.md#candidates-and-promotion)).

**`upstream/<tool>/`:** each recipe is one module per pack, listed in `scripts/pack-data/recipes/index.ts`.

- **Generators.** `packs/python/upstream/ruff/generate.py` is one. ESLint-based ones share
  `scripts/pack-data/eslint-expected.mjs`, which also writes rename variants under `fixtures/renamed/`.
- **Expected findings.** `npm run fixtures:derive -- --pack <id>` copies the tool's tests into `fixtures/` and
  writes `fixtures.expected.json`: the pinned tool's findings on them (`expectedFindingsSchema` in
  `conformance/languages/pack.ts`, each unit's upstream origin in `source.units`).
- **Publishing.** That derived test data is never in git. It is staged into `upstream/<tool>/publish/`
  (`npm run fixtures:stage -- --pack <id> --tool <tool>`) and published by the publish workflow, which commits the
  tool-pack's `test-data.lock` ([docs/upstream.md](upstream.md#publishing-pack-data)).
- **Provenance.** The generator writes the tool's `provenance.json` and `LICENSES.md` last, from the pack's
  `licenses/<project>/UPSTREAM.md` records ([docs/upstream.md](upstream.md#what-the-generators-write)).
- **Authored cases.** List each in the tool's own `UPSTREAM.md`, or keep a corpus with its ground truth under
  `upstream/<tool>/authored/`.
- Get other packs' data with `npm run fixtures:pull`.

**`licenses/<project>/UPSTREAM.md`** has:

- a `License: <SPDX expression>` line;
- a `Source` table: repository, tag, 40-hex commit, licence;
- a `Taken from it` table mapping each pack-relative glob of copied files to its upstream path and copy method;
- a `Not copied` line per upstream path under a licence the allowlist refuses.

Add the tool's licence to `package.json`'s `license` (`Apache-2.0 AND MIT` for an MIT tool), and annotate the rule
documents in `REUSE.toml` with that expression.

</details>

Score one rule, listing every false positive and false negative, then run the contract and the gate:

```sh
npm run conformance:languages -- --pack <id> --rule <port id> --fixtures
npx vitest run test/engine/catalog/packs.test.ts -t "pack <id>"
npm run conformance:languages -- --check --pack <id>
```
