# Engine gaps found while writing the TanStack Query practice rules

Found while writing the `tanstack-query-practice-*` rules and running them over the corpus of
[README.md](README.md#the-corpus) with `rerail-engine test-rr` and `run-rr` (debug and optimized builds, 2026-11-24).
Each gap names its repro, the rules it affects and the workaround they use. Two engine branches made for this work
address the first three: `engine-ts-lanes` (`409baf3a61`: a dialect's run reads its language's other dialects' files
as text) and `engine-tsconfig` (`27c35a7edb`: core.rerail's tsconfig and package_export inputs are made, and an
ECMAScript document's module is its path). The last column says what each does for the gap; "not confirmed" marks
what was inferred and not run.

| # | Gap | Rules affected | Status |
|---|---|---|---|
| 1 | `tsconfig_for`/`tsconfig_path` have no producer, so tsx/names' cross-file `target`/`sym_of` are refused | all 9 cross-file rules (each resolves imports itself) | Closed by `engine-tsconfig` |
| 2 | Every TypeScript document's module is `""` | server-action-as-query-fn; any rule using tsx/names symbols | Closed by `engine-tsconfig` |
| 3 | `.ts` and `.tsx` files run in separate lanes and never join | all cross-file rules; false reports in cache-key-matches-no-query | Partly addressed by `engine-ts-lanes` (text only); open for joins that read the other file's tree |
| 4 | A world read whose path is derived from `doc` or `file` is refused | module-query-client-in-ssr, query-client-factory-called-in-render, infinite-and-plain-share-key | Open (by design, decision 0065 §2a); gap 1's producers remove the need for tsconfig reading |
| 5 | The same world read fed by a recursive relation gives no rows, or fails on statistics | module-query-client-in-ssr | Open |
| 6 | A disjunction binding a variable used by a functor in an atom argument fails to plan | module-query-client-in-ssr | Open |
| 7 | `run-rr` into an existing out directory returned the previous program's results | module-query-client-in-ssr | Open |
| 8 | `rerail-engine csv <out>` reads only the root lane's `outputs.rro` | every rule's corpus run | Open in the engine; the corpus script reads every lane |
| 9 | `earlier` and `visit_index` have no rows for TypeScript documents | fetch-without-ok-check | Open |
| 10 | `es_global` holds ECMAScript globals only, no browser or Node globals | fetch-without-ok-check | Open |
| 11 | `match_all` positions bound only in a rule body give one row with an empty value | query-fn-input-hidden-from-key | Open |
| 12 | `!fn_param(f, _, i)` on a node of another file held for parameters that exist | query-fn-input-hidden-from-key | Not confirmed |
| 13 | A negation after `key_static` in a body is ignored | infinite-and-plain-share-key | Open |
| 14 | `package_field` (and, by one report, `package_for`) unbound in `.rerail` runs | infinite-and-plain-share-key | Not confirmed: same-key-different-query-fn reads `package_for` and it works |
| 15 | A message cannot hold `{` or `}` literally | infinite-and-plain-share-key | Open |
| 16 | "content store hard limit rejects new payloads" on the largest repos | infinite-and-plain-share-key | Open |
| 17 | `.scope exclude` removes a file's facts, not only its findings | cache-key-matches-no-query | Open (may be intended; then document it) |
| 18 | `match(re, n)` is a full match, which the language guide's examples do not make plain | dehydrated-state-json-stringify | Docs |
| 19 | A demand set recursive with a cross-file lookup costs about 4x | query-fn-input-hidden-from-key | Cost note |
| 20 | Engine panic "rows past 96 chunks" (`blocks/src/rows.rs`) on supabase | query-fn-input-hidden-from-key | Open |
| 21 | `run-rr` and `test-rr` do not apply `rerail-ignore` to a `.rerail` rule's findings | query-data-copied-to-state (message) | Not confirmed for audits |

---

## 1. `tsconfig_for` and `tsconfig_path` have no producer

**Symptom.** Any rule whose cone reaches tsx/names.rerail's cross-file `target` or `sym_of` (or tsx/node-resolve) is
refused by `test-rr` and `run-rr`: "tsconfig_for is read, and nothing derives it or binds it: the corpus runner has no
producer of it". A program may not derive it either: adding `tsconfig_for(p, "") :- doc(_, p, _).` gives
"tsconfig_for is core.rerail's: a program reads it and cannot declare or derive it". Every writer met it, even for an
import of a bare package.

**Repro.** A rule that `#include "tsx/names.rerail"` and reports `X(n) :- sym_of(n, _).`, on any fixture with a
`tsconfig.json`.

**Rules affected and workaround.** Every cross-file rule resolves imports itself, keyed on file paths: relative
specifiers with TypeScript's extensions and `index` files, `@/` and `~/` (and `#/`) by layout, and, in
module-query-client-in-ssr and query-client-factory-called-in-render, tsconfig/jsconfig `paths` found by a `dir_entry`
walk (the `tsr_` shim). dehydrated-state-json-stringify and same-key-different-query-fn avoid resolution: they read
an import's binding (`import_binding`, `ref_site`) in the file only. The six resolvers are listed in
[NOTES-tanstack-query.md](NOTES-tanstack-query.md#library-candidates).

**Status.** `engine-tsconfig` (`27c35a7edb`) makes `tsconfig`, `tsconfig_for`, `tsconfig_path` and `package_export`
in the repository walk that makes `package*`: each folder's tsconfig.json (else jsconfig.json) over its `extends`
chain, `baseUrl`, `paths` targets, and a document's nearest configuration. With it the rules can drop their own
resolvers for tsx/names' `target`/`sym_of`; that rewrite is not done.

## 2. Every TypeScript document's module is `""`

**Symptom.** `doc(d, p, m)` gave `m = ""` for every `.ts`/`.tsx` file, so tsx/names' `resolve_base`, `target`,
`exports` and `@qname(m, x)` collapse every workspace module to `""`: every file's `x` is the one symbol `#x`.

**Repro.** Two `.ts` files and `.violation E(at: node) "e"` / `E(p) :- doc(d, _, ""), program(d, p).`: it fires on
both.

**Rules affected and workaround.** server-action-as-query-fn keys symbols on paths; the other resolvers do the same.

**Status.** Closed by `engine-tsconfig`: an ECMAScript or Vue document's module is its path without its extension
(`src/ui/button#Button`).

## 3. `.ts` and `.tsx` files run in separate lanes

**Symptom.** A `.language typescript` rule runs once per dialect (`.ts`, `.tsx`, `.js`), and a cross-file join in one
run sees only that dialect's files. A `.tsx` component never sees a hook, key factory, fetcher, Server Action or
QueryClient factory in a `.ts` module, nor the reverse. `run-rr` also writes one `outputs.rro` per lane (root for
`.ts`/`.js`, `tsx/` for `.tsx`; gap 8).

**Repro.** Fixtures `a.ts` and `b.tsx`; `X(n) :- kind(n, "program"), docof(n, _), doc(_, "a.ts", _).` fires only in
`.ts` files, and the same probe for `"b.tsx"` only in `.tsx` files. For cache-key-matches-no-query (`test-rr`,
`exactly`, no expectations): `src/queries.ts` = `export const todoKeys = { all: ["todos"] as const };`,
`src/TodoButton.tsx` = `<button onClick={() => qc.invalidateQueries({ queryKey: ["todos"] })}/>` is reported at
`TodoButton.tsx:2`.

**Rules affected.** The cases mark the lines it hides `// cross-lane` (11 lines in 9 rules' cases) and do not expect
them.

| Rule | Effect |
|---|---|
| cache-key-matches-no-query | False reports: 37 of 40 dev-subset matches (openship 28, umami 4, nodedr 4, ever-teams 1). Starts off until this is closed |
| server-action-as-query-fn | Misses 2 of 6 real sites the text scan found (workout-cool `add-exercise-modal.tsx`, `users-table.tsx`); a `'use client'` `.tsx` component calling a `.ts` action is the common App Router shape |
| query-data-copied-to-state | Misses every component using a custom query hook in a `.ts` file (all of umami's and ever-teams'); recall unknown |
| query-fn-input-hidden-from-key | 8 of 224 key-factory query sites in formbricks and ever-teams unresolved (`.tsx` site, `.ts` factory) |
| module-query-client-in-ssr | Misses spliit `src/trpc/client.tsx:39` (a `.ts` factory called at import) |
| query-client-factory-called-in-render | Misses the commonest layout (factory in `lib/query-client.ts`, provider in `providers.tsx`); the pack example keeps both in `.tsx` |
| fetch-without-ok-check, same-key-different-query-fn, infinite-and-plain-share-key | Miss pairs split across the lanes; no corpus number (both fetch matches and the Infisical pair are same-lane) |

set-query-data-mutates-cache and dehydrated-state-json-stringify read one file at a time and are not affected.

**Status.** `engine-ts-lanes` (`409baf3a61`) makes each dialect's run read the other dialects' files in scope as text
documents: `doc`, path, source text and lines, no tree; a finding in another dialect's file is dropped by the run that
read it as text. From its description (not confirmed by a run of these rules on it), that closes the gap for a join
that reads only another file's path or text, and not for these rules, whose cross-file halves read the other file's
tree (declarations, exports, string literals, calls). Each would need either a tree-level join across dialects or its
cross-file half rewritten over text. cache-key-matches-no-query is the nearest to text: it asks whether a key root
occurs as a string anywhere, which a search of the other files' lines could answer.

## 4. A world read whose path is derived from `doc` or `file` is refused

**Symptom.** A relation that makes a world request (`file_line`, `dir_entry`) may not read a base relation: deriving
the path to read from `doc` or `file` fails with "the cone of requests reads base relation doc|file: a load answers
exports only (decision 0065 §2a)".

**Repro.** `x(f) :- file(f), suffix("tsconfig.json", f). y(t) :- x(f), file_line(f, _, t).`, or
`p(f) :- doc(_, x, _), package_for(x, d), f = @join(d, "tsconfig.json"). q(l) :- p(f), file_line(f, _, l).`

**Rules affected and workaround.** module-query-client-in-ssr and query-client-factory-called-in-render find
configuration files by a `dir_entry` walk from the root; infinite-and-plain-share-key takes packages and aliases from
the layout. `file_line` and `dir_entry` do give rows in `test-rr` and `run-rr`, for literal and walked paths.

**Status.** Open, and by design; gap 1's producers remove the need to read tsconfig files from a rule.

## 5. The same world read fed by a recursive relation gives no rows

**Symptom.** Before gap 4's refusal appeared, a `file_line` read fed by a recursive relation over `doc` paths
(`anc(d, @dir(a)) :- anc(d, a), …`, then `file_line(@join(dir, cfg), _, l)` in a derived relation) silently gave no
rows, while the identical `file_line` atom in a `.violation` clause's body gave rows; split into its own relation it
failed with "no statistics for mqc_anc.c0". The runner should refuse it the same way every time.

**Rules affected.** module-query-client-in-ssr (uses the `dir_entry` walk instead).

## 6. A disjunction binding a functor's argument fails to plan

**Symptom.** `(cfg = "a" ; cfg = "b"), file_line(@join(dir, cfg), …)` fails: "unplaced literals [\"@equals($cfg,
@set(…)) = 1\", \"$rr_t13 = @join($dir, $cfg)\"]".

**Rules affected and workaround.** module-query-client-in-ssr: a fact relation for the names works.

## 7. `run-rr` into an existing out directory returned stale results

**Symptom.** `run-rr` into an out directory a previous run had used returned that program's results for a changed
program (0.06 s). A fresh out directory gives the right ones.

**Rules affected.** Corpus runs of module-query-client-in-ssr; every corpus run now uses a fresh directory.

## 8. `rerail-engine csv <out>` reads only the root lane

**Symptom.** `run-rr` writes one `outputs.rro` per language lane (root for `.ts`/`.js`, `tsx/` for `.tsx`), and `csv`
reads only the root, so a run looks as if `.tsx` files were never parsed.

**Repro.** A directory with `src/a.ts` and `src/b.tsx`, the rule `A(n, "p") :- kind(n, "program").`, `run-rr` then
`csv <out>`: only `src/a.ts` is listed.

**Rules affected and workaround.** Every corpus run; the corpus script reads every lane's `outputs.rro`.

## 9. `earlier` and `visit_index` have no rows for TypeScript documents

**Symptom.** core.rerail documents both as given, but `.violation V(at: node) "v"` / `V(t) :- kind(t,
"throw_statement"), visit_index(t, _).` reports nothing on a `.ts` file with a `throw`; the same with `earlier(c, t)`
for a call before the throw.

**Rules affected and workaround.** fetch-without-ok-check orders nodes by `line`/`col`.

## 10. `es_global` has no browser or Node globals

**Symptom.** tsx/es-globals.rerail's `es_global` holds ECMAScript globals only, so `global_ref(f, "fetch")` and
`global_ref(f, "window")` never hold.

**Rules affected and workaround.** fetch-without-ok-check takes "a `fetch` nothing in the file declares"
(`fwoc_global`).

## 11. `match_all` positions bound only in a rule body

**Symptom.** `match_all("/", p, o, st, en)` used only to bind its positions inside a body (not the head) produced one
row with an empty root instead of one row per slash; the same call with the positions in the head works.

**Repro.** `qfh_root(d, substr(p, 0, en)) :- doc(d, p, _), match_all("/", p, o, st, en), o >= 0, st >= 0.` gives
`""` for `src/a/b.ts`; expected `src/`, `src/a/`.

**Rules affected and workaround.** query-fn-input-hidden-from-key walks directories with an `@dir` recursion.

## 12. `!fn_param` on a node of another file (not confirmed)

**Symptom.** `!fn_param(f, _, i)`, with `f` a node of another file reached through the rule's import resolver and `i`
bound from `call_arg`, was true for parameters that exist; negating a positive helper `qfh_has_param(f, i)` works. A
same-file and a simple cross-file repro did not reproduce it, so the trigger is narrower (possibly negation of a
library relation on a node reached through a recursive cross-file derivation).

**Rules affected.** query-fn-input-hidden-from-key (uses the helper).

## 13. A negation after `key_static` is ignored

**Symptom.** `!r(o)` holds where `r(o)` is true when the positive part of the body joins through
eslint-a-common's `key_static`.

**Repro.** `test-rr` reports line 2, which it must not; with `text(n, "queryKey")` instead of `key_static(n,
"queryKey")` only line 3 is reported:

```ts
// a.ts
declare function f(o: object): void
f({ queryKey: 1 })
const x = { queryKey: 2 }
```

```
#include "tsx/eslint-a-common.rerail"
.decl ma(o: node)
ma(o) :- kind(o, "object"), child(al, o, 0), kind(al, "arguments").
.decl mq(o: node)
mq(o) :- kind(p, "pair"), field(p, "key", n), key_static(n, "queryKey"), child(o, p, _), kind(o, "object").
.violation V(at: node) "object not a first argument"
V(o) :- mq(o), !ma(o).
```

**Rules affected and workaround.** infinite-and-plain-share-key reads the key's text (`tqis_key_name`).

## 14. `package_field` unbound (not confirmed)

**Symptom.** infinite-and-plain-share-key reported `package_for`, `package_field` (name, dependencies) and the
tsconfig inputs unbound in `.rerail` runs, and took package boundaries from the layout (`apps/*`, `packages/*`, …).
same-key-different-query-fn reads `package_for` and it works, and `engine-tsconfig` describes `package*` as made by
the repository walk, so the open part is probably `package_field` alone; not rerun.

## 15. A message cannot hold `{` or `}` literally

**Symptom.** A brace in a `.violation` message is read as a column ("no column  pages, pageParams"); no escape was
found.

**Rules affected and workaround.** infinite-and-plain-share-key words its message without braces.

## 16. The content store's hard limit on the largest repositories

**Symptom.** On the local runner, Infisical, supabase and cal.diy fail: "content store hard limit rejects new
payloads".

**Rules affected.** infinite-and-plain-share-key (its run on those three did not finish; a grep stood in).

## 17. `.scope exclude` removes a file's facts, not only its findings

**Symptom.** A file excluded by scope contributes no facts to other files' joins. A generated client excluded by scope
would hide its keys and make every invalidation of them a false report.

**Repro.** Scope-exclude `**/generated/**`, put `const b = "y"` there, and a rule that needs `str(_, "y")` derives
nothing.

**Rules affected and workaround.** cache-key-matches-no-query keeps generated, test and story files in scope and
filters its findings in Datalog (`ckm_quiet_doc`). If this is intended, the pack format should say so: a cross-file
rule cannot use `.scope exclude` for files it must read.

## 18. `match(re, n)` is a full match

**Symptom.** `match("<[A-Za-z]", t)` never holds on a template's text; it needs `(?s).*…​.*`. Not a bug, but the
language guide's examples read like a search.

**Rules affected.** dehydrated-state-json-stringify (uses the anchored form).

## 19. Cost of a recursive demand set

**Symptom.** A demand set for a strip helper (`qfh_sw`) recursive with the factory lookup made one strongly connected
component that cost about 4x the plain `deparen`-based strip (15.8 s against 3.9 s local CPU on a 324-file `src/`).
The whole rule: 13.8 s local and 2.3 s cross-file CPU on those 324 files with the debug engine.

**Rules affected.** query-fn-input-hidden-from-key (uses the plain strip).

## 20. Engine panic "rows past 96 chunks" on the largest repository

**Symptom.** query-fn-input-hidden-from-key over supabase (`50bab5657f`) runs for about 352 s on the optimized engine
(at load ~30 on 18 cores) and then a CPU worker panics; the run exits 1 with no findings:

```
thread 'rerail-cpu 7' panicked at native/crates/blocks/src/rows.rs:197:13:
rows past 96 chunks
rerail-engine: host stream strata: a job panicked
```

`Rows::try_reserve` asserts that a relation never needs more than `DIRS` (96) chunks; one relation of this rule's
program grows past that on supabase (9,095 TypeScript and JavaScript files) instead of the run reporting a resource failure.
It is the only failed run of the pack's 231 rule-by-repo runs over the 21 corpus repos that use TanStack Query; the
same rule finishes on civitai (137 s) and Infisical (102 s). Reproduced on the rule's current revision (with the
credential-name exemption): the same panic after 265 s.

**Repro.**

```
mkdir -p /tmp/qfh-panic/tmp && TMPDIR=/tmp/qfh-panic/tmp nice -n 19 ./target/profiling/rerail-engine run-rr \
  packs/tanstack-query/rules/tanstack-query-practice-query-fn-input-hidden-from-key.rerail \
  <corpus>/supabase__supabase /tmp/qfh-panic/out --memory-mb 4096
```

or `/tmp/rer133/corpus-local.sh tanstack-query tanstack-query-practice-query-fn-input-hidden-from-key
supabase__supabase` (the corpus clone at the README's pinned commit).

**Rules affected.** query-fn-input-hidden-from-key: on supabase it reports nothing. A relation that outgrows its
block should fail the run with a resource message (or spill), not panic a worker.

## 21. `rerail-ignore` is not applied to `.rerail` findings in `run-rr` and `test-rr`

**Symptom.** A `// rerail-ignore <rule-id> -- why` comment on the line above a finding of a `.rerail` rule leaves the
finding in `run-rr`'s output and fails a `.test cases exactly` that expects it silenced. docs/rule-scoping.md says
every code rule answers to `rerail-ignore`; the `rr` language adapter (`native/src/code_rule_dsl/language/adapter/rr.rs`)
lists no comment kinds, and nothing in `run-rr` reads the directive. Whether the audit pipeline applies it to `.rerail`
findings later was not checked.

**Repro.** In query-data-copied-to-state's cases, put `// rerail-ignore tanstack-query-practice-query-data-copied-to-state
-- kept on purpose` on the line above `const [opening] = useState(score)` in a component whose `score` is query data,
and run `rerail-engine test-rr` on the rule: the line is still reported.

**Rules affected.** query-data-copied-to-state told the agent to "say so in a comment and keep it"; no comment
silenced it, so the message now advises the prop-seeded child component, which the rule does not report. A message
should name `rerail-ignore` only once it is shown to silence a `.rerail` rule.
