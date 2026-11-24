# 0664. An index another batch took is never given back under it

Status: done (2026-11-23).

## Intro

A device batch that needs an index of a relation looks in three places (`Batch::index_of`):

1. an index built already in the relation's block;
2. one this batch is building;
3. one another batch is building, its work committed (an `InFlight` in the block's list). The taker's work is
   committed after the builder's, so it runs after it on the one queue.

The 10-file django run on the device (`RERAIL_DEVICE=always`, `RERAIL_DEVICE_NARROW=0`) gave other findings than the
CPU's in about one run in seven: one RUF059 more, or all 389 TC006 findings lost, or both. Decision 0661 put the first
such run down to a string join's row stage bound to the view format. That was wrong: the runs since had only been lucky.

In each bad run one probe of `home` by its first two columns found no rows: `ibind`'s (459 -> 0), or `otgt`'s
(44 -> 0). `home` held its 862 rows. Then `qn` was empty, or `bound_in` lost its `otgt` rule, and the findings that
follow from them changed. In the good runs `ibind` built its own index of `home`. In the bad ones it took `otgt`'s
batch's, in flight.

## Three gaps, and what each does now

### A build behind a round end was published

A recursive component's program may stop at a round end (0603, 0657): a build queued after it never runs, and its
room goes back to the block at `finish`. Such a build was told to the block as in flight, and a batch that took it
probed empty buckets. It is no longer told (`submit_open`: `behind` is none).

### A taken index was given back under its taker

When its batch is done, a builder gives its index back if the same index was marked built meanwhile ("built twice").
The CPU builds one of its own when it finds none marked, and a batch that built its own may finish first.
`give_back_index` clears the index's entry in the relation's directory: its spans of buckets and rows are set to 0.
A taker's stages read the index through that entry, and its work can run after the builder's batch is done on the
host. So it read an empty index. The bad runs were this.

Now a taker marks the `InFlight` it takes (`taken`), and a builder whose index was taken never gives it back. It marks
it built beside the other, which is as good: two indexes of one key, either of which a probe may find. In the real
runs the counter "device.index built twice, kept: another batch took it" counts a few such indexes in about half the
runs.

### A builder's program may park before its build

A program parks at a phase whose room it cannot know (a size script, an arena's claim), and its later phases run in a
later submission of the same batch (`finish`'s `again`). A build published with the first submission is then built
after a taker's work, which only the first one orders.

A taker now checks what it read. At its reservation the room's last bucket offset is set to `UNBUILT` (`u32::MAX`),
which no count of rows is. A build writes the index's rows there (`ix_offs_one`). The taker's program copies that word
at the start of the level that probes the index (a `Sum` of one part), and its `finish` fails the batch when the copy
is not the index's rows: its strata run on the CPU. The real runs count 1 to 10 such batches a run ("an index taken in
flight, read before it was built"). A taker that instead waited for the builder's build would need the device to wait
on a build whose submission is not yet committed; that is not built.

## Found on the way

A profiled run whose day-old cache of the machine's roofs had lapsed ended under `RERAIL_SQL_STRICT`: the roofline's
probe of SQLite's step cost runs SQL in a database of its own in memory. It now runs behind the run
(`profile::behind`), as the store's writer does.

## Tests

- `a_batch_takes_an_index_another_batch_has_committed` (metal_join) gained the race: a batch takes another's index
  in flight, the host builds and marks the same index, and the builder finishes first. Its index is kept, built, with
  its rows; given back, its entry was cleared.
- The 55 tests of `setfix::device` and `metal_join` pass on Metal (M3, M1 Max) and on the CPU's device (a Linux host,
  `RERAIL_GPU=cpu`).
- The 10-file django run on the device, repeated: 24 runs on the M1 Max have the CPU's findings. One more ended at its
  120 s alarm while the farm removed 33 GB of build targets on that host. Before the fixes about one run in seven
  differed on the M3.

## At the hub merge (2026-11-24)

The hub fixed the same race on its own (`b5d3983148`, `ac66bcb1f8`): `InFlight::lent` is this record's `taken`, and
`InFlight::whole` fails a taker whose builder's program stopped before the build (an arena passed, a queue's room).
The merge keeps the hub's mechanism as the one definition: `lent` replaces `taken`, and a taker waits for its
builder's work on the host before it encodes a read (the hub's). This record's check of what a taker read is kept
where the hub's does not reach, a builder that parked before the build (its rest a later submission's), and it is now
a host read: once the builder's work is done, the room's last bucket offset still `UNBUILT` fails the taker's batch
to the CPU. The device copy (`check_taken`) and its words are gone.
