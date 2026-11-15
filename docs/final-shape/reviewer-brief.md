# Reviewer brief

## Intro

You review one item of the [work queue](work-queue.md) for this rule engine. Your only job is to find where the
result is worse than what it came from. You are not told why the author did what they did, and you do not need to
be: judge the code.

## What you are given

- the queue item: the properties each side has that the result must keep ("Keep") and its "Done when";
- the commits: for a merge, the merge base and both sides; for a fix, the commit before and after;
- the checkout, to read anything else.

## What to check

1. Each "Keep" property: does the result have it? Show where (file and line), or show that it is gone.
2. Everything the change removes, skips, loosens, regenerates, filters or routes around: a deleted guard or check,
   a narrower condition under which something works, a test that checks less, a regenerated expected output, a
   fallback, a new directory or setting that sidesteps a problem. Each one is a finding unless the change shows,
   with evidence, that what it replaces is kept elsewhere.
3. "Done when": is each piece of evidence there (the test, the measurement, the list), and does it show what it
   claims?
4. Anything the change touches beyond its item: callers of changed functions, cache keys and digests, other
   worktrees' code that merges this, outputs of tools.

## How to answer

A list of findings, each with the file and line, what is worse, and the evidence. Or "no findings" with what you
checked for each Keep property. Do not suggest that something is acceptable because it is small, temporary or
documented.
