# 0108: The cone of requests is magic sets, maintained by delta

Status: proposed (2026-11-10). The join half is built (below); the delta half is not.

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

## Steps for the delta half

1. `fs_dir` and `fs_line` as relations of the cone's fact space (t33's K2): the native step
   writes rows there, not into `World`'s maps.
2. The cone's relations kept with counts in frames (as K4 keeps the cross part), not as
   whole-stratum digests (`kept_strata`).
3. `maintain::rewrite` accepts gates by the wave order above, instead of refusing them.
4. The inputs' delta: `Δrequests` from the per-file facts' delta, `Δfs` from the journal.
5. Remove the cone's whole-run cutoff (`World::still`). The budget falls back as described.

Test: random edit sequences (add or remove an import, a typeshed stub, a workspace package)
give the same rows maintained as computed fresh.

## Building on it

The same shape holds for any relation read with a bound key that is answered by a native step
or a search, such as decision 0083 §4's memoised searches, including misses. Declare the
relation's adornment, write its answer as `r :- m_r, source`, and the rewrite, the gates and
maintenance apply unchanged. Two parts are not built yet: adornments over several columns (the
demand key is only the first column today, which 00-review measured as 13× slower for a
one-file query), and a load budget that is a function of the requests (a rank, not wave order).
