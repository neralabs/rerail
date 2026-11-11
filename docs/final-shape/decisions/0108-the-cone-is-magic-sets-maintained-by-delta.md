# 0108: The cone of requests is magic sets, maintained by delta

Status: built (2026-11-10). The join half and the delta half (rounds) are both built (below).

## Intro

**The problem.** Rules read the file system through two relations, `dir_entry(dir, name,
is_dir)` and `file_line(path, i, text)`, and ask for documents with `load`. Nobody lists every
directory up front. A rule that reads `dir_entry(d, …)` *asks* for `d`, and the engine answers
inside the fixpoint (decision 0065 §2a). The asks and their answers form the *cone of
requests*: for Python, import resolution against typeshed and the workspace.

Today an edit that touches the cone (adding an `import`) reruns the cone as whole strata. This
decision says what the cone is, in terms that are known to be correct, so that it can be
maintained from the edit's delta instead, and so that other "ask, then answer" analyses can
use the same machinery.

**The rule.** The cone is the magic-sets transformation of the program for the world
relations, with the file system as an ordinary input relation. Given that, maintaining it is
maintaining ordinary rules, with one extra ordering condition for the gated negations.

## What the cone is

Magic sets (Bancilhon et al. 1986; Beeri and Ramakrishnan 1991) rewrite a program so that a
relation is computed only for the keys some rule needs. A relation `r` read with its first
column bound gets a *magic* relation `m_r(key)`, the keys asked for, and `r` is computed only
for those keys.

The cone already has this shape, written by `rr::lower` (`world_demand`):

```text
// what the rewrite writes, for a rule that reads dir_entry(d, n, k) with d bound
rr_ctx7(d)    :- <the rule's literals that bind d>.   // supplementary magic relation
rr_dir_req(d) :- rr_ctx7(d).                          // magic relation: m_dir_entry^bff
```

The answer is the part not written as a rule: the engine lists `d`. As a rule it is a join of
the request with a relation of the file system, `fs_dir`, which nobody materialises in full:

```text
dir_entry(d, n, k) :- rr_dir_req(d), fs_dir(d, n, k).    // the native step "list d"
file_line(p, i, t) :- rr_file_req(p), fs_line(p, i, t).  // the native step "read p"
```

`fs_dir` and `fs_line` are input relations whose rows exist only where asked. With that one
step written out, the cone is a plain Datalog program over inputs `fs_*`, the corpus's
per-file facts, and the rules. Three facts follow from the standard results:

1. **The answers are right.** A magic-sets rewrite derives exactly the original program's rows
   for the asked keys when every request is derived from literals that hold in the original
   program. `world_demand` takes the literals that do not read the atom's own outputs, and
   drops literals only to widen demand, never to narrow it (`context`). Asking for too much is
   sound; asking for too little is not, and it does not happen here.
2. **Maintenance is ordinary.** Requests, answers and the cone's derived relations are
   derived relations. Their inputs are `fs_*` and the per-file facts. An edit is a delta of
   those inputs, and counting or DRed (decision 0077) maintain derived relations from input
   deltas. A deleted request deletes its answers (`dir_entry` loses `d`'s rows), because the
   answer is a join with the request.
3. **The journal gives `Δfs`.** The journal (decision 0105) names the paths changed since the
   last run. `Δfs_dir(d)` is `d`'s new listing against its kept one, for each `d` both named
   and asked. `Δfs_line(p)` is the same for files. Paths not named have `Δfs = ∅`.

## The gates: the one non-standard part

Inside the cone a negation can read a relation that is still growing: `!py_workspace(q)` is
true until `q`'s requests are answered. The fixpoint gates each such negation per key
(`answers::gate`). `done_N(t)` opens only when nothing `N` reads has changed in a wave and no
request is unanswered. This is sound when `N(t)` reads only answers that `t` determines
(0065 §2a.3). The program is then *locally stratified by key*: for each key, `N(t)` is complete
before any rule reads `!N(t)`.

Maintenance keeps the same order. It runs the delta in waves as the fixpoint does, and holds
every gate shut until its relation is quiet:

```text
wave 1..k   apply Δinputs; propagate through every rule except across a gated negation
            (Δ of N(t) accumulates for each key t)
quiet       for each gate key t with ΔN(t) ≠ ∅:
              ΔN(t) inserted  -> Δ(!N(t)) deleted  (rules reading !N(t) over-delete, as DRed)
              ΔN(t) deleted   -> Δ(!N(t)) inserted
wave k+1..  propagate those, gates shut again for any key they reach; repeat until quiet
```

This is DRed with negation, applied stratum by stratum, where a stratum is a key's position in
the gate order rather than a relation's. It is correct under the same condition the fixpoint
needs. A key whose `N(t)` changes after its gate opened breaks that condition, and then
maintenance must fall back to rerunning the cone.

## What is not a function of the requests

Two things the world does today are not Datalog functions of the inputs. Maintenance treats
each one explicitly:

- **The load budget** (`load.budget`, 4000) admits loads first-come, in wave order. While
  `loaded < budget` the budget does not bind, and `loaded` is a function of the requests. When
  it binds, the run reruns the cone and says so. (On the workload: 208 loads, so it never binds.)
- **Refusals** (a path outside the roots, missing, another grammar) are rows of `fs_*`
  (absent), or a fixed relation of roots. Both are inputs, so neither needs special handling.

## Built (this change): the join under the cone

The rerun cost was not the cone's evaluation strategy. It was three plans:

| where | before | after |
|---|---|---|
| `py_try` "stubs": a delta of `dir_entry(b, x, 0)` crossed `py_want` | 404 → 4 M rows → 151, 5.2 s | probes by `h` |
| the same rule, delta at the gate | 1,872 → 683 K rows | probes by `q` |
| `py_qlast`: `substr` had no native form, so the rule ran as SQL over the vtabs every wave | 12 s | native, 0.09 s |

The fixes:

- **A concatenation runs backwards.** For `v = cat(a, x, b)`, the rewrite adds
  `x = @str_between(v, a, b)` for each variable part that is named once. Concatenation is
  one-to-one in any one part, so whichever side the plan binds first, the other becomes a test.
  The rows are the same.
- **A function keys a join.** In `R(…, q, …), v = @f(q)` where another atom joins on `v`, the
  rule reads `R__key_f(v, …)`, a relation holding R's rows with `f`'s value first. It is an
  ordinary derived relation, kept and maintained like any other. Not done for node relations,
  the world's relations, or inputs.
- **A gate waits for its key.** `done_N(t)` holds only keys that its own rule's literals give.
  The planner therefore joins it only once `t` is bound, and never crosses into it to bind `t`.
  Its size is whatever has been answered so far, which an earlier wave's statistics price at
  one row.
- **`substr` natively** (`rr_substr`), with SQLite's semantics, tested against SQLite.

Workload (237 files plus typeshed; load 42–56), CPU = user + sys:

| run | before | after |
|---|---|---|
| import edit | 19–21 s | 2.4 s (cone 0.78 s of it) |
| comment edit | 1.0 s (load ~30) | 1.7 s (load ~60), and the same with the rewrites off |
| unchanged | 0.18 s | 0.3 s |

The outputs are identical when computed fresh with the rewrites on and off
(`RERAIL_JOIN_KEYS=off`, with an `--env` setting changed so that the program is prepared again:
the kept prepared program is keyed by the program's text, not by this switch).

## Built: the delta half, as rounds

**What makes it ordinary.** Take the answers (`dir_entry`, `file_line`, `loaded` and a loaded
document's per-file rows) and the gates (`done_*`) as inputs. The cone's rules then stratify.
On the workload every relation is a stratum of its own: 12, 37 and 28 strata in the three
cones, the largest of one relation. All of the cone's recursion goes through the answers.

So the cone runs as *rounds* of one stratified program, the *round program*. Each round is an
ordinary maintained run (`corpus::maintained`, the cross part's `cut`, `rewrite` and `after`
core). The requests are answered between rounds:

```text
round     inputs (per-file rows, answers, open gate keys)
            -> the round program, maintained from the last round
            -> requests, asks
between   new dir and file requests asked     -> answer them, next round
          else new loads asked                -> answer them, next round
          else (quiet) done_N := ask_N        -> if any gate key opened, next round
          else                                -> the cone is done
```

This is the gated fixpoint's own order: answers as they are asked, gates opened only when
nothing is unanswered. No new evaluator is involved. A gated negation is a plain negation
guarded by `done_N`, which is an input of the round.

**Across runs.** The last run's final round is kept in the store (`kept_strata`, in frames
under the round program's key). A run starts from it:

- The kept answers, loaded documents' rows and open gate keys stand in for this run's. They
  are *provisional*.
- Once the run is quiet, every key still asked is answered for real: directories and files
  first, then loads. Then the provisional rows are dropped, and the rounds go on until quiet.
- A load the last run made that this run no longer asks is never asked of the main thread. Its
  rows go with the provisional ones.

**A run with nothing kept** runs the gated fixpoint and records its answers. It then runs one
*closing round* over them with every gate open. That round must ask nothing new, or it is not
kept. The kept round is what the next run starts from.

**Fallback.** The rounds give up past 64 rounds, or when the kept state is not a final round of
this program (it must hold every request and every input). The gated fixpoint then runs.
`RERAIL_CONE_ROUNDS=off` runs the fixpoint alone.

**Inputs by delta.** A round rebuilds and binds again only the inputs whose rows changed: an
answer's relation, the provisional rows when they are dropped, a gate when it opens. The
statistics of a bound source are made when a stratum first plans with it (decision 0101 §4).

Workload, import edit (`import csv` added to one file, then removed), CPU = user + sys, load
50–75:

| run | before (fixpoint) | rounds |
|---|---|---|
| whole run | 2.4 s | 1.4–1.5 s user |
| the cone's job | 0.78 s | 0.28 s (4 rounds; 22 of 33 strata run, all maintained) |

The outputs equal a fresh run's (`--env cmp.k=…` with `RERAIL_CONE_ROUNDS=off`), for the
edit and for its revert.

Test (`the_cone_maintained_by_rounds_loads_what_a_fresh_cone_loads`): a new import, a
library's own import, an import taken away, and a library file removed. Each time the kept run
runs rounds and loads what a run with nothing kept loads.

## Open

- **The main thread's answers.** Each run answers every key still asked: 2011 directories,
  0.31 s on the workload, 0.26 s of it in `World::dir`, the observer's kept listing, about
  128 µs a key. The journal (0105) says which directories changed, so an unchanged
  directory's kept answer could stand without asking.
- **A withdrawn load inside a run.** A load can be answered from rounds whose provisional
  answers later prove wrong; a library file changed on disk is the case. Such a load is
  already in the run's documents. The rounds do not ask it again, but its per-file rows reach
  the rest of the cross part. Not seen on the workload or in the test.
- **Sessions.** The rounds are kept in the store, not in a session's memory. A session's run
  reads them back from the frames.
- **The load budget** still admits loads in answer order. When it binds, the result can differ
  from the fixpoint's.
- `fs_dir` and `fs_line` as relations of the cone's fact space (t33's K2). The answers are the
  join `r :- m_r, fs_r` above, but the native step still answers from `World`'s maps.

## Building on it

The same shape holds for any relation read with a bound key that is answered by a native step
or a search, such as decision 0083 §4's memoised searches, including misses. Declare the
relation's adornment, write its answer as `r :- m_r, source`, and the rewrite, the gates and
maintenance apply unchanged. Two parts are not built yet: adornments over several columns (the
demand key is only the first column today, which 00-review measured as 13× slower for a
one-file query), and a load budget that is a function of the requests (a rank, not wave order).
