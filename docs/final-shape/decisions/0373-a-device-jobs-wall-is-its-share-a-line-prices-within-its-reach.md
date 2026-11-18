# 0373. A device job's wall is its share of the lane; a learned line prices only within its samples' reach

Status: accepted (2026-11-17)

## Intro

Since 0344, the GPU lane runs a matcher wave while the next is encoded, so two waves are often in flight at once.
Work queue Q50 left an open item: placement still sent about half of an unpinned s1000 run's waves to the cores.
The waves' estimates summed to 2.36 s on the GPU lane, which was busy for 1.00 s.

Two faults made the price wrong, and this decision fixes both.

## What was wrong

1. **Shared time was counted twice.** While two waves ran on the device, each job's wall included the time the
   other one held the device. The scoreboard fed that wall to the run's learned rate (`price::observe`). It also
   fed it to the lane's outstanding seconds, which placement adds a new launch's time to. So the GPU looked about
   twice as busy as it was.
2. **A line from small launches was stretched to large ones.** A run's first waves are small: 6.7e4–1.3e5 units,
   against 6e6–3e7 for the large ones. A Theil–Sen line through a narrow range of sizes has a slope that is mostly
   noise, and it was used up to 400 times past its largest sample. Two s1000 runs show the damage:
   - The cores' line was flat, about 0.5 ms a launch with almost no slope. A wave of 6e6 units was priced at
     6.5 ms and took 200 ms, so large waves went to the cores.
   - The GPU's line from 4 small waves had a slope of 2.7e-8 s a unit, against 6e-9 on its large waves. 14 of 18
     waves went to the cores: 3.2 s of wall there, against 0.6 s for the same work on the GPU.

## Decision

1. **The share clock** (`score::Unit::share_clock`). On a lane with one slot (a device), the clock advances by
   dt / n for each stretch of dt seconds during which n jobs are running.
   - A job's share is the clock's advance from its start to its end, so the shares of a lane's jobs add up to the
     time the lane had any job running.
   - The share replaces the job's own wall in the run's learned rate, the forms' actual seconds, the lane's busy
     seconds and the stored sample's `wall`.
   - A pool lane's jobs run on threads of their own, so they keep their wall.
2. **A line prices only within its reach** (`price::SPREAD`, 2). A learned line prices a launch only when:
   - its samples' sizes span at least 2× (largest over smallest), and
   - the launch has at most 2× the largest sample's units.

   Otherwise the price starts from the line's value at the largest sample, and grows with the smaller of two
   slopes: the prior's, or that sample's own rate a unit (`Learned::at`). That keeps what was measured at the
   samples' size, and past it a price grows no faster than either source says. The profile's rate lines print each
   line's largest sample and its reach.
3. **The stored cost statistics are not migrated.** Their GPU samples from before this change hold the doubled
   walls. The file is shared with other worktrees' binaries, which still write those walls, so a one-time delete
   would neither clean it nor be ours to do. A run's own rate replaces a stored one after 4 waves, and benchmark
   runs use a cost file of their own.

## Measured

internal_dead_code at s1000 with Metal, unpinned. The two engines ran in turn at load 57–98, each with a fresh
cost file of its own. Each engine was warmed by an s10 run first. The findings were the same in every run.

| Engine | Runs | Waves' wall on the cores | Run (CPU / wall) |
|---|---|---|---|
| Before (each job's own wall, lines stretched) | 4 | 2.74 and 3.22 s in 2 runs; 0.26 s in another | 11.1–18.1 s / 3.1–5.8 s |
| The share clock only | 2 | 0.65 s, then 3.22 s (the GPU's narrow line) | 11.2–14.7 s / 3.3–4.8 s |
| The share clock and the line's reach | 3 | 0.35, 0.01, 0.01 s | 11.0–12.2 s / 3.1–3.7 s |

With both changes, one run's GPU lane estimated 0.84 s for all its jobs, and their shares came to 0.80 s busy.
Before, a run pinned to the GPU estimated 2.36 s against a busy span of 1.00 s.

## Consequences

- The cost model's stored training samples now hold a device job's share, which is what placement adds up.
- A cold run with no stored statistics keeps its large waves on the GPU from its first waves on. Before, it
  depended on whether a large GPU wave happened to run early enough to correct the line.
