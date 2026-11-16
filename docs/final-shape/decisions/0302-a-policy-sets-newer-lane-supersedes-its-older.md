# 0302. A policy set's newer lane supersedes its older: an edit's predecessor keeps no protection, and the program that reuses its frames holds them

Status: decided (2026-11-16, the user's choice among three). Builds on 0053 (corpus runs' bounded history), 0227
(collection by reachability), 0104 (one read before the first wave), 0197 (a warm run queues no empty job). Q19 step 2
in the work queue. Numbered 0301 until another worktree's 0301 (the planner's shared state) was found.

## Intro

A run of no phase keeps its cross-file part in one space per root. Collection (0227) drops a frame once no live
program holds its relation there, and a program is live while a protected manifest names it. A root's last runs are
protected by lane, and a lane is the programs' texts (`rerail.corpus-lane/v2`): policy sets run one process each over a
root keep their plans while the others run. Up to 64 lanes are protected.

So every rule edit was a new lane, and its predecessor stayed protected, up to 64 of them. On ep2 (a 1,208-file
corpus, one B101 body edit at a time) all 357 plans were live, 31 programs held the cross space, and the space held
191,787 frames over 56,157 identities. The run reads the space by identity, but the read scans every frame's blob
(Q19), so each edit's read grew with the edits before it.

Deleting what a run does not read, in the run, was tried first and dropped: the space is one per root, so a run of one
set would delete a sibling set's frames, and the first such delete on ep2 (165K frames) held the run past its cap.

## Decision

### 1. A lane names its policy set; its newer lane supersedes its older

A manifest records its lane's policy set (`corpus_manifests.policy`, added to older stores by `ADDED_COLUMNS`; an
older binary ignores it). In the keep decision (`Protection`), manifests met newest first, a manifest whose set's
newest lane is another is superseded: it is not protected, and takes no place among the root's 64 lanes. Its plans die
unless a protected manifest names them too, and the frames only they held go with the next sweep. A newer lane of a
set also makes a decision worth taking (`corpus_decision_worth`).

Every lane this binary records names a set (§2); one recorded with none is an older binary's (§4).

Undoing an edit is a new run of the older texts: its lane is the set's newest again, and the edit's lane is
superseded. Its plans were dead, so that run is colder.

### 2. What names the set

The programs' policies name it: every `.policy "<name>"` line of the run's programs, as a sorted set, hashed with all
of the run's options (`rerail.corpus-policy/v2`). An edit of a rule's body keeps its policy's name, so the edit's lane
is the same set's; another dialect, other settings, or a phase's or the constants' marks give another set. A run whose
programs name no policy (tests, a session's programs), or a run inside another (`record_run`'s `outer` false), is its
lane's own set: it never supersedes another lane.

(A first build named the set by the command line's rules path, carried among the run's options. The path then keyed
the constants' cached rows too, and a batch or session, which has no path, named none: both dropped.)

### 3. The program that reuses frames holds them

A run whose kept part changed nothing puts nothing (0227, 0211). After an edit, the new program reuses its
predecessor's frames, and only the predecessor's `frame_program_used` row held them: once it is superseded, the sweep
dropped the space and the next edit ran its cross part cold, every other edit (seen on the PYI062 repro).

At those returns the run calls `ContentStore::hold_frames`: when its program does not hold the space yet, it queues
its holding behind the run (a `frame_program_used` row, no frames). Which programs hold which spaces is read with the
snapshot, before the first wave (0104); a lookup in a wave is refused under `RERAIL_SQL_STRICT`. A program that holds
queues nothing (0197). Writes this process queues with a program keep the snapshot's holdings current.

### 4. A lane of no set is an older binary's, retired where sets are named

This binary names every lane's set (§2). A manifest with a lane and no set was recorded before it, and protected
under 0053 alone. Over a root where some manifest names a set, such a manifest keeps no protection
(`Protection::over`): the root's lanes are this binary's from then on, and the older lanes' plans and frames go with
the next decision and sweep. A root no named manifest has run keeps its older lanes as before.

## Consequences

- Tests: `corpus_runs::tests::a_policy_sets_newer_lane_supersedes_its_older`,
  `corpus_runs::tests::an_older_binarys_lane_keeps_no_protection_where_sets_are_named`,
  `frames::tests::a_program_that_reuses_frames_holds_them`. Each fails without its change.
- ep2, B101 body edits, same build, outputs identical as sets (1,334 rows):

  | | Cross space after | Identities left out | Edit |
  |---|---|---|---|
  | Before (11-16, g71–g72) | 155–192K frames, growing | 45–55K | 67.8–69.9 G |
  | After, sweep stalled (h2–h3) | 196,767 | 55,162 | 64.4–67.9 G |
  | After the backlog's sweep (h4–h5) | 131,371 | 36,777 | 52.8–54.9 G |

- The frames sweep had not completed on ep2 since run 289: the process's end did not wait for a sweep the upkeep
  thread was in, and exited under it (decision 0303). Once it ran (10.5 s, then 33.7 s once the older lanes were
  retired: 105,291 frames), the cross space fell to 41,045 frames over 9,161 identities, 3 programs holding it.
- Older lanes (§4): ep2's 36,777 identities left out by older lanes went with them; 5,444 are left out now.
- PYI062 repro (pyi p4): every manifest names a set; consecutive edits stay warm (230 frames read).
