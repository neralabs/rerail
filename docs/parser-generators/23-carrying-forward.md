# Parser generators: JSON's pass 1 on Metal, carrying forward only

## Intro

Pass 1 of the JSON parsers (doc 21) carries state from each 64-byte block to the next. The
Metal kernel of doc 22 moves that state between SIMD groups by decoupled look-back: each group
publishes its carries and the next group waits for them. For JSON step 14 this takes 14
**exchanges** a group, one after another.

This page looks at other ways to move the carry, where no group ever waits on another:

- what the carried state is, and which parts of it forget their past (§1);
- starting each piece of a file from a guess, then checking and running again (§2);
- making only the long-memory carries exact, a level at a time (§3);
- a CPU-like form: one thread runs a piece of the file in order (§4);
- the same idea in lanes: one SIMD group runs a piece of the file (§5);
- what is left (§6).

The machine, batch and checks are those of doc 22:
- an Apple M5 Max with 40 GPU cores;
- simdjson's 17 files, repeated 4 times: 68 files, 62 MB;
- every run's outputs, positions and totals equal Cranelift's.

Times are GPU times, the fastest of 5 runs. The machine was shared, at load averages of 70
to 160. At 62 MB a batch runs as two halves of 31 MB, and the times add both halves.

## 1. The carried state

A block's carried state has two kinds of part:

- A `prev` reads the last k bits of a value from the block before. It forgets in k bytes: a
  piece of the file started k bytes early from zeros has it right.
- A **scan** is a 1-bit carry, such as "inside a string" or "inside a number". A block's effect
  on it is a 2-entry table: the carry out for carry in 0 and for carry in 1.

A scan's table says how much state there is:

- When both entries are equal, the block **forgets** the scan. The carry out is known whatever
  came before: only one state is possible.
- When they differ, the carry out depends on the carry in: two states are possible.

Step 14 has 6 scans:

| scan | forgets | why |
|---|---|---|
| escaped (after a backslash) | within a byte or two | a run of backslashes ends |
| inside a string | never | it is a parity of quotes: a wrong guess is wrong until the file ends |
| inside a number, and three others like it | at the next token of their kind | |

The "inside a number" carry is the hard one. It is set by a number's first byte, and cleared
only at the next number's first byte. Between them its value is stale: no output reads it.
Through a long run of strings it stays as it was, perhaps for kilobytes.

The look-back kernel already uses the one-state case. A group whose own table forgets
publishes an inclusive state at once, and a look-back stops at such a group (doc 22).

## 2. Starting from a guess

The simplest forward-only shape works like this:

1. Each piece of a file (a **segment**) starts a few blocks early, from all zeros. These
   blocks are its **warm-up**.
2. A check compares each segment's state at its first block with its predecessor's state at
   its end.
3. A segment whose start differs runs again from its predecessor's end.

A run again goes on into the next segment for as long as its end differs from the end it had
before. This keeps an error from moving one segment a round.

Guessing every scan fails on JSON, in two ways:

- **Parity.** A segment that starts on the wrong side of a quote is wrong to its end, and so
  is every segment after it in the file. On 15.5 MB, about 20% of starts were wrong and
  the check took 3,250 rounds.
- **Stale state.** With the string carry made exact (§3), the number carry was still wrong in
  many starts. A wrong stale value changes no output, but it makes the check fail:

  | warm-up | wrong starts, of 8,077 |
  |---|---|
  | 1 block | 1,335 |
  | 2 blocks | 504 |
  | 4 blocks | 253 |
  | 8 blocks | 89 |

A run again is serial work, and it can carry a stale difference through a whole file. Such
runs cost 100 to 290 ms, against about 4 ms for the whole batch.

So a guess is good only for the parts that forget quickly: the `prev`s and the escape carry.

## 3. Exact scans, a level at a time

A scan's 2-entry table has the same size whatever the scan reads. So the exact carries can be
made in passes, without the rest of the state:

1. A pass over every segment makes each chosen scan's table for the segment.
2. A prefix over each file's segments turns the tables into each segment's exact carry in.

A scan that reads another exact scan needs that scan's carries first. Each exact scan has a
**level**: one more than the highest level of the exact scans it reads. Each level is one
pass and one prefix.

Inside a level's pass, the other parts of the state are guessed from a warm-up, as in §2.

Making every scan exact takes 5 levels for step 14:

| level | scans |
|---|---|
| 1 | escaped |
| 2 | inside a string |
| 3 | inside a number, and one other |
| 4 | one |
| 5 | one |

Making only the string and number carries exact takes 2 levels. With them, no start was wrong
on the batch, so nothing ran again.

Which scans to make exact is learned:
- A program starts with its toggles exact (the parity carries), since they never forget.
- After a run, a scan whose guessed start was wrong in more than one segment in 1,000 is made
  exact in the program's later runs.

On step 14 the first run has 258 wrong starts, all of them the number carry. That run is still
correct: its wrong segments run again. From the second run on, nothing runs again. Step 15 (with
comments) learns its number carry the same way.

## 4. The CPU's shape: one thread a segment

The first forward-only form runs each segment in one GPU thread, the state in registers, as
Cranelift's loop does on one core. The body of a block is the same generated MSL as the lanes
kernel's. Each block takes one of three bodies:
- the general body, at a file's edges;
- a full body, inside the file;
- an ASCII body, when the whole SIMD group's blocks are ASCII.

The order of work:

1. The two levels' passes and prefixes.
2. The main pass: each segment starts two warm-ups early.
   - At the first warm-up, the exact scans start from their carry there. The `prev`s start
     from zero.
   - At the second warm-up, the exact scans are set to their carry at that point.
   - The second warm-up exists so that the exact scans' operands, which read `prev`s, are
     right by then.
3. The check.
4. Positions and totals, from the finished output words: a count, a prefix and a write.

Each pass for one 31 MB half, step 14, with 16 blocks a segment:

| pass | ms |
|---|---|
| string carry's level | 0.19 |
| its prefix | 0.05 |
| number carry's level | 0.70 |
| its prefix | 0.05 |
| main pass | 2.4 |
| check | 0.01 |

The whole batch, with positions, takes 7.1 to 7.7 ms (8.0 to 8.7 GB/s). The look-back lanes
kernel takes 4.2 ms (14.8 GB/s) on the same batch.

One thread per segment is slow. Each thread runs its blocks one after another, so each block
waits for the one before it. The segment size trades threads against warm-up:

| blocks a segment | 4 | 8 | 16 | 32 |
|---|---|---|---|---|
| GB/s | 6.0 | 6.6 | 7.5 | 5.9 |

## 5. The same in lanes: one SIMD group a segment

The lanes form keeps the forward-only plan of §3 and §4, but runs each segment across one SIMD
group: block `a + l` in lane `l`.

- A `prev` takes the word of the lane before by a shuffle. Lane 0 takes zeros.
- A scan takes its lanes' tables' prefix, from the segment's carry: the exact carry, or 0 for
  a guessed scan.
- No group waits on another. There are no exchanges between groups.

A segment of S blocks uses S + 3 lanes:
- two warm-ups (as in §4);
- the segment's S blocks;
- the block after, whose bits finish the delayed outputs' last word.

With 29 blocks a segment, a group's 32 lanes are all used. The levels' passes, the runs again
and the positions are in lanes form too. The main pass counts each segment's output bits, so
the positions need only a prefix and a write.

Each pass for one 31 MB half, step 14:

| pass | ms |
|---|---|
| string carry's level | 0.21 |
| its prefix | 0.03 |
| number carry's level | 0.37 |
| its prefix | 0.03 |
| main pass | 1.14 |
| check | 0.01 |

Whole batch, 62 MB:

| shape | step 14 | step 15, with comments |
|---|---|---|
| look-back lanes (doc 22) | 4.19 ms, 14.8 GB/s | 5.52 ms, 11.2 GB/s |
| forward carry, one thread a segment | 7.1 to 7.7 ms, 8.0 to 8.7 GB/s | 10.1 ms, 6.1 GB/s |
| forward carry, in lanes | 4.10 ms, 15.1 GB/s | 5.51 ms, 11.2 GB/s |

One run of the lanes form took 2.96 ms (20.9 GB/s). Later runs at similar loads did not repeat
it, so the table uses the steady figure.

The segment size matters more here than in §4. A group always has 32 lanes, so a short segment
leaves lanes idle:

| blocks a segment (warm-up 1) | 29 | 22 | 14 |
|---|---|---|---|
| ms, step 14 | 4.1 | 5.2 | 7.9 |

The main pass alone runs at about 27 GB/s. The forward-only shape gives up the exchanges, but
it pays for two extra reads of the bytes (the levels' passes) and for more launches.

## 6. What is left

Ideas, in order of expected gain:

- **Look back for the two exact scans only.** A main pass could take its two 1-bit carries by
  look-back, as doc 22's kernel does for all of its 14 exchanges, and guess the rest. That
  drops the levels' passes, which read all of the bytes twice more. The estimate is close to
  the ceiling of doc 22's ablation, about 3.5 ms.
- **Fuse the string carry's level.** Its table needs only the quote and backslash bits. A
  pass over those classes alone could replace the full read of the bytes.
- **One command buffer.** A run again's size is known only on the device. Dispatching it from
  the device (an indirect dispatch sized by the check) would keep the check and the runs
  again in the first command buffer.
- **Compare only what is seen.** A run again goes on while its end state differs, even when
  the difference is stale and no output reads it. Bounding the hops, or leaving unread parts
  out of the compare, would keep one stale value from running a whole file serially.
- **Skip what a segment forgets.** When a segment's table for an exact scan forgets, its carry
  out is known without the prefix: one state only. Such segments could skip the prefix, and
  the level's pass could stop at them.
