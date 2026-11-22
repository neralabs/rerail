# Roll-up 47: main (b922e65dc8) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at 18ec208c5f. Theirs: main at b922e65dc8, pinned 2026-11-22 in
`tmp/rollup/pins4.txt`. The merge base is 1609ac3a30, roll-up 43's main. Theirs brings two test fixes, and no engine
or product code:

- #986 (04ad4184ea): in `outside-git.test.ts`, the named-pipe case waits for its writer's exit before its folder
  goes. CI failed it on Linux when the folder's removal found the shell still running.
- #988 (b922e65dc8): in `engine-proof.test.ts`, the case for a pipe holder that answers takes the holder's answer,
  however late PowerShell writes it.

## Items

1. **`outside-git.test.ts`, the named-pipe case's `finally`: theirs.** Both sides fixed the same race. The case's
   folder is removed while the shell writing into the pipe is still blocked in its open.
   - Ours, in 96142c32b3: SIGTERM only when the writer had not ended yet, then wait for the exit, listening from
     the kill on.
   - Theirs, in #986: listen from the writer's start, send SIGKILL and wait. Then check that the signal ended the
     writer (`[null, "SIGKILL"]`), so nothing read the pipe.

   One mechanism does this job, and theirs holds what ours did plus two things ours did not:
   - an exit before the `finally` is not missed;
   - the check that the pipe was never opened for reading, which is the case's claim.

   Ours' block, kept beside theirs, would also hide theirs' `exited` and skip the kill that theirs' check needs.
   The audit lists ours' four lines as dropped. They are superseded by #986.

   Ours' other changes to the file stay. Theirs did not touch them:
   - the GIT_DIR shapes, which do not ask Git, since Rerail's Git never gets the repository variables (96142c32b3);
   - `gitTop`'s filtered environment;
   - the `describe` title.
2. **`engine-proof.test.ts`: theirs, with no conflict.** Ours has not changed the file since the base, and the
   merged file is the same as main's. Its changed cases run on Windows only (`describe.runIf(win32)`), and main's
   CI ran them there.

## Checks

- `scripts/check/merge_audit.py` on the merge in progress: four findings, ours' lines in item 1.
- Here, `outside-git.test.ts` and `engine-proof.test.ts` pass, 70 of 70. The 13 skipped are engine-proof's three
  Windows-only `describe`s.
- No bench: neither side's change touches the engine.
