# Merging branches

## Intro

Many branches of this repository are worked on at once, each by its own agent. A merge brings two
lines of work together. Each line has made design decisions, and its code holds them. Examples:
"a tape stores only named nodes", "a cache is two ordinary rules", "one crate defines the tape
format". A merge that resolves conflicts by taking whichever side looks right in the hunk keeps
the text and loses the decisions. Then the fix that one branch made quietly comes undone, and later
merges spread the loss.

An audit of the merges of 2026-11-11 to 2026-11-13 found this happening. Some merges took one side
whole across many files. Others dropped one side's added tests and checks with no conflict at all,
because the other side had rewritten the code around them. Six decision numbers were used twice and
never renumbered. This page is how to merge without doing that.

## Before resolving: know what each side decided

1. Find the merge base and read both sides' logs since it:

   ```
   base=$(git merge-base HEAD MERGE_HEAD)
   git log --oneline $base..HEAD        # ours
   git log --oneline $base..MERGE_HEAD  # theirs
   ```

2. Read the decision records each side added or changed. These are the invariants the merge must
   keep:

   ```
   git diff --stat $base MERGE_HEAD -- docs/final-shape/decisions docs/final-shape/open-items.md
   git diff --stat $base HEAD       -- docs/final-shape/decisions docs/final-shape/open-items.md
   ```

3. For every conflicted file, read both sides' commits that touched it (`git log $base..MERGE_HEAD
   -- <file>`), not only the hunk. A hunk shows text. The commit message says why the text changed.

## While resolving: by invariant, not by side

- Never resolve a file wholesale with `git checkout --ours/--theirs`, `-X ours/theirs`, or by
  copying one side's file over the result. This applies to a format, a constant, a grammar, a
  schema, or a file both sides changed in meaning. Resolve hunk by hunk.
- When both sides changed one mechanism, the result must hold both sides' invariants. If they
  cannot both hold, one side's decision supersedes the other's. Say which, and why, in the merge
  commit. Example: one side added a stored digest to the tape header (format 11). The other side
  had already removed stored digests (format 15). The result keeps format 15 and drops the digest.
  The merge commit says so and cites the decisions.
- Keep one definition of each shared thing: the tape format and its version, header offsets, a
  grammar's rules, a constant like `KERNEL_LAUNCH_LIMIT`. If both sides defined it, the result has
  one definition, and the other places re-export or use it.
- A side's added test, check or guard is kept, even when the code around it was rewritten. Port it
  to the new code. If it no longer applies, say so in the commit.
- Lines that one side deleted on purpose stay deleted. An auto-merge can bring them back when the
  other side moved the surrounding code.

## Decision numbers

A merge that brings two decision records with one number renumbers one of them at once. Don't ask.
Keep the number on the record that more worktrees already cite. Move the other record past the
highest number in any worktree:

```
ls ../*/docs/final-shape/decisions | grep -oE '^[0-9]{4}' | sort -n | tail -1
```

Add a line "Numbered N where it was written" to the moved file. Update every reference to it, and
its row in `docs/final-shape/decisions/README.md`. A reference is a decision number in code
comments, profile strings, rule files and docs, not only links. Decide which record each one means
from the topic beside it, or from which record existed when the line was written (`git blame`).

Only renumber a collision your merge creates. A collision that both sides already have (one
inherited from the hub, the branch `final-shape-serverplan-quality`) is the hub's to fix. If a
branch renumbers it too, the two renumberings conflict at the next merge. When a merge brings the
hub's renumbering, take it, and move your branch's references to match.

## Before committing: audit what was dropped

Run the audit on the merge in progress (it reads `HEAD`, `MERGE_HEAD` and the working tree):

```
scripts/check/merge-audit.py
```

It lists:

- each side's added lines that the result has nowhere in its tree;
- each side's deleted lines that the result has again;
- files a side added that the result lacks;
- decision numbers used twice.

Moved code is not reported, because a line found anywhere in the tree counts as kept. Each
finding is either restored before committing, or named in the merge commit as superseded, with the
commit or decision that supersedes it. `scripts/check/merge-audit.py <merge>` audits a merge
already made. `--since <date>` audits every merge on the branch since then.

## The merge commit

The message says, for each conflict that needed a choice:

- what each side wanted, citing decision numbers or commits;
- which invariant the result keeps, and how;
- what was dropped as superseded, and by what.

"Kept theirs" is not a reason.

## After committing: test both sides

Run the targeted tests that each side added or changed since the base (`git diff --stat $base
<side> -- '*tests*.rs' '*_tests.rs'`), and the tests of the files that had conflicts. Do not run
full suites (see AGENTS.md). A failing test that one side had passing is a merge defect. Fix it in
the merge, or in the commit right after it, naming the merge.

## If a loss is found later

Fix it forward. Restore the lost code from the side's commit (`git show <commit>:<path>`), port it
to the current code, and say in the commit which merge lost it. Don't redo or revert the merge.

## Invariants that merges have undone before

This list is not complete. The decision records are the full set.

- Tapes store only named nodes, and no producer writes a grammar-implied token (RER-131; decision
  0140 for the writer).
- One tape format, defined in `native/crates/tape_format` (`FORMAT_VERSION`, `header`), with no
  stored digest. Other crates re-export it.
- A cache is two ordinary rules in `.rr`, a key rule and an output rule (0103). Nothing kept
  outside facts, and no wave waits for SQL (0104). `scripts/check/check-no-new-caches.py` refuses
  new cache stores.
- A relation carries its delta, and an edit runs the strata its deltas reach (0117, 0120).
- Planning costs no more than running (0122, 0132).
- A store's collection keeps every plan a live manifest uses, including inner phase runs.
- Nodes live in tapes, with no per-node structure beside them (0026, `check-node-structures`).
- The Metal path does no evaluation on the host. Generated checks have no per-byte loops.
