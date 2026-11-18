# 0378: A kept input's documents are laid out where first asked, from what its frame recorded

Status: built (2026-11-17), measured on ep2 below. Work queue Q91 (Q18's kept inputs).

## Intro

A run that keeps the cross part's inputs (decision 0117) reads each input's frames back: its documents frame (each
owner's digest and rows' count) and its granules (the rows, grouped by owner). From the documents it lays the input
out: each owner's range of rows, the granules that hold them, and the rows' total the planner reads (`Lazy::len`).

A rule edit changes no file. Of its 1,008 kept inputs it reads only each one's digest, and, where it reads the kept
statistics, the stats part. Yet every edit decoded and laid out all 1,008 documents frames (0.077 s on ep2), and
the kept part decoded them a second time to inherit them (`after`, 1,010 decodes).

## Decision

1. **The keeper records each documents frame's granules where it writes it** (`write_input_frames`): a read row
   `#granules` with the mask of the granules that hold rows. The rows' total is then the granules' rows, which
   their parts give undecoded (`len`). This is a change to the frame's format. A frame written before it has no
   mask, and its input is laid out at once, as before.
2. **An input's layout is made on first use** (`InputLayout`, a `OnceLock`): its documents, each owner's range, and
   each granule's owners. It is checked against the granules the frame recorded. An input whose layout fails tells
   it where it is read, as `Lazy` tells a granule that fails.
3. **A kept part's documents are handles** (`FramedDocs`), decoded where first asked. They are carried from one run
   to the next undecoded, and are held as they are where a run hashed them itself.

## What it does not do

- The granules' parts are still admitted for every kept input (0.013 s), since `Lazy` reads their lengths.
- An input whose documents a run reads (an evaluated owner's patch, a changed file) is laid out whole.

## Measured

ep2 on the m3 (300 files, 317 rules, `RERAIL_PREPARE=parts`, an edit of B101's body, profiled). e57 was the cold
run; e58 and e59 were edits. Per edit:

- the kept inputs: 0.077 s → 0.021 s (the stats parts and granules admitted, 0.013 s of it);
- the kept part's documents decoded to inherit them: 1,010 (0.020 s) → none;
- `cross`: 0.11 s → 0.09 s;
- the edit: 2.16 s CPU (e56) → 2.17 s (e59), in the run's noise;
- rows: 1,334, none differing from h1.
