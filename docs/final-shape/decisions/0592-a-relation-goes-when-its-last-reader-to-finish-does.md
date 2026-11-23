# 0592: A relation goes when its last reader to finish does, not when the loop passes its last reader

Status: built (2026-11-21). The user asked for it, as the next step of the footprint work (2026-11-21): "Next I'll
make release happen when a relation's last reader finishes, not when the in-order loop reaches it." Follows 0113 and
0114 §1.

## Intro

The strata loop drops a relation the run made once no later stratum reads it. Until now it did so by position. Each
relation's last reader is a stratum index (`last_reads`). When the loop reaches the stratum after it, the relation is
released (`after_last`, `carry`).

Since 0113, strata do not run in the loop's order. The scoreboard issues each stratum as soon as the strata it reads
have retired, as part of a component that runs as a job. Jobs end in any order (0114 §1), and `collect` absorbs each
as it ends. The loop itself still walks the strata in program order. At each stratum it waits for that stratum's
component (`retire_at`).

So while the loop waits on one slow stratum, many later strata may already have run, and all their reads are done.
Every relation those strata read stayed held anyway, because the loop had not reached the last reader by position. The
peak footprint was set by the program order, not by the work still to do.

## Decision

A relation is released when the last of its readers to finish does, in whatever order they finish.

- `ReadsLeft` holds, per relation, how many of the strata that read it have not finished. It counts the same reads as
  `last_reads` (`stratum_reads`).
- A stratum finishes in one of two ways:
  - Its component's job ran it. `Board::taken` records it, and `take_finished` hands it over.
  - The loop passed it (`passed`). This covers strata that ran on the loop's thread, waiting strata whose wave is
    flushed before any reader of theirs, and SQL strata.
- A stratum suspended on its requests (0114 §3) is not finished while it asks. Its fixpoint reads its inputs again
  when the answers come. It is counted once it no longer asks.
- The loop releases from both points:
  - at each stratum, before it runs;
  - inside `retire_at`'s wait, each time a job ends. This is where the gain is: the loop is blocked on stratum `at`,
    while later strata end and their reads go.
- `release_unread` releases only what the loop's own release would release:
  - a native relation the run made;
  - not one that stays (outputs, kept relations, gates' relations, what a view that stays reads);
  - not an input (inputs are unloaded by their own schedule), and not a view;
  - not one a stratum still asking reads.

  A relation released here is no longer native, so the in-order release passes it by.
- A stratum of views is read where its readers read it (`last_reads` moves its reads to its last reader). What such a
  stratum reads is not counted here and stays with the in-order release.
- The serial loop (`RERAIL_STRANDS=off`) and maintenance runs have no board. There, finishing and position are the same
  thing, and nothing changes.

The profile line "relations released when their last reader finished (decision 0592)" counts the relations
released this way, how many of them went before the loop passed their last reader, and by how many strata in all.
`RERAIL_RELEASE_FINISHED=off` leaves the loop's in-order release alone, for comparison.

## Measured

The bench is the M3 run of 300 files at a 1,024 MB memory cap (ab141): one binary, a warm run, then one measured run
with the change and one without. Each line below is one run.

| | on | `RERAIL_RELEASE_FINISHED=off` |
|---|---|---|
| Peak footprint | 3,355 MB | 3,587 MB |
| Relations held at the peak | 675 | 1,203 |
| Their rows | 26.4 M | 35.9 M |
| Their values | 175 MB | 376 MB |
| Metal driver at the peak | 2,683 MB | 3,132 MB |
| Host blocks at the peak | 1,013 MB | 1,412 MB |
| Violations | 463 | 463 |

In the main chunk, 1,929 relations went this way. 1,629 of them went before the loop passed their last reader, 891
strata early on average.

The footprint left is not the relations' values (175 MB). It is the blocks and the Metal driver's memory that hold
them. That is the next thing to look at.

## Tests

`a_relation_goes_when_its_last_reader_to_finish_does` (plan's tests):

- readers that finish out of order;
- a relation shared by two readers;
- a stratum still asking holds its reads until it stops asking;
- a stratum counted twice counts once.

The strata loop's tests (plan, session, oracle, corpus, maintain, strata, issue) pass with the change.
