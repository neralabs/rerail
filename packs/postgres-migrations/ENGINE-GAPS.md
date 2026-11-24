# Engine gaps found by the postgres-migrations rules

Gaps in the engine, its SQL grammar and test-rr that the rules of this pack ran into, each with a repro, the rules it
affects and the workaround in use. The two engine sub-branches of this batch close none of them: `…-engine-ts-lanes`
(409baf3a61) fixes the `.ts`/`.tsx` lane split and lets text-only documents join across lanes, and `…-engine-tsconfig`
(27c35a7edb) adds the missing tsconfig producers; of this pack only `dropped-column-still-used` reads TypeScript, and its
cases pass with or without them.

## 1. test-rr withholds every rule that negates `git_rev`

A test's fixture directory is not a Git repository, so `git_rev` is unreadable and every rule that negates a relation
built on it is withheld: the run prints `unknown: <Rule> (git_rev unreadable …): rows may be missing` and reports no
findings. Even `count : { git_rev("base", _) } = 0` is withheld.

Repro (`.language sql`), run by `test-rr` with one `.sql` fixture and no `.git`:

```
a(s) :- kind(s, "alter_table"), git_rev("base", _).
R(s) :- kind(s, "alter_table"), !a(s).
```

gives "no findings". Affects the nine SQL rules (the library's `pgm_applied`/`pgm_pending` negate `git_rev`). Workaround:
the one-commit repository `cases/_git/` (`HEAD`, `config`, `refs/heads/main` and the two loose objects of an empty
commit) written as `.git/…` fixtures by each rule's `.fixtures repo`. A `.git` test directive, or treating a fixture
root as a repository with no base, would remove it.

## 2. test-rr cannot give a Git base

test-rr runs without `--base`, so `git_rev("base", _)` has no row in any test, and the library's Git-base clauses
(a pending migration is one the base lacks; a table a pending migration creates is new) are not exercised by any case.
Since the library reads only each folder's newest migration when there is no base, the `cases` tests give
`pgm_applied` no rows (`.given pgm_applied`) to read every case migration. A `.base` directive naming a commit of a
fixture repository would let the cases cover both. Affects the nine SQL rules.

## 3. `file_line` cannot read a path computed from a document

`file_line(q, _, l)` with `q` computed from `doc` or `file` fails `run-rr` with "the cone of requests reads base
relation doc: a load answers exports only (decision 0065 §2a)" and in `test-rr` silently gives no rows. Repro
(`.language sql`):

```
P(r, l) :- kind(r, "create_index"), docof(r, d), doc(d, p, _),
    file_line(@join(@dir(@dir(p)), "migration_lock.toml"), _, l).
```

So a rule cannot read Prisma's `migration_lock.toml` provider or drizzle-kit's journal `dialect` beside the migration
it checks, and `file(p)` cannot feed `file_line(p, …)` either. Workarounds: the library reads the lock file as a
document of the run (each rule lists `**/migrations/migration_lock.toml` under `.scope include` and `.scope text`, and
`pgm_other_group` joins `doc(_, cat(g, "/migration_lock.toml"), _)` with `split_line`); `lock-timeout` finds its
configuration files with a `dir_entry` walk from the checkout root. Before the library, `constraint-on-existing-table-
not-valid` had a lock-file clause written this way that never matched; its MySQL case passed through its backticks.
Affects every SQL rule (`pgm_other_db`) and `lock-timeout`.

Related: `file()` has no row for `prisma/migrations/migration_lock.toml` when the policy's scope does not include it
(it lists `package.json`, `meta/_journal.json` and the `.sql` files). Repro: a scratch repository with
`prisma/migrations/migration_lock.toml` and `F(n, v) :- file(v), kind(n, "program").` lists no `.toml` path.
`concurrently-inside-transaction` reads the Prisma layout from the path shape instead.

## 4. `match_group` over a span another `match_group` returned gives no rows

Over a file `ALTER TABLE "User" DROP COLUMN "a";`:

```
x(sp) :- source(d, src), match_group("ALTER TABLE (?<t>\\S+)(?<b>[^;]*)", src, _, "b", b),
    match_group("DROP COLUMN (?<c>\\S+)", b, _, "c", sp).
```

is empty, though the same second `match_group` over `src` finds `"a"`. `match_all` behaves the same. Affects
`dropped-column-still-used`, which matches each ALTER TABLE action over the whole file and gives it to the statement
whose lines (`line`, `end_line` of the match) hold it.

## 5. One policy cannot read SQL trees and TypeScript trees (G1)

A policy has one `.language`. `dropped-column-still-used` must read Drizzle `pgTable` calls and raw SQL templates as
TypeScript trees, so it reads the migrations and `*.prisma` files as text (`.scope text`), cannot include the pack's SQL
library, and so has no Git-base gate: every migration is read, applied or pending, and a `DROP COLUMN` inside a
comment within an `ALTER TABLE` statement would count.

## 6. A node kind cannot be read as a symbol

`kind(a, k), match("…", k)` and `cat("", k)` with `k` from `kind(_, k)` fail to type-check: "`$k` is a Kind and a
Name". So "any action node of an ALTER TABLE" has to be written as a list of `!kind(a, "…")` (`lock-timeout`'s
`lt_action`), and a type's name is read from the node's text (`column-type-change-rewrites`).

## 7. `iglob` does not support an empty brace alternative

`iglob("{small,big,}serial{,2,4,8}", x)` matches none of `SERIAL`, `BIGSERIAL`, `serial8`. Repro:

```
R(k) :- kind(k, "keyword_serial"), text(k, x), iglob("{small,big,}serial{,2,4,8}", x).
```

over `ALTER TABLE t ADD COLUMN c SERIAL;` gives no row; an explicit list (`pgm_serial_type(@lower(x))`) does. Affects
`required-column-without-default` and `volatile-default-rewrites` (the library's `pgm_serial`).

## 8. SQL grammar: `TIMESTAMPTZ(3)` loses its precision

In `ALTER COLUMN "x" SET DATA TYPE TIMESTAMPTZ(3)` the alter_column's text ends at `TIMESTAMPTZ`, so a precision
decrease on it reads as a removal and is not reported. Affects `column-type-change-rewrites`.

## 9. SQL grammar: a quoted column name in a column definition is a `literal`

In `ADD COLUMN "slug" TEXT`, `"slug"` parses as a `literal`, an unquoted name as an `identifier`, so `ident_pg` (which
reads identifiers) does not fold it. The library's `pgm_ident` and `pgm_colname` read the node's text either way.
Affects `required-column-without-default`, `volatile-default-rewrites`, `column-type-change-rewrites`,
`rename-as-drop-and-add`.

## 10. `visit_index` has no rows for SQL documents

A rule joining `visit_index` over a SQL document gets nothing; `node(d, n, _, start, _)` offsets work, and the pack
orders statements by them. Affects `lock-timeout` (and every rule's "earlier in the same file").

## 11. A probe anchored on `doc_root` failed to plan

A probe with `doc_root(d, r)` as the anchor failed `run-rr` with "no statistics for rr_ctx1_at.c0" (found while writing
`index-on-existing-table-blocks-writes`; not reduced further).

## 12. `rerail-engine csv` drops a violation's message parameters

The csv export of a run has each violation's location and code, not its parameters; reading what a message says needs
the run-rr JSON (`.diagnostics[].parameters`). Affected the reading of `rename-as-drop-and-add`'s corpus matches.
