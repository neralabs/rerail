# Engine gaps found while writing the Prisma practice rules

Found while writing the `prisma-practice-*` rules and running them with `rerail-engine test-rr` and `run-rr` (debug
and optimized builds of this branch's engine, 2026-11-24). Each gap names the rules it affects and how they work
around it. Two engine branches made since close some of them, without a rerun of these rules on them: the engine-ts-lanes
branch (`409baf3a61`: a dialect's run reads the other dialects' files of its language as text documents, path and
lines with no syntax tree) and the engine-tsconfig branch (`27c35a7edb`: the runner makes `tsconfig`, `tsconfig_for`,
`tsconfig_path` and `package_export`, and an ECMAScript document's module is its path). The last column says which.

| # | Gap | Rules affected | Closed by |
|---|---|---|---|
| 1 | Cross-file `sym_of`/`target` refused: no producer for `tsconfig_for`, `tsconfig_path` | global-client-in-transaction, side-effect-in-transaction, sensitive-fields-to-client | engine-tsconfig |
| 2 | TypeScript documents have module `""` | side-effect-in-transaction, sensitive-fields-to-client, global-client-in-transaction | engine-tsconfig |
| 3 | `.ts`, `.tsx` and `.js` files run in separate lanes | global-client-in-transaction, side-effect-in-transaction, sensitive-fields-to-client | engine-ts-lanes, partly |
| 4 | `file(p)` refused, and `file_line`/`dir_entry` requests may not derive from documents | bulk-write-without-where, request-body-as-data, side-effect-in-transaction, dev-migration-in-deploy, global-client-in-transaction | open |
| 5 | A `.prisma` file is a document only with `.scope include`; `.scope text` alone gives none | request-body-as-data | open |
| 6 | `earlier(m, n)` gives no rows for TypeScript nodes | read-modify-write, bulk-write-without-where | open |
| 7 | A negation misses rows of a regex-backed relation | raw-unsafe-interpolation | open |
| 8 | `match_all` with captures gives no rows inside a recursive rule | dev-migration-in-deploy | open |
| 9 | A literal `{` in a `.violation` message is a placeholder, and the error names neither place nor fix | every rule with a message showing code | open |
| 10 | A head variable bound only by `match` named groups is refused as ungrounded | global-client-in-transaction (an earlier draft) | open |
| 11 | test-rr names neither the missing nor the extra lines of a failed `.test cases` | query-in-loop (all rules) | open |
| 12 | `_` in a rule head fails with a message that names neither rule nor line | query-in-loop | open |
| 13 | Findings placed on files outside the rule's scope | dev-migration-in-deploy | open (core behaviour unknown) |
| 14 | No language for Dockerfile, YAML, TOML or shell; JSON strings split at escapes | dev-migration-in-deploy | open |
| 15 | The content store's hard limit stops a run on a large repository | raw-unsafe-interpolation (zoonk/zoonk) | open |
| 16 | `deparen` yields two nodes for one pair value; `field(p, "operator", o)` gives nothing | find-first-without-order-by (dropped) | open |
| 17 | Planner: "no statistics for <relation>.c0" on a demand relation | global-client-in-transaction, side-effect-in-transaction | open |
| 18 | Syntax guide: `match` is a whole-string match; aggregate bodies share outer variables | raw-unsafe-interpolation | documentation |

---

## 1. Cross-file `sym_of`/`target` refused

`tsconfig_for` and `tsconfig_path` (core `.input`s) have no producer in test-rr or run-rr, so any rule whose cone
reaches tsx/names' `target` (and with it `sym_of` across files, `spec_resolved`) fails with "tsconfig_for is read, and
nothing derives it or binds it".

**Repro:** a rule `X(n) :- kind(n, "identifier"), text(n, "prisma"), sym_of(n, q).` (or `sym_of(n, "react#useState")`)
with `#include "tsx/names.rerail"` and two fixture files; test-rr fails with that message.

**Workaround:** three rules resolve imports themselves: global-client-in-transaction (`gct_sym`/`gct_target`:
relative, `@/` and `~/` by file layout, barrels, a package-symbol fallback), sensitive-fields-to-client (`sfc_imports`:
relative, `@/`/`~/`/`#/` and `@scope/pkg/x` by path suffix), side-effect-in-transaction (`ses_sym`/`ses_resolve`:
tsconfig `paths` read as text, monorepo packages by package.json name). Each misses tsconfig `paths` its layout
guess does not cover ([NOTES-prisma.md](NOTES-prisma.md#library-candidates)).

**Closed by** engine-tsconfig (`27c35a7edb`), which makes `tsconfig`, `tsconfig_for` and `tsconfig_path` from each
folder's tsconfig.json over its `extends` chain. The three resolvers can then go back to tsx/names; not done here.

`package_for` and `package_field` were already bound: one-client's `UncachedNextClient` expectations fail when `"next"`
is removed from its case package.json, so it reads them. dev-migration-in-deploy found `package` (a package's name)
unbound and reads package names and scripts from the package.json documents instead; engine-tsconfig's commit says the
same repository walk makes `package*`, so this may have been a misreading [inference, not rerun].

## 2. TypeScript documents have module `""`

`doc(d, path, module)` gives `module = ""` for every `.ts`/`.tsx` file, so names.rerail's `resolve_base` and
`@qname(m, x)` symbols all collapse to `"#x"`.

**Repro:** `D(r, p) :- kind(r, "program"), docof(r, d), doc(d, _, m), p = cat("[", m, "]").` as a debug violation
reports `[]`.

**Workaround:** side-effect-in-transaction names modules by path; global-client-in-transaction's `@qname(m, x)` keys
are unique only through its own resolver.

**Closed by** engine-tsconfig: an ECMAScript document's module is now its path without its extension.

## 3. `.ts`, `.tsx` and `.js` files run in separate lanes

A `.language typescript` rule runs once per dialect, and a cross-file join from a file of one dialect never sees the
files of another. Text-scoped files (`.scope text "**/*.prisma"`) are seen from every lane.

**Effects:**
- global-client-in-transaction misses a `.tsx` caller of a `.ts` helper that queries the outer client (case
  `src/app/admin/orders/page.tsx:10`, marked `// cross-lane`); `.tsx`→`.tsx` works.
- side-effect-in-transaction misses a `.tsx` server action or route calling a helper in a `.ts` mail or queue module
  (case `src/app/admin/actions.tsx:12`, marked `// cross-lane`).
- sensitive-fields-to-client misses a `.tsx` page passing a row to a `.jsx`/`.js` client component (case
  `src/app/team/legacy/page.tsx`, marked `// cross-lane`), and does not see a global omit in a `.ts` client file from a
  `.tsx` query: a false report (repro: the case repository plus `src/app/keys-page.tsx` returning
  `prisma.apiKey.findMany()` as a client prop is reported with `hashedKey of ApiKey`).
- A `.scope text "**/tsconfig.json"` document sits in a lane of its own that TypeScript documents cannot join, so
  global-client-in-transaction could not read tsconfig `paths` as text either.

No corpus match was lost to it that the writers could find: every global-client-in-transaction match is the
single-file form, and no code read for the other two rules had a cross-lane shape. The single-file rules
(unawaited-transaction-query, raw-unsafe-interpolation, read-modify-write, one-client, query-in-loop) are unaffected;
bulk-write-without-where and request-body-as-data join only the schema, read as text in every lane (their `.tsx`
cases pass).

**Partly closed by** engine-ts-lanes (`409baf3a61`): each dialect's run now reads the other dialects' files as text
documents (path, source text and lines, no syntax tree). Joins that need the other file's tree (a helper's body, the
`new PrismaClient({ omit })` call, a `'use client'` directive read from the tree) stay open [inference from the commit,
not rerun].

## 4. `file(p)` refused, and file reads may not derive from documents

`file(path)` is declared `.input` in core, but a rule that reads it is refused by the planner: "the cone of requests
reads base relation file: a load answers exports only (decision 0065 §2a)". `file_line(path, …)` and `dir_entry`
requests computed from `doc` paths fail the same way ("reads base relation doc"); `file_line("prisma/schema.prisma",
_, l)` gave no rows for a fixture.

**Repro:** in a TypeScript rule, `m(x) :- file(p), match(".*\\.prisma", p), file_line(p, _, t),
match("\\s*model\\s+(?<x>\\w+).*", t).` plus any violation using `m`; test-rr fails with that message. And
`file_line(@join(@dir(p), "tsconfig.json"), …)` from `doc(_, p, _)` fails with "reads base relation doc".

**Workarounds:** bulk-write-without-where walks directories from the root with `dir_entry("", …)` six levels deep
and reads the schema with `file_line`; request-body-as-data and sensitive-fields-to-client add `**/*.prisma` to
`.scope include` and `.scope text` and read `source`/`split_line`; side-effect-in-transaction walks folders three
levels deep for tsconfig.json; dev-migration-in-deploy walks the checkout from `""` six levels deep and reads every
walked `.sh` file, not only those a deploy command names, to keep its requests free of documents.

## 5. A `.prisma` file is a document only with `.scope include`

`.scope text "**/*.prisma"` alone gives no `doc` row for the schema; it must be in `.scope include` too.

**Workaround:** request-body-as-data and sensitive-fields-to-client list it in both.

## 6. `earlier(m, n)` gives no rows for TypeScript nodes

core.rerail says `earlier` is start-offset order outside Python, but it gives nothing for TypeScript.

**Repro:** under `rerail-engine logic`, one `.ts` file `a();\nb();` and a policy (`.language typescript`,
`.scope include "**/*.ts"`) with

```
.decl e(x: number, y: number)
e(x, y) :- kind(a, "call_expression"), kind(b, "call_expression"), earlier(a, b), line(a, x), line(b, y).
.output e
```

gives `e: 0 rows`; with `node(d, a, _, s, _), node(d, b, _, t, _), s < t` in place of `earlier`, 1 row. In
bulk-write-without-where's cases, `function f(userId?: string) { if (!userId) return; await
prisma.session.deleteMany({ where: { userId } }); }` with `ref_site(o, s), ref_site(r, s), earlier(o, c)` is empty.

**Workaround:** read-modify-write (`rmwp_before`) and bulk-write-without-where compare start offsets.

## 7. A negation misses rows of a regex-backed relation

`RawUnsafeInterpolation(e, …) :- …, !rui_safe(e), …` still fired for a node that a debug rule reading `rui_safe(e)`
positively shows, where `rui_safe(e) :- kind(e, "call_expression"), field(e, "function", f), text(f, n),
match("…Number…", n)` tests a regex on another node than its head.

**Repro:** drop `rui_cand(e),` from `rui_coerced` in raw-unsafe-interpolation and run test-rr: it reports
`src/server/tenancy.ts:25` and `:26` (`${Number(limit)}`, `${escapeLiteral(label)}`). A 10-line program with the same
shapes (call argument, template substitution, join aggregate) did not reproduce it, so the trigger is something in
the larger program (the includes' strata, or the aggregate feeding `rui_pasted`).

**Workaround:** every regex-backed `rui_safe` rule is guarded by a positive candidate relation (`rui_cand(e) :-
rui_pasted(e, …)`).

## 8. `match_all` with captures gives no rows inside a recursive rule

```
.decl sc(n: symbol, t: symbol)
sc("a", "npm run b && x"). sc("b", "pnpm c"). sc("c", "danger").
.decl run(t: symbol)
run("npm run b && x").
run(t3) :- run(t), match_all("(?:npm run|pnpm) (?<s>[a-z]+)", t), sc(s, t3).
```

`run` holds only the seed; with `match("(?:.*[ ;&|])?(?:npm run|pnpm) (?<s>[a-z]+).*", t)` instead it reaches
"pnpm c" and "danger". The positional form `match_all(re, t, o, s, e)` fails the same way; outside recursion both
work (json language, any package.json).

**Workaround:** dev-migration-in-deploy extracts the calls from a non-recursive `dmid_text` relation.

## 9. A literal `{` in a `.violation` message

`{` and `}` in message text are interpolation even with a space after `{`; a literal brace is written `{{`/`}}`. A
message showing code (`(async (tx) => { … })`, `data: { name: body.name }`, `{ await tx.… }`, `orderBy: [{ createdAt:
"desc" }]`, `import { prisma }`) fails with "message: no column  await tx", which names neither the violation's text
position nor the fix.

**Repro:** `.violation V(at: node) "use { x }"` with any rule body.

**Workaround:** every rule writes `{{`/`}}`. The syntax guide could say so, and the error could name the escape.

## 10. A head variable bound only by `match` named groups is refused

`gct_spec_base(b, e) :- match("(?<x>.*)\\.js", b), …` is refused as ungrounded even when every caller binds `b`.
Found in an earlier draft of global-client-in-transaction's resolver, which now resolves through tsx/names'
`resolve_base`.

## 11. test-rr does not say which lines a failed `.test cases` missed

When an expectation is missed, test-rr prints only `FAIL …::cases`, with no list of the missing or extra lines.

**Repro:** remove any `.expect` line of query-in-loop and run test-rr. The only way to see which line fired was
`run-rr` on the cases folder with `csv` output.

## 12. `_` in a rule head

`prq_batched("createMany", _) :- …` fails test-rr with "test cases: _ in a head", which names neither the rule nor the
line.

## 13. Findings outside the rule's document scope

dev-migration-in-deploy is a `.language json` rule over package.json, vercel.json and railway.json; it reads
Dockerfiles, Procfiles, workflows, TOML/YAML platform files and shell scripts with `dir_entry`/`file_line` and places
findings there with `@place(path, line)`. run-rr and test-rr report them; whether core runs the rule again when a
session edits only such a file is unknown.

## 14. No language for deploy files; JSON strings split at escapes

There is no language for Dockerfile, YAML, TOML or shell: dev-migration-in-deploy reads them as lines, with regexes
for guards and workflow jobs (jobs must be indented two spaces). JSON `string_content` nodes split at escape sequences
(`"if [ \"$X\" ]"` gives three), so the rule takes a string's text from the `string` node with its quotes sliced off
(`dmid_jstr`).

## 15. The content store's hard limit

raw-unsafe-interpolation did not finish on zoonk/zoonk (about 9,700 TypeScript files): the engine's content store hit
its hard limit. The other 16 apps of its run finished (civitai, about 17,000 files, in 83 s).

## 16. `deparen` yields two nodes for one pair value; `field(p, "operator", o)` gives nothing

Found by the dropped find-first-without-order-by rule. With `fff_pair` reading a pair's value through `deparen`,
`Dbg(v, t) :- fff_pair(w, k, v), !kind(v, "object"), text(v, t).` reports `{ contains: q }`: one value yields two nodes
of the same text, one of kind `object` and one not. Probably the two `deparen` definitions (`tsx/modules.rerail` and
`tsx/eslint-a-common.rerail`) disagree. The rule keyed its check on the key instead (`fff_object_key`). Separately,
`field(p, "operator", o)` gives nothing for `unary_expression`/`binary_expression`; the builtin `op(p, "!")` works.

## 17. "no statistics for <relation>.c0"

A demand relation the planner makes can lack statistics: `file_line(f, 1, _)` over candidate tsconfig paths derived
from document paths failed in planning with "no statistics for gct_alias_doc_at.c0" (global-client-in-transaction), and
deriving folders through a per-file relation anchored on a node failed the same way (side-effect-in-transaction). Both
rules moved to walks from the root (gap 4).

## 18. Syntax guide

- `match(re, s)` is a whole-string match (Soufflé's), not a search: `match("[=<>]\\s*$", "id = ")` fails and
  `match(".*=\\s*", …)` holds. `(?i)`/`(?s)` work.
- A variable used inside a `join … : { … }` aggregate body and outside it is shared, not local to the aggregate
  (Soufflé semantics): `pre = join x by k : { child(t, f, k), … }, child(t, f, i - 1)` silently restricts the
  aggregate to one row. Rename the inner variable.
