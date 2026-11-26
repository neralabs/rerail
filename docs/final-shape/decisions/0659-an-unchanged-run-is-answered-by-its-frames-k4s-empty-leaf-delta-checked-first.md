# 0659. An unchanged run is answered by its frames: K4's empty leaf delta, checked first

Status: decided (2026-11-23), by the user: "build the proposal", after the spike that measured it (commits
`cfdc9f1034`, `06bc84f2fa`). It replaces the route t33 had agreed for step 1 ("keep cutting the per-process steps,
and retire the frame once a clean run costs the same without it").

## Intro

A run over files and rules that have not changed should cost almost nothing. t33 (decision 0104's moves) says this
must not be a mechanism of its own: K4 covers it, since with no leaf delta no stratum runs and every output is as
stored. The agreed route was to cut what a new process derives from the program until a run without its frame
cost what a framed one does, and then delete the frame.

The cuts went from about 3.4 to about 2.9 times the framed run, and then stopped falling: what is left is some forty
derivations of a few percent each, which a new process makes from the program before it can tell that no stratum
runs. The route would not close step 1.

## Decision

1. **The check of an empty leaf delta is K4's, and comes first.** A run's frame is kept facts of the store
   (`plan/corpus/frame.rs`, the space `rerail.frame`): the leaves the run read, as `frame_read(kind, path, digest)`,
   its plans, its program's summary and its output image. A run whose every leaf is as it was takes the image and
   prepares no program. The frame is named in the run's manifest, so collection keeps it while it keeps the run.
2. **One frame mechanism.** The whole run's frame (`reuse`), phase 1's (`phases::phase_one`), the constants'
   (`constants`) and a command's (`reuse::command`) are the same relations. A command's frame holds the leaves read
   before its run (the rule files and their directory's listing) and names the run's frame (`frame_then`): an
   unchanged command reads no rule text.
3. **One table of leaf readers** (`plan/corpus/leaves.rs`): what each kind of leaf is now (documents, rule files,
   the input readers' kinds, Git's, the world's requests). `inputs::still`, `World::changed`, `World::still`,
   `unchanged` and the constants' own check are gone. `git::still` and `strings::git::still` are two of the
   table's readers.
4. **The output image stays a derived fact**, written once where the run computes it (decision 0353): it holds
   messages composed from the rules' templates, leaves out rows about loaded documents and records places, so it is
   not rebuilt from kept rows.
5. **Cuts to the per-process steps go on for edits' sake**, measured on one-file edits, which pay every step an
   unchanged run without its frame pays. They no longer gate anything.

## What it cost and brought

Measured on 30 files and 317 rules, alternating runs against the build before:

- one mechanism in place of two frames and three leaf checks: cold runs, framed warm runs and one-file edits cost
  what they did;
- with the command's frame, an unchanged run costs about 30% of the framed run before it (the command line had read
  every rule text and derived phase 0's include graphs twice before the frame was looked at);
- every warm run's outputs equal the cold run's (661 relations).

Found on the way and fixed: the selected-directory leaf went to a reader that did not know its kind, so a run that
read it never took its frame; the per-file memo's keys left out what a text test tests, so an edited pattern
answered every file with the old rows; and the frame's key read `rerail.env` where a revision's run does not.

## What stays

- The observer's planned records that are not cutoffs: the Git files' listing (`git-files:`), the cone's rounds'
  cursors (`cone-rounds:`), the Git functors' answers, each checked by the journal's cursors, and a revision's
  constants (`constants:<key>:<shape>:<values>`), kept by the values their run read in that revision and found by
  their shape.
- `RERAIL_FRAME_OFF`, which measures the path an edit pays.
