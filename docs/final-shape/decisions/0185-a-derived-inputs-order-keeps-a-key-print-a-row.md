# 0185: A derived input's order keeps a key print a row, so a probe reads prints, not rows

(Numbered 0176 on its branch, jeremybarnes/final-rerail-shape-phased-facts; renumbered at the roll-up.)

Status: built (2026-11-13), `setfix/runs.rs` (`Run::prints`, `print_span`, `Index::printed`) and `setfix.rs` (the
derived input's index). It narrows how decision 0112's derived-input order is probed (cd6df3ee4).

## Intro

**What changed in 0112.** A derived relation's index used to be a hash table on its key columns: 18 bytes a row
kept, 46 while built. Since cd6df3ee4 it is a sorted order of row ids on those columns, 4 bytes a row. A probe
finds its key's window by the order's fences (the leading value every 64 ids), then binary-searches the window.

**What that cost.** Each step of the binary search reads a row. The rows are in the order they were made, not in
key order, so each read lands somewhere else in memory: about 14 scattered reads a probe, where the hash table
took one. On the O125 replay's cold run (optimized build, unprofiled), the cross part retired 9.21 G instructions
with the hash table and 9.66 G with the sorted order: 0.45 G, 4.9%.

## The print

Each id in the order gets one more byte beside it: 8 bits of a hash of its row's key (`print_of`). The probes of a
derived input always give the whole key (the order is built on exactly the probe's columns), so a probe:

1. finds the window as before (fences, or a tagged run's tag span);
2. scans the window's prints for the key's print, eight at a time, as word operations (`first_print`);
3. reads the row at the first match. A print equal by chance (one in 256) is passed over;
4. the key's ids sit next to each other in the order, so the span runs while the prints match. Only its last
   row is read, to drop matches by chance at its end.

```text
ids     … 812  77  4091  9  1204  33 …     sorted by key
prints  …  a3  5e    5e 5e    5e  c1 …     key 7's print is 5e
                     └─ 77, 4091, 9 hold key 7; 1204 is 5e by chance
```

A probe reads about two rows instead of about fourteen. The prints are made in row order, as the run is built
(the rows read in turn), and placed as the ids sort. A merge carries them with the ids. Orders probed by a prefix
of their columns (kept chains) are not printed.

- **Bytes:** 4 → 5 a row for these orders, inside 0112's 4.5–8 target; the hash table's 18 are not back.
- `RERAIL_KEY_PRINTS` (measurement aids): `off` probes the sorted order by its rows, `hash` builds the hash table
  of before 0112.
- Tests: `a_printed_order_finds_what_the_rows_find` (`runs.rs`) compares every find against the unprinted order:
  one, two and three columns (tagged), keys present and absent, sorted and shuffled batches, delta probes, after
  the runs merge, and the byte count. It fails when the span's end is not checked (matches by chance kept).

## Measured

O125 replay, `--profile profiling`, unprofiled, one binary in its three modes, two alternating rounds, "same as
full" in all six. Cross part, G instructions retired:

| | cold | 06 | 07 | 08 |
|---|---|---|---|---|
| hash table (before 0112) | 9.23, 9.19 | 4.18, 4.18 | 6.30, 6.36 | 7.45, 7.43 |
| sorted order | 9.65, 9.67 | 4.09, 4.10 | 6.53, 6.46 | 7.71, 7.70 |
| sorted order, printed | 9.53, 9.50 | 4.13, 4.26 | 6.44, 6.45 | 7.67, 7.77 |

The steps differ by about their noise (load 90–100). On the cold run the prints take back 0.14 G of the 0.45 G.

**Where the rest is.** Profiled, the same binary: the joins cost the same with prints as with the hash table
(0.640 against 0.636 s, 574 calls, the same 4.468 M rows of work). What is left is the build: 385 derived-input
indexes, 0.023 s sorted against 0.0095 s hashed. The trace (`RERAIL_SPIKE_INDEX`) puts the large ones (109 of 10k
rows or more, 4.7 M rows, 99 of one key column) at about 18 ns a row sorted, 6.5 ns hashed.

**What was tried and not kept.** A radix sort for orders of one or two columns (stable, by bytes, constant bytes
skipped): the large builds went from 18 to 8.4 ns a row, but it retired more instructions than the comparison sort
(its histograms and passes are instructions; the comparison sort's cost is mostly waiting on memory). Cold: 9.53 G,
no change. Edit steps rose 0.1–0.2 G: they sort many runs of a few hundred to a few thousand rows of wide values.
Removed.

**Open.** The build's remaining cost is the comparison sort over 24-byte keys. Building fewer orders (an order
made when first probed, not at open) or sorting fewer bytes a key would be the next levers. Both are built in
decision 0186 (the builds' CPU halves).

**On the earlier numbers.** The regression was first measured on debug builds (cold cross 75 → 82 G, 11%). Debug
code makes each probe's search about eight times as costly, so the share was overstated. The optimized build's
figures above are the measure.
