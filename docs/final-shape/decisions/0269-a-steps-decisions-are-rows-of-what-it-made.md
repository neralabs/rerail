# 0269: A step's decisions are rows of what it made

Status: built (audited 2026-11-23). Decided (2026-11-15). Builds on 0060 §3 (the prepared program kept for the
process), 0073 and 0155 (kept in the store, as facts and texts), 0103 (a cache is two rules), 0246 (loading as module
images and derivations) and 0264 (rule images made when the engine is built). Agrees with 0251: a run that reuses
something records what a run that made it records.

## Intro

A profiled run prints the decisions the engine made on the way: how demand split the program, which closures the
reach index holds, how many rules run as the transducer's matchers. Most of them are made while the program is
*prepared*: pruned, simplified, demanded, lowered and split.

A run usually does not prepare. It reuses a program prepared earlier, from the process's memory or from the store,
and with 0264 from the images shipped with the engine. So its profile showed none of the decisions behind the
program it ran.

The first fix (2026-11-07) kept a copy of the profile's output. Before compiling it took the profile's counts, after
compiling it took their difference, kept that with the program, and a reuse added it back to the profile:

```rust
let before = profile::decisions();
// … prepare …
let decisions = profile::decisions_since(&before);   // kept with the program
// a later run, reusing it:
profile::add_decisions(&compiled.decisions);
```

That was a memo of a side effect, and it was wrong under load: the profile's counts are the whole process's, so a
decision another thread made in the window became the program's, and was stated again by every reuse. It also kept
nothing when the process that prepared did not profile. When the compiled-program cache gave way to the prepared
program's reuse (0073), the replay was left behind, and the profile went back to showing nothing on reuse.

## Decision

### 1. What a step decides is part of what it makes

A step that makes something a run may reuse (a prepared program, and with 0246 each derivation of the image) keeps
its decisions with it, as one more relation:

```
prepared_decided(k, topic, text, count)     -- beside prepared_demanded, prepared_pushed, …
```

They are written where the step's other rows are written, read back where they are read, and keyed by the same key:
nothing else stores them, and nothing replays them. A decision that is a function of rows the step already keeps is
printed from those rows rather than stored twice; the prepare step's decisions today are counts of rewrites it made,
which no other row holds, so they are stored.

### 2. A step collects its own decisions

The step opens a collector (`profile::Deciding`) and closes it when its output is complete. Every decision made on
the step's thread while it is open is the step's, and an enclosing step's too. A step that fails keeps nothing; its
enclosing step keeps what was decided.

Inside an open step a decision is made whether or not the process profiles. The kept rows are then a function of the
step's inputs, not of how the process that made them was run. Outside a step, a decision is made only when
profiling, as before.

The prepare step's passes decide on its thread (a pass that uses the pool, such as `simplify`, counts there and
decides after). A decision made inside a pool job is the profile's alone.

### 3. A reuse states them, marked

A run that reuses a step's output states its rows as the run's decisions (`profile::reused`), with where the output
came from:

```
export demand:
  - future_annotations: 2 reads, cut by 1 rules  (kept: store)
  - stub_file: 2 reads, cut by 1 rules  (kept: store)
```

They enter the same decisions the run makes, so every report that reads decisions reads them. The reused program's
own line ("program prepared: read from the store") is unchanged.

### 4. A measurement's steps take the decisions

A measurement that wants each of its steps' decisions (the O125 replay) takes them (`profile::take_decisions`): the
next take returns what was decided after. `decisions_since`, the difference of two snapshots, is removed.

## As built (2026-11-15)

- `Prepared::decided`, written as `prepared_decided` by `prepared::remember`, read back by `prepared_of`. The
  pointer's schema is 106 (was 105): every stored prepared program is prepared once more.
- Where `corpus::run_observed` prepares (its `None` branch), it opens `Deciding` first and closes it as it builds
  `Prepared`; both reuse branches (memory, store) call `profile::reused`.
- Checked with a pack rule (`ruff-up006`) run three times in three processes, profiled: cold, the same again, and
  after an edit. The third read the program from the store and stated 25 decisions as kept, each one the cold run
  made.
- A profiled run that the whole-run cutoff answered wrote no profile, and then failed appending the run's lifecycle
  to it (`profile lifecycle: No such file or directory`): it now writes its profile.
- Tests: `profile::tests::a_step_keeps_its_decisions_and_a_reuse_states_them` (nested steps, a failed step,
  profiling off, the explain line); `a_prepared_program_reads_back_from_its_text_and_facts` checks `decided` is made
  without profiling and reads back.

## With 0246 and 0264

When preparation becomes derivations of the linked image (0246 §4), each derivation keeps its decisions as rows of
the version it makes, by this record, and images made when the engine is built (0264) carry theirs. Nothing in this
record depends on `Prepared`'s fields: only on a step's output having rows.
