# Rule quality: gates, harmful steering, and the fix ladder

A lint rule has to be right most of the time. A steering rule has to be right every time it fires, or say clearly when it might
not be. The agent obeys a steer in its very next edit about 62% of the time. A wrong steer is therefore not noise: it is a change
to working code.

## Gates

A draft goes to the read-back only when it passes all of these.

| Gate | Pass | Otherwise |
|---|---|---|
| Valid | `rules test` reports `valid: true` | Fix what the error names. It gives the field path. |
| Cases | Every case `ok`, none `notChecked`; at least 2 caught and 2 passed, one of them a near miss | Fix the matcher, or the case's code if it does not parse |
| Evidence | The code or command the person corrected is a caught case | The rule does not catch what was asked for: rework it |
| Repo matches | You read every example the report shows, and each is a real violation | Narrow the matcher, and add the false positive as a passed case |
| Repo agreement | Matches in no more than about 25 files | Many matches mean the repo's convention says otherwise. Show the person 2 or 3, and ask whether the existing code is wrong. If not, narrow the rule or drop it. If it is but fixing is not the agent's job now, use Log. |
| Command rules | No repo pass runs; the cases carry the proof, so include the tools of every agent (`Edit`, `write`, `apply_patch`, `bash`) | — |
| Zero matches | Fine when the person asked for it, a standard states it, or the evidence shows the agent breaking it: the repo already follows it | With none of those, the rule may be aimed at nothing: mark the Tested row `Not sure` |
| Standard vs. safety | Following the steer is safe everywhere in the scope | A written standard can still be unsafe in part of its scope (a Node-only logger steered into browser components). Narrow the scope to where it is safe and say so in the read-back |
| History (architecture rules) | Catches the first draft the maintainers sent back; no more matches at the merge than at the base; quiet on recent merged PRs in its scope, as in [architecture-rules.md](architecture-rules.md#proving-it-against-history) | Rework the matcher. A rule that matches the accepted merge steers against the maintainers' own design |
| Unchecked files | `repository.unchecked.count` is 0, or the files are ones the rule does not care about | Say in the read-back that the rule cannot see those files |
| Left out | `repository.leftOut` is 0, or the rule's scope is not about hidden, secret-named or Git-ignored files | The repo pass does not read those files; Rerail enforces this in code. Hidden and secret-named files are never read; Git-ignored ones are read only after a person turns the repository's Follow .gitignore off. Zero matches then says nothing about them: mark the Tested row `Not sure` and say the repo pass could not check them |
| Safe to obey | No item in [Harmful steering](#harmful-steering) applies | Use the fix ladder below, or Log |

## Harmful steering

These are the five ways a steer makes code worse, each with a case found in Rerail's own rule packs:

1. **It breaks behaviour, because a deliberate idiom looks like a mistake.**
   - "Hoist `count()` out of the loop" breaks a fixed-point loop that re-reads the size of a collection it grows.
   - "Remove `var_dump`" breaks code that captures its output with `ob_start`.
   - "Delete the `if (false) { class X {} }`" removes a declaration stub that tools read.
2. **It breaks the build or the runtime, because it depends on settings the rule cannot see.**
   - The promise form of `fs.watch` is an async iterator, not a drop-in replacement for the callback form.
   - A fix that is only valid under one compiler setting breaks under another.
3. **It fights the repo's own convention.** The rule is generic and the repo chose otherwise. The agent rewrites working code
   into a style nobody here uses, and is steered again.
4. **The matcher is wrong.**
   - The regex is unbounded on the left.
   - The pattern matches in strings.
   - The rule tests a name it cannot resolve. For example, it flags every `.extra(`, not only Django's.
5. **It over-corrects or loops.** The steer repeats in the same run, or the "fix" deletes tests or checks to make the message go
   away.

Questions to ask of every draft:

- **Is there any code this matches where following the message is wrong?** Write that code as a passed case. If the matcher
  cannot tell it apart, the message must say when to leave the code alone.
- **Does the message say "remove" or "delete"?** Prefer naming what to use instead.
- **Does the message claim something the rule did not check?**
- **Does the description agree with the matcher?** Check every alternative in the regex, and every scope glob.
- **Is the message tied to one literal spelling** of something that has several?

## The fix ladder

When a rule misfires, take the most targeted step that removes the misfire and keeps the rule's intent. "The intent of the rule
is fine, but the rule itself is harmful" is the usual finding.

1. **Exempt the idiom.** Add a `not` for the shape that is legitimately different, plus a passed case for it. A `regex` has no
   look-around: narrow the pattern itself, or move to `ast` where a `not` can say it.
2. **Narrow the scope.** Exclude the folders where the rule does not apply: tests, scripts, generated code.
3. **Tighten the matcher.** Bound the regex, move from `regex` to `ast` so strings and comments stop matching, or require the
   import that makes the name what the rule thinks it is.
4. **Reword the message so it informs.** Replace a command with a statement plus a condition: "`@` hides errors. If this call can
   fail in normal operation, keep it." instead of "Remove the `@` operator."
5. **Set it to Log.** Rerail keeps a record and the agent is not told. This is right when the repo's existing code chose
   otherwise, or when the matcher cannot be made safe.
6. **Remove the rule.**

Each step needs a passed case that failed before it, and a raised `revision`.

## Cost

A rule runs on every edit inside its scope, so cost matters.

- **Scope narrowly.** `src/components/**/*.tsx` costs less than `**/*`.
- **Prefer one clause** with `any` over many clauses that read the same nodes.
- **Avoid broad `inside` searches with `stopBy: "end"` over huge files** when a `kind` filter on the outer rule can narrow them
  first.

To measure a draft, time its repository pass with `time rerail rules test <draft.json>`. If that takes seconds on a repository
where a narrower draft takes a fraction of one, narrow the rule, or say so in the read-back.
