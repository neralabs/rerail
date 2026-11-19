# 0019: walks carry four cursors; passes of one domain share their roots

Status: accepted (2026-10-31)

## Context

After 0018, the S7 Metal wave (250 files) still had problems with tasks and memory:

- **Interpreted tasks.** 67 k tasks still ran on the interpreter. They needed two workspaces of
  ~900 MB each: final captures 288 MB, pending 208 MB, results 160 MB.
- **A misleading refusal histogram.** `Micro::refusal` named the first TEXT op even where `of`
  accepts it. Corrected, it showed that every refused start came from a program saving a third
  cursor.
- **Peak RSS** went from ~490 MB to ~1.2 GB during labels (`prof::peak` by stage).
  - **The plan:** +380 MB. Each of 605 passes held its own root list, 70 M roots in all, yet there
    are only 264 distinct (domain, entry, active tapes) keys.
  - **The GPU runs:** +320 MB. The walks' `items` buffer copied those roots again.

## Decision

1. **Four cursors.** `CURSORS` = 4 in `micro.rs`, and `MicroFrame.cursors[4]` in MSL. 576 of 579
   label programs are now walks, and no interpreter workspace is allocated in S7.
2. **Shared roots.** In the plan, `labels_metal` shares root lists behind an `Arc`, keyed by
   (domain or label bound, entry tests, the tapes the clause is active on). Plan roots fall from
   70 M to 10 M.
   - In `size()`, walk passes with the same list share one item range.
   - Each `MicroPass` carries its first thread (`threadStart`, formerly `pad`). `label_micro` finds
     the pass by thread and reads `items[itemStart + tid - threadStart]`.

## Evidence

- **Walk differential:** walks equal the interpreter (`micro_walks_are_the_interpreters_label_outputs`).
- **Gates:** S2 Metal parity is 87 of 87; S1, S4–S6 and vtab pass.
- **S7 against W:** 710 keys against W's 710 on both backends, and both mutations fail the gate.
- **A first attempt shared item ranges** without per-pass thread starts. It found 4266 keys, and
  the gate caught it.

S7 on Metal, before (0018) → after:

| stage | before | after |
|---|---|---|
| peak RSS through labels | 1197 MB | 885 MB |
| process peak | 1.25–1.4 GB | 1.15–1.2 GB |
| `demand.labels` | 2.9 s | 1.7–2.1 s |

S7, 250 files, load ~40:

| backend | wall | CPU (user+sys) |
|---|---|---|
| CPU | 27.4 s | 28.0 s |
| Metal | 7.6–8.1 s | 13.9–15.1 s |

## Open (O30)

- The process starts S7's run at ~400 MB already, because it holds W's run and the 250 tapes.
- The engine still adds ~700 MB:
  - plan: ~80 MB;
  - GPU runs: ~300 MB;
  - roots and execute: ~220 MB.
