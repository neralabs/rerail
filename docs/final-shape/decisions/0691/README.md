# Decision 0691: its pages

These are the pages written while decision 0691 was worked out. Each is kept as it was when it was written, so its
numbers are those of its date. The decision itself, [0691](../0691-what-crosses-to-the-cross-file-part-is-checked-on-main.md),
holds the final state.

GitHub shows the Markdown page here. It shows an HTML page as its source. To read an HTML page, open it from a
checkout of the branch, or download it with the file view's "Download raw file" button and open that in a browser.

| Page | What it is | As of |
|---|---|---|
| [survey-of-the-bend-and-oscar-branches.md](survey-of-the-bend-and-oscar-branches.md) | How the bend and oscar branches enforce the invariants, against main's split. The survey this port started from. | 2026-11-25, before the port |
| [cross-file-offenders.html](cross-file-offenders.html) | Every pack's offenders on main at `d54013c841`, in three buckets: fixed on oscar (port), fix known, needs a redesign. | 2026-11-25, main before the port |
| [per-file-cross-file-rules.html](per-file-cross-file-rules.html) | For rule authors, and for models that write rules: the two parts of a program, the three rules, how rules break them, a fix for each kind of break, and how to check. | 2026-11-25; still current |
| [split-port-decisions.html](split-port-decisions.html) | Two choices the port needed: points-to (the branch's fix 3e) and a producer for `spec_resolved`. Both were then built in this pull request (fixes 11 and 12). | 2026-11-25, after fix 10 |

## Runtime against the merge base

Measured on 2026-11-26, at `60736390bd` against the merge base `d54013c841`. Both ran the CLI as a profiling build,
each reading its own tree's libraries and rules, from a fresh store, over the same files. Every pack's policies ran
as one batch, two rounds each. The policies a side cannot run (main's 25 failing ones) were left out of both. The
machine was shared, so the instructions retired are compared, not wall time.

| Pack | Files | Instructions, base → this branch | Peak MB, base → this branch | Findings |
|---|---|---|---|---|
| rust | 117 | 98.3 → 40.2 G (−59%) | 933 → 592 | 267 = 267 |
| next | 120 | 77.5 → 47.5 G (−39%) | 640 → 448 | 1 = 1 |
| django | 120 | 281.0 → 184.1 G (−34%) | 1,491 → 888 | 2 = 2 |
| python | 120 | 981.1 → 707.8 G (−28%) | 2,460 → 1,272 | 3,058 = 3,058 |
| react | 120 | 380.4 → 311.6 G (−18%) | 886 → 761 | 97 = 97 |
| rerail | 762 | 11.6 → 10.0 G (−13%) | 176 → 169 | 0 = 0 |
| vue | 9 | 10.1 → 8.9 G (−12%) | 154 → 146 | 5 = 5 |
| typescript | 84 | 162.9 → 145.1 G (−11%) | 854 → 684 | 2,168 → 2,166 |
| node | 84 | 130.6 → 115.8 G (−11%) | 536 → 475 | 295 = 295 |
| sql | 80 | 6.3 → 5.8 G (−9%) | 174 → 163 | 0 = 0 |
| javascript | 84 | 267.6 → 247.9 G (−7%) | 1,048 → 1,043 | 4,585 = 4,585 |
| php | 74 | 10.1 → 10.2 G (+2%) | 222 → 213 | 241 = 241 |
| complexity | 762 | 28.8 → 30.6 G (+7%) | 247 → 246 | 558 = 558 |
| rr | 762 | 4.5 → 4.9 G (+8%) | 156 → 149 | 0 = 0 |
| blade | 38 | 3.1 → 3.4 G (+10%) | 131 → 123 | 2 = 2 |

- **TypeScript's two fewer findings** were false findings on main. A local arrow function named `settings` was
  reported as an exported one.
- **Complexity's 7%** is mostly phase 1's constants fold: a nested run of about 16 ms on a cold store, which keeps
  its result (see fixes 18 and 19 in the decision). The four packs that are slower each take under 3 s of CPU.
- **python-main** is not in the table. Its whole batch passes 8 GB on both sides, even over 15 files, and one shard
  stops with `no declaration of within` on both.
- **Before fixes 18 and 19**, Django was 45% slower than the merge base and rerail 52% slower. Both now run faster
  than the merge base.
