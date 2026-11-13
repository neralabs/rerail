# 0043: Every rule the engine runs is written in `.rr`

Status: accepted (user, 2026-11-05)

## Intro

Until now the engine read its rules in its own dialect, `.rdl`: Datalog with `$`-prefixed
variables, `@`-functions for text tests (`@glob($s, "x*")`), and `.demand` lines for demand.

Another branch (More-ergonomic-rule-syntax) designed `.rr`: standard Soufflé Datalog, plus a small
set of additions in `docs/rule-syntax/lib/core.rr`. The user chose it as the one format (2026-11-05:
"This is the file format we need to target", and "make sure that we truly do end up with only
souffle-format rules").

A short example. The same rule in both forms:

```
// .rdl (before)
cache_name($d, $n) :- name($d, $n, $s), @glob($s, "_*cache*").

// .rr (now): bare variables, the pattern first, as Soufflé's match() takes it
cache_name(d, n) :- name(d, n, s), glob("_*cache*", s).
```

## Decision

1. **One reader.** `logic::rr::parse` (and `parse_file`, which follows `#include`) is the only
   reader on the engine's path. It lowers `.rr` to the same `Program` the planner, magic sets and
   the backends already take, so nothing after the front end changed.
2. **One writer.** `logic::rr::print` turns a `Program` back into `.rr`. Programs the engine builds
   itself (a wave's rules, the YAML front end, the converter's output) are passed on as `.rr` text.
   A test checks that each printed program reads back as itself.
3. **The converter writes `.rr`.** `candidates/datalog/{id}.rr`. Its 60 earlier `.rdl` outputs are
   gone.
4. **The engine's own programs are `.rr`.** The data-layer programs and their prelude, the
   serverplan quality program and its template (`final-shape/logic/template.rr`) were translated by
   `rr::translate`, with comments kept. A test checked that each translation gave the same program
   before the `.rdl` was removed.
5. **Copied `.rdl` files stay as copies until step 31.** T's typing programs and the plan goldens'
   programs keep their `.rdl` beside the `.rr` the engine reads; a test checks the two still agree.
   The old `.rdl` parser (`front::parse`) is kept only for that test.
6. **What lowering doesn't cover yet is refused by name**, not guessed. Example: `.violation`,
   `~` patterns, `ordered by`, `sum`. The hand-written data-layer rules in `docs/rule-syntax/data-layer`
   all stop at one of these today; lowering them is list step 30's work.

Small changes that came with the move:

- the wave relation `match` is now `matched`, because `match` is a built-in in `.rr`;
- relations named like a built-in get a trailing `_` (`glob` → `glob_`);
- merging two adjacent negated tests is for text tests only. Before, a comparison such as
  `@instr(x, "a") = 0` was merged too, which was wrong.

## Evidence

- The recorded findings replay gives the same totals as before: `total: 2350 155 153 0`
  (8.0 s CPU, machine load about 50).
- The `logic::`, `waves::`, `slice::`, `typing::`, `convert::` and `front::yaml` tests pass.
- One-rule measure on the 9,010-file corpus: 8.2 s CPU cold, 200 MB resident. It is cold because the
  warm caches are keyed by family name, and `match` became `matched`.

## Prior work consulted

- **Lessons ledger, row 19** ("lower patterns to embeddable SQL fragments … don't build a SQL
  interpreter"). *Agrees.* A rule is Datalog text; it carries no SQL. The lowering owns the SQL.
- **Spike design index, single-file relations** (77 of 99 specs need only one file's syntax).
  *Unchanged.* The format changes how rules are written, not how they are partitioned.
- **0037 and 0042 (text tests).** *Kept.* `.rr`'s `glob`, `prefix`, `match` and sets of them lower
  to the same canonical text tests. `match` matches the whole string, so it lowers to an anchored
  regex.

## Open

O71–O75 in `open-items.md`.
