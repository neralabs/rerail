# 0497: A bare repository is a folder, and a scan reads the checkout it names

Status: accepted (2026-11-20, the user, at the hub's merge into the sync branch). Supersedes git-commit-tree-metadata's
87d18d609a (a bare repository as a checkout) and 250abffc6b's rule for every Git process (a caller's repository
variables never inherited). Keeps main's placement (#813, #895) and both sides' tests.

## Intro

Rerail asks two kinds of Git questions. A placement asks where a folder belongs: which checkout, which repository,
which worktrees the repository has. A content read asks what a checkout holds: its `HEAD`, its index, its blobs. Main
and git-commit-tree-metadata answered two edge cases differently, and their tests contradicted each other at the
merge.

A bare repository is a Git repository with no working files (`git clone --bare`, as on a server; working folders are
added with `git worktree add`). Git places nothing in it: `git rev-parse --show-toplevel` refuses there.

A Git hook runs with Git's lookup variables set for its own checkout (`GIT_DIR`, `GIT_INDEX_FILE`,
`GIT_OBJECT_DIRECTORY` and others). A Rerail command started from a hook inherits them, and they override `-C`.

## Decision

1. **A bare repository is a folder**, as Git places it (`git: false`, its id in `<bare>/.rerail/repository-id`), keyed
   by its remote where it has one. It still shares one identity with its linked worktrees, whichever is recorded
   first, read from files alone:
   - a linked worktree with no id yet takes the bare folder's (`bareFolderCloneId`);
   - a bare folder takes its worktrees' id (`<bare>/rerail-repository-id`) over its own marker (`sharedGitCloneId`),
     and a row recorded under its own marker keeps its settings (`folderMarkerCloneId`).
2. **Placements follow Git's lookup variables**: with `GIT_DIR` or `GIT_OBJECT_DIRECTORY` set, Rerail places a folder
   and lists worktrees as Git does under them (main's tests).
3. **A read that names its checkout leaves them out**: the content reads (`readGit`, `streamGit`: `HEAD`, blob ids,
   blobs) run Git without this process's repository variables, and a headless scan, a process of its own that names
   its checkout (`--repo`, else its folder), drops them at its start (`leaveInheritedRepository`), so every placement
   and read it makes is of that checkout, also when a hook started it.
4. **Every blocking Git start goes through `runGit`**, the bytes form too (`output: "bytes"`, for `-z` listings whose
   names are not text): main's tests count Git processes there.

## Direction (the user, 2026-11-20)

"We need to abstract away from a filesystem. We should not be running git commands at all in the code that we're
bringing, we should use gix to get whatever we need from the repo directly." Point 1 adds no Git process (the
bare-repository check by `rev-parse --is-bare-repository` and the common-directory lookup are gone). Moving the
TypeScript catalog's remaining Git commands to the engine's gix reads is work queue Q387.

## Tests

- Main's: `outside-git.test.ts`, `checkouts/git-placement*.test.ts`, `checkouts/kept-answers.test.ts`.
- git-commit-tree-metadata's: `repository-registry.test.ts`'s two bare tests (one identity in either order; an earlier
  bare id adopted, its settings kept), now asserting the bare folder's placement; `registered-worktrees.test.ts` on
  `runGit`; `git.test.ts`'s reads of the requested checkout under another Git directory and index; the headless
  scan's run inside a foreign Git environment (`headless-scan.test.ts`, needs the Node addon).
