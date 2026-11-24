# Engine gaps found while writing the Next.js practice rules

Found while writing the 11 `next-practice-*` rules and running them with `rerail-engine test-rr` and `run-rr` over the
apps of [README.md](README.md#the-corpus), 2026-11-24. Each section gives the symptom, a minimal repro, the rules it
affects and what they do instead. Rule names drop the `next-practice-` prefix.

Two engine changes, on sub-branches of this work and not yet on main, address the first two gaps; they were not run
against these rules, so what they close is read from their commit messages:

- `…-engine-tsconfig` (`27c35a7edb`): the repository walk now makes core.rerail's `tsconfig`, `tsconfig_for`,
  `tsconfig_path` and `package_export` inputs, and an ECMAScript document's module is its path without its extension.
  It closes gap 1 and gap 3.
- `…-engine-ts-lanes` (`409baf3a61`): a TypeScript rule's run for one dialect (`.ts`, `.tsx`, `.js`) also reads the
  other dialects' files in scope as text documents (`doc`, path, source text and lines; no tree); a finding in another
  dialect's file is left to that dialect's own run. It closes the part of gap 2 that needs only the other dialect's
  file to exist or its text: an import resolved to a `.ts` file from a `.tsx` file by its path (every rule's resolver
  checks `doc` paths), middleware-not-applied's `pageExtensions` check of a `next.config.mjs`, and
  revalidate-unknown-tag's "the tag is written elsewhere" fallback. Joins that need the other dialect's syntax tree
  (its imports, calls, writes, directives) stay open until the rules read those from text or the engine gives the tree.

| # | Gap | Rules affected | Closed by |
|---|---|---|---|
| 1 | No producer of `tsconfig_for`/`tsconfig_path`/`package_export`; `spec_resolved` empty | all but public-env-secret, revalidate-unknown-target and revalidate-unknown-tag, which need no import resolution | engine-tsconfig |
| 2 | `.ts`, `.tsx` and `.js` files are separate runs; joins never cross them | without-auth, unvalidated-input, middleware-not-applied, redirect-in-try, revalidate-unknown-tag, request-api-in-cache-scope, non-serializable-client-prop, edge-imports-node-only, client-reaches-server-module | partly, engine-ts-lanes |
| 3 | A TypeScript document's module name is empty | non-serializable-client-prop | engine-tsconfig |
| 4 | Findings outside code documents are reported once per language run | public-env-secret | open |
| 5 | Hidden files are never documents | public-env-secret | open |
| 6 | A request (`dir_entry`, `file_line`) keyed by a value derived from `doc` or `file` fails | public-env-secret, middleware-not-applied, revalidate-unknown-target | open |
| 7 | A disjunction in front of `dir_entry` gives no rows | public-env-secret | open |
| 8 | A recursive walk building two path columns is refused as unbounded | revalidate-unknown-target | open |
| 9 | A recursive relation with `substr`/`strlen` on its delta has "no native step" | server-action-unvalidated-input | open |
| 10 | A message cannot contain `{` or `}` | server-action-without-auth, server-action-unvalidated-input | open |
| 11 | A symbol constant in a violation's head prints as -1 | revalidate-unknown-target | open |
| 12 | A kind or field name cannot be put into a string (`$k is a Name and a Kind`) | middleware-not-applied | open |
| 13 | `.scope text "**/tsconfig.json"` gives no document in a TypeScript run | client-reaches-server-module | open |
| 14 | `run-rr` `.output` writes partial or empty CSVs | client-reaches-server-module | open |
| 15 | `test-rr --json` lists findings without their messages | revalidate-unknown-target | open |
| 16 | `child(p, c, i)` counts comments as children (a trap, not a bug) | unstable-cache-closure-key | not a bug |

---

## 1. No tsconfig or package-export facts; `spec_resolved` empty

**Symptom.** A rule whose cone reaches `tsx/names.rerail`'s cross-file `target`/`sym_of`, or that includes
`tsx/modules.rerail`, is refused by `test-rr` and `run-rr`: "tsconfig_for is read, and nothing derives it or binds it:
the corpus runner has no producer of it" (or the same for `tsconfig_path`). A program may not derive it either
("tsconfig_for is core.rerail's: a program reads it and cannot declare or derive it"). `tsx/node-resolve.rerail`'s
`spec_resolved` has no rows ("no statistics for spec_resolved.c0"). `doc`, `file`, `package`, `package_for` and
`package_field` are bound (edge-imports-node-only and middleware-not-applied read them); `package_export` is not.

**Repro.** A rule with `#include "tsx/names.rerail"` and
`R(e) :- kind(e, "identifier"), sym_of(e, "next/cache#unstable_cache").`, or a rule whose only body is
`tsconfig_path(d, p, t)`, or `P(n, f) :- spec_resolved(n, f).`, under `test-rr` with any fixture (a `tsconfig.json`
included).

**Workaround.** Every cross-file rule carries its own import resolver from `doc` paths: relative specifiers, `@/` and
`~/` (some `#/`) by file layout, the `.ts/.tsx/.js/…` and `index.*` candidates; edge-imports-node-only also reads
workspace packages' `exports` through `package_field`. The single-file rules read their file's own
`import_binding(…, "next/cache", "unstable_cache")` instead of `sym_of`. The eight resolvers are listed in
[NOTES-next.md](NOTES-next.md#library-proposed-ruleslibnextrerail); tsconfig `paths` other than `@/*` and `~/*`, and
`baseUrl`-relative bare imports, are missed.

**Closed by** engine-tsconfig (`27c35a7edb`); the rules keep their resolvers until they move to `target`/`sym_of`.

## 2. `.ts`, `.tsx` and `.js` files are separate runs

**Symptom.** `run-rr` and `test-rr` evaluate a `.language typescript` rule once per dialect (`.ts`, `.tsx`, and
`.js`/`.mjs`), each over only its own files, and write one output lane per dialect. A cross-file join never crosses
them: a relation derived in one lane is invisible in another.

**Repro.** `nd(@dir(p)) :- file_name(c, f), glob("next.config.*", f), doc(c, p, _).` and
`C(n, r) :- kind(n, "program"), nd(r).` report only at `next.config.mjs`, never at a `.tsx` file. Each rule's cases
mark the shape it misses (`// cross-lane: missed until the engine joins .ts and .tsx`, no `// caught`):
request-api-in-cache-scope's `src/components/greeting.tsx:6` (a `'use cache'` component calling `getLocale()` from
`src/lib/locale.ts`, which reads `headers()`) is not reported while `greeting.tsx:7` (a call into a `.tsx` file) is.

**Effect, by rule.**
- server-action-without-auth: a `.tsx` action whose write is in a `.ts` module is missed (case
  `src/app/notes/page.tsx` `archiveFromNotes`); one whose only session check is a `.ts` helper with a neutral name
  would be reported.
- server-action-unvalidated-input: a `.tsx` inline action passing its argument to a `.ts` helper is missed (case
  `src/app/projects/page.tsx:18`).
- middleware-not-applied: reads the file tree and package.json (shared across lanes) for its main check; the
  `pageExtensions` check sees a `next.config.ts` only from a `.ts` middleware's run, and a `.tsx` file importing
  `middleware.ts` is not seen as an import.
- redirect-in-try: a `.tsx` caller of a helper in a `.ts` module (the common page → `lib/auth.ts` shape) is missed
  (case `src/app/posts/[id]/page.tsx` `DraftBanner`).
- revalidate-unknown-tag (the tag half of revalidate-unknown-target, split off so it can log while this gap is
  open): a tag assigned only in a `.tsx` file is unseen from a `.ts` call, a false report (the rule's `lanes` test;
  in the corpus, cal.diy's `viewer.apiKeys.list`, tagged in `page.tsx` and revalidated from `actions.ts`).
  revalidate-unknown-target reads routes from the disk and is not affected.
- request-api-in-cache-scope: a `.tsx` cached page or component calling a `.ts` helper (`app/**/page.tsx` →
  `lib/session.ts`) is not followed, nor a `.ts` scope calling into `.tsx`.
- non-serializable-client-prop: a plain function imported from `lib/*.ts` is missed; a Server Function in a
  `'use server'` `.ts` file is unresolved and left alone (right, by accident); server reachability through `.ts`
  barrels is lost, so some server-only components are not recognised (fewer matches, not wrong ones).
- edge-imports-node-only: a `.tsx` edge route importing a `.ts` module is missed (case `app/api/og/route.tsx:2`;
  openstatus's six `.tsx` OG routes had 21 such imports); `.mjs` files are a third lane (minute's
  `../env/server.mjs`).
- client-reaches-server-module: its main shape, a `.tsx` component importing a `.ts` module, is invisible (case
  `src/components/ContactForm.tsx` → `src/lib/mailer.ts`); both real corpus cases found by a script that joins the
  lanes are `.tsx` → `.ts`. What it sees today (a `.tsx` → `.tsx` chain to a Node built-in) already fails `next
  build`, so the rule ships off until this gap closes.

**Closed by** engine-ts-lanes (`409baf3a61`) for what needs only the other dialect's path or text (see the top); joins
over the other dialect's tree stay open. A rule-side route, core's `load(path, module, grammar)` for the other
lane's files, was not tried.

## 3. A TypeScript document's module name is empty

**Symptom.** `doc(d, p, m)` gives `m = ""` for every `.ts`/`.tsx` file, so `tsx/names`' `resolve_base(b, m)` /
`doc(d2, _, t)` joins every document to every import (a first draft of non-serializable-client-prop resolved
`./shared-row` to all 9 `.tsx` files of its case).

**Repro.** `A(n, p, m) :- kind(n, "program"), docof(n, d), doc(d, p, m).` reports an empty module for every file.

**Workaround.** The rule matches on paths (`nscp_file`). **Closed by** engine-tsconfig: an ECMAScript document's
module is its path without its extension.

## 4. Findings outside code documents are reported once per language run

**Symptom.** run-rr's typescript, tsx and javascript lanes each report every `@place` finding and every finding in a
text document (`.env*`, Dockerfile, YAML): public-env-secret's 6 such case findings appear 3 times in the outputs.

**Effect.** Corpus counts were deduplicated by path and line. Unless core merges identical findings, an agent would
see such a finding up to three times.

## 5. Hidden files are never documents

**Symptom.** `.scope text "**/.env.*"` reads no `.env.example`: `corpus.rs` (`classified` and `scoped`) drops every
file whose name starts with `.`, in `test-rr` too (a `.env` fixture is no document).

**Repro.** A policy with `.scope include "**/.env.*"`, `.scope text "**/.env.*"`,
`D(@place(p, 0), p) :- doc(_, p, _).` and a fixture `.env.example` reports nothing for it.

**Workaround.** public-env-secret reads `.env*` files with `dir_entry` + `file_line` and reports with
`@place(path, line)`; other hidden files (karakeep's `kubernetes/.secrets_sample`) are not read.

## 6. A request keyed by a value derived from `doc` or `file` fails

**Symptom.** `dir_entry` or `file_line` keyed by a folder or path computed from a `doc` or `file` row fails:
"the cone of requests reads base relation doc: a load answers exports only (decision 0065 §2a)", or, through
`match`/`@replace_all`/`substr`, "no statistics for <rel>_at.c0" in both `test-rr` and `run-rr`. A folder derived from
`dir_entry` itself works.

**Repro.**
- `pes_env_dir(dir) :- file_name(d, f), glob("next.config.*", f), doc(d, p, _), match("(?<dir>.+)/[^/]+", p).` then
  `dir_entry(dir, n, 0)`: "no statistics for pes_env_dir_at.c0".
- `R(x) :- doc(_, p, _), x = @dir(p), dir_entry(x, "app", 1).`: "the cone of requests reads base relation doc".
- `pup(a) :- doc(_, p, _), match("(?<a>.+)/[^/]+", p). pup(a) :- pup(b), match("(?<a>.+)/[^/]+", b).
  pent(a, n, k) :- pup(a), dir_entry(a, n, k).`: "no statistics for …_pup_at.c0" (run-rr, any repository).

**Workaround.** public-env-secret walks two levels of folders down from the root with `dir_entry`;
revalidate-unknown-target walks from the root three folders deep (so it misses an `app/` deeper than that);
middleware-not-applied answers "does folder X/app exist" from `file()` paths (every folder that holds a file).

## 7. A disjunction in front of `dir_entry` gives no rows

**Symptom.** `nd(a) :- (d1(a) ; d2(a)), dir_entry(a, f, 0), …` finds nothing, with no error, where
`dd(a) :- d1(a). dd(a) :- d2(a).` and `nd(a) :- dd(a), dir_entry(a, f, 0), …` do.

**Workaround.** public-env-secret uses the two-rule form.

## 8. A recursive walk building two path columns is refused

**Symptom.** "unbounded value cycle: {rut_dir} creates values the data does not bound", even with a `@count_matches`
bound on one of them:
`rut_dir(r, @join(x, m), @join(p, m)) :- rut_dir(r, x, p), @count_matches("/", x) < 10, dir_entry(p, m, 1).`
One built column with the bound on it works.

**Workaround.** revalidate-unknown-target builds one column and derives the other.

## 9. A recursive relation with `substr`/`strlen` on its delta has no native step

**Symptom.** A relation computed with `substr`/`strlen` from a recursive relation's column, then used under
`prefix(top, p1)`, fails: "a probe of sau_root_of($root, top) planned when its delta came has no native step".

**Workaround.** server-action-unvalidated-input's alias resolver was rewritten as ancestor-directory equality lookups.

## 10. A message cannot contain `{` or `}`

**Symptom.** Braces in a `.violation` message are always read as a column reference, and there is no escape: a code
example such as `{ next }` or `{ "not": "" }` fails the policy ("message: no column next").

**Repro.** `.violation V(at: node) "a { \"x\": 1 } b"` → test-rr refuses with `no column  "x": 1`.

**Workaround.** The messages word it ("an object with an operator key such as `not`").

## 11. A symbol constant in a violation's head prints as -1

**Symptom.** `.violation K(at: node, k: symbol, t: symbol) "{k} :: {t}"` with
`K(n, "IMP", x) :- import_binding(n, _, _, x).` reports "-1 :: …".

**Workaround.** Bind the constant through a variable (`w = "…"`).

## 12. A kind or field name cannot be put into a string

**Symptom.** `cat("…", k)` where `k` is bound by `kind(c, k)` or `field(e, k, c)` is refused: "$k is a Name and a
Kind". Kinds and field names cannot be put into messages.

## 13. `.scope text "**/tsconfig.json"` gives no document in a TypeScript run

**Symptom.** With that scope, a run-rr over client-reaches-server-module's case directory has no `doc` row for the
`tsconfig.json` in the typescript lanes, so a tsconfig cannot be read as text from a TypeScript rule either (moot once
gap 1's facts are used).

## 14. `run-rr` `.output` writes partial or empty CSVs

**Symptom.** `.output rel` wrote partial or empty CSVs for relations that had rows (`crs_reached` empty while a
finding depended on it), so debugging needed `.violation` probes instead.

## 15. `test-rr --json` lists findings without their messages

**Symptom.** To read a finding's message, `run-rr` was needed.

## 16. `child(p, c, i)` counts comments

Not a bug, but a trap: comments are named children, so an argument's index shifts when a comment sits in an argument
list (`unstable_cache( // note` makes the function argument 1). unstable-cache-closure-key counts non-comment siblings
(`ucck_arg`); a small shared `ts_call_arg` would avoid the trap.
