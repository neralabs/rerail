# 0561. A climb's readers are answered in the climb, each of a row's tests asked once

Status: accepted (2026-11-21), the CPU form built. t50 step 5, second part; extends 0523.

## Intro

The scope lookup is a climb (decision 0523) and two readers of where it ends:

```
walk(n, x, s) :- nameref(n, x), wstart(n, s), !tp_ref(n, x).
walk(n, x, p) :- walk(n, x, s), !bound_in(s, x), !gbefore(n, x, s), nearest(s, p, lfscope).
walk(n, x, p) :- walk(n, x, s), cls_late(n, x, s), nearest(s, p, lfscope).
look(n, x, s) :- walk(n, x, s), bound_in(s, x), !cls_late(n, x, s).
look(n, x, s) :- walk(n, x, s), !bound_in(s, x), gbefore(n, x, s).
```

W climbs each reference's scopes and asks each scope's names once: bound here or not. The rules asked more. The
climb asked `bound_in(s, x)` for every row, then `look`, a stratum of its own, scanned `bound_in` and probed `walk`
for the rows the climb had just tested, and asked `cls_late` again. On s1000 the climb was 458 ms of probes (about
750 ns a row) and `look` about 100 ms more. Storing the rows was a small part: the cost is the questions asked
again.

## Decision

1. **The climb's ends join its stratum** (`plan::climb_ends`). A stratum of one relation joins the stratum of a
   keyed walk (0523's shape as written: `plan::keyed_walk`) when:
   - each of its rules reads the walk once, as a positive atom;
   - everything else it reads is made before the walk.

   It is then answered where the walk's rows are made. A maintained run keeps the version's strata.
   `RERAIL_CLIMB_ENDS=off` keeps them apart.
2. **Readers in the climb** (`closure::recognize`, `closure::keyed`). In the walk's component, a relation whose
   probes all read the walk's delta first, with a distinct slot in every column, is a reader. At each depth of the
   keyed climb (the seeds, then each depth's new rows), its rest runs on the depth's rows, and its rows go into its
   relation. With readers, the component takes the keyed form and runs in one wave.
3. **A row's tests are asked once** (`closure::take_tests`). Some atoms of an input are keyed only by the row's
   columns or constants, and bind nothing (`bound_in(s, x)`, `cls_late(n, x, s)`, `gbefore(n, x, s)`). Each such
   atom is taken out of every rest that has it, with either sign. The same source, index and keys make one shared
   test.
   - At each depth, a rest runs only on the rows that pass its tests.
   - A test is asked of a row only when a rest still needs it there, and once, whatever rests and signs read it.
   - A reader left with no step makes its head from the row's columns: nothing runs for it.

   `RERAIL_ROW_TESTS=off` leaves the tests in the rests.

So each reference's row asks `bound_in` once, `cls_late` once, `gbefore` where it is not bound, and the axis where
it climbs: W's questions, read from the rules.

## Test

`a_walks_readers_are_answered_in_its_climb`: the walk of 0523's test with `look` and `miss` beside it, the walk
last in the component's relations, as a stratum's names sort. The rows of all three are a search's and
semi-naive's, in one wave.

## Measured

s1000, M1 Max, rows unchanged (every stage equals W's):

| | before | readers in the climb | and each test once |
|---|---|---|---|
| instructions | 321.9 G | 322.8 G | 311.0 G |
| lookup stage (rules) | 1.48 s | 1.65 s | 1.18 s |
| `walk` and `look` | ~0.70 s | 1.09 s | 0.67 s |
| the keyed climbs (all programs) | 458 ms | 752 ms, the readers 359 | 450 ms, the readers 273 |

The readers alone were slower than `look`'s join: they asked each row again what the climb asks. With the tests
shared, the climb and its readers take what the climb alone took.

`all` (3,089 files), the same host, rows unchanged (t50's known `ref` and `open_name` differences only):

| | before | after |
|---|---|---|
| instructions | 3.165 T | 3.159 T |
| peak memory | 9.89 GB | 9.14 GB |
| lookup stage (rules) | 11.74 s | 12.19 s |
| `walk` and `look` | 5.22 + 0.93 s | 6.35 s |

On `all` it is even, not a gain. The keyed climbs take 4.35 s, the readers 2.56 s of it. The readers ask the shared
tests first, about 9.8 M probes at about 260 ns each. That is the executor's per-row work around each probe
(`run_join_each_row`'s frames and callback), not the probe itself. (Wrong: asked directly, the probe is most of it;
"Left".) The memory is `look`'s index of `walk` and the
rows it no longer holds: `walk` is now read only in its own stratum, so the run releases it there (`last_reads`).

## Left

- **A test asked directly: built, a small gain.** A shared test is asked as its index's probe alone
  (`cands::holds`, with `candidates` moved out of `run_join_in_each` into `setfix/cands.rs`), not as a join of one
  atom. `RERAIL_DIRECT_TESTS=off` asks it as a join. On s1000 (M1 Max, rows unchanged) the readers went 271 →
  244 ms and 309.8 → 306.2 G instructions. Split, the readers are 191 ms of tests and 47 ms of pushing their rows.
  So the executor's frames around a probe were about a tenth of a test's cost, not most of it, as "Measured" guessed.
  The rest is the probe, the probe structure's job (below).
- `walk`'s rows are still pushed during the climb, as its set check. They are released after its stratum.
- W's probe is a hash probe. Cheaper probes are the probe structure's job (t50 step 5, the spike; no sorted hash,
  the user, 2026-11-21).
- Metal: the device refuses a keyed closure, as in 0523.
