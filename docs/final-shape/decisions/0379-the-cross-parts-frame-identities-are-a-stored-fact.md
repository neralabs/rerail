# 0379: The cross part's frame identities are a stored fact, read before the first wave

Status: built (2026-11-17), measured on ep2 below. Q19's identities (work queue Q18).

## Intro

A run of the cross part reads only the frames its programs can read (Q19): each derived relation's identity
(`plan::identities`, hashed from its rules and what they read, by the program's components) and each input's. The
run computed that set on every run (`retain::frame_rels`, 0.031 s on ep2), for the cross program and the maintained
one. Then the kept part computed the maintained program's strata identities again (`Retained::from_frames`,
0.013 s). Both are functions of the programs alone, and on a rule edit the programs were the last run's.

## Decision

1. **The identities are kept as stored facts** (`retain::kept_identities`, through `parts::keep_facts`), each
   under a key of everything it is made of: the engine build, each program's version (its identity, digest and
   maintained parts), and the components, binds and bound relations it was made under.
   - Each program's strata identities are kept under the program and its components (`strata_idents_kept`).
   - The cross part's whole set is kept under the programs, `binds` and `bound` (`frame_rels_kept`).
   A run reads them where kept, and otherwise makes them and keeps them behind the run.
2. **They are read before the first wave** (decision 0104), where the run reads the cross part's frames. The
   strata identities are handed to the session's kept part (`Retained::from_frames_named`), matched by the
   program's digest. A reader inside the waves (a cone's rounds) computes its own as before: the store is not read
   there.

## Measured

ep2 on the m3 (300 files, 317 rules, `RERAIL_PREPARE=parts`, an edit of B101's body, profiled). Per edit:

- the cross part's frame identities: 0.031 s → 0.010 s (the two kept facts read, 0.007 s);
- the kept part's identities (`Retained::from_frames`): 0.013 s → gone, handed over from the kept fact (e68);
- rows: 1,334, none differing from h1.

## What it does not do

- The read of the cross part's space itself (0.028 s) admits all 33,534 kept frames, of which none is left out. Its
  cost is the admission, not the identities.
