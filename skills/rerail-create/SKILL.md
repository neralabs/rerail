---
name: rerail-create
description: Suggest, write, test, tune and approve Rerail rules for the repository you are in. Suggests rules from this repo's own sessions (the person's corrections, risky commands, which rules already steer) and its written standards, drafts each rule as a code rule or a command rule, tests it against cases and the whole repo, and reads it back. Every rule it adds runs in Log until the person approves it on its page in the dashboard; a rule the person asked for is added after its read-back. Also writes architecture rules (where code belongs, which way dependencies run) from the repo's design docs and maintainers' review comments, proved against the repo's history. Changes where a rule applies, shipped rules included. Audits a rule or pack that waits for approval before the person approves it in the dashboard. Use for /rerail-create, "suggest rules", "what rules should we have", "make a rule that…", "architecture rules", "layering rules", "this rule is wrong", "update the rule … I want it to apply to", "exclude tests from this rule", or "approve the rules" / a rule waiting for approval.
---

# Create a Rerail rule

A Rerail rule checks the coding agent's work while it works, and tells the agent ("steers" it) when the work breaks the rule. A
good rule catches what a reviewer of this repo would catch, and is right every time it fires. One that steers the agent to break
working code is worse than no rule.

You do four things here. You suggest rules from evidence, you write and test one, you tune a rule that misfires, and you audit a
rule that waits for approval. The person decides what steers, and approves each rule themselves on its page in the
dashboard: from your session `rerail rules approve` is refused. Every rule you add runs in Log until then, so the person
sees it on the dashboard before it steers.

A rule Rerail did not publish (one created by you, one in a worktree's `.rerail/rules/`, a third party's or a folder's pack)
steers no one until a person approves its content; until then it runs in Log. A dashboard link you pass on shows the
dashboard only in a browser already connected to Rerail: say so with it, and that for another browser the person runs
`rerail manage` in a terminal. An edit waits again. From your session, an
edit of the person's approved rule is saved as a copy (`<id>-copy`) that runs in Log beside it, and the rule keeps running
as it is until the person approves the copy, which turns the original off.

## Before you start

- **The launcher.** Use the one your `/rerail-create` command named. Without one, use `~/.rerail/bin/rerail` if it exists, else
  `rerail`. Below it is written `rerail`. Paste it in full each time; it may start with an environment assignment, which does not
  survive being put in a shell variable.
- **Drafts live outside the repo**, in a temporary directory. Never write rule files, instruction files (CLAUDE.md, AGENTS.md,
  Cursor rules) or tests into the repository. Keep the draft file: it holds the cases, and `rules export` returns rules without
  them. The one rule file you write in the repo is a rule the person keeps to this worktree, and only once they say yes
  ([Write](#write), step 7).
- **References.** Read each one when you reach the step that names it, not before:

| File | Read it when |
|---|---|
| [references/suggesting-rules.md](references/suggesting-rules.md) | you suggest rules: evidence, the value model, the shortlist |
| [references/code-rules.md](references/code-rules.md) | you draft a rule over the code in files |
| [references/conversation-rules.md](references/conversation-rules.md) | you draft a rule over a command the agent runs (internal testing) |
| [references/architecture-rules.md](references/architecture-rules.md) | the rule is about where code belongs or which way dependencies run, or the person asks for architecture rules |
| [references/rule-quality.md](references/rule-quality.md) | you check a draft before showing it, and when you tune a rule |
| [references/approving-rules.md](references/approving-rules.md) | you approve a rule or pack you did not write in this session |

## Which path

- **The person said what the rule should check:** go to [Write](#write).
- **No words, or "suggest", "what rules should we have":** go to [Suggest](#suggest). It writes and tests each candidate, adds
  those that pass in Log, and lists them for the person to approve in the dashboard.
- **"Architecture rules", layering, or where code belongs:** read
  [references/architecture-rules.md](references/architecture-rules.md), then go to [Suggest](#suggest) or [Write](#write) as
  above. It adds evidence to gather in Suggest and a history check in Write.
- **A rule misfires** ("this rule is wrong", "stop telling me X", a `◆ Rerail` message the person disputes): go to [Tune](#tune).
- **Only where a rule applies changes** ("exclude tests from this rule", the dashboard's "Using Rerail, update the rule …
  I want it to apply to: …"): go to [Scope](#scope). This holds for a rule that ships with Rerail too.
- **"Approve the rules", a rule or pack waiting for approval** (`rerail rules approve` lists them, `rerail packs list` says
  "waiting for approval"): go to [Approve](#approve).

## Suggest

Read [references/suggesting-rules.md](references/suggesting-rules.md) first, and the
[Harmful steering](references/rule-quality.md#harmful-steering) section of rule-quality.md, which the scoring needs.

1. **Gather the evidence. Do these at the same time:**
   - `rerail rules evidence --json`. This returns the person's corrections in this repo's sessions with what the agent had just
     done, risky commands the agent ran, how often each rule steered, and the files where the repo writes down its standards.
     It covers every coding agent and every worktree of the repo, for the last 30 days. Add `--days 90` if that is thin.
     `repository.known: false` means Rerail has recorded no session here yet; suggest from the standards and the code, and
     mention that `/rerail-start` lets Rerail gather evidence for next time.
   - Read the standards files it lists, and take every statement that can be checked mechanically.
   - Look at the code for conventions it already keeps. Count both forms with a search: for example, 140 files call `logger`
     and 3 call `console.log`.
   - `rerail rules export --output <tmp>/rules.json`, to see the person's own rules and avoid duplicating them.
   - For architecture rules, also read the design docs and the maintainers' review comments that
     [Where the intent is written](references/architecture-rules.md#find-the-intent) lists.
2. **Turn the evidence into candidates.** Write one checkable sentence each, in the person's own words. Never turn a "should" into
   a "must". Score each candidate with the value model and drop the ones it says to skip.
3. **Pick the shortlist:** at most five, best first. If nothing is worth a rule, say so in one line, ask what the person finds
   themselves correcting, and add nothing.
4. **Write and test each one**, as [Write](#write) steps 1 to 5, without the read-back: pin down what it means, choose the kind,
   write the cases, test, get it reviewed. Only code rules: leave out a candidate that would be a conversation rule, and name
   it under Left out. A candidate that fails a gate, or does not pass after about five rounds, is left out too.
5. **Add the ones that pass, in Log.** First leave out a candidate that does the same as a rule in `rules export`, and say
   so under Left out. Then give each a new id if its id is taken, by a rule in `rules export` or a By Rerail rule
   (`rerail rules show <id>` prints one): `rules add` would change that rule, or save the draft as an edited copy of it.
   For each: `rerail rules add <tmp>/draft.json`. A rule that is added but not approved runs in Log whatever its mode:
   Rerail records what it would catch and tells the agent nothing. It waits on the dashboard's Overview, under "Your coding
   agent added …", and the person approves it from its page there.
6. **List them, in exactly this form.** One block per rule added, in the order added:

   ```text
   I added <N> rules to <repo>. Until you approve them, they run in Log: Rerail records what each would catch and tells
   no one. Approve each one you want on its page in the dashboard; once approved, each runs in its own mode.

     1. <the title, as the dashboard will list it>
        From             <the evidence, e.g. "you corrected this 3 times">
        Where it runs    When I edit a file under <folder>
        Tells the agent  "<its message, quoted exactly>"
        Once approved    Steer: I'm told in the session and fix it | Log: Rerail keeps a record. I'm not told.
        Tested           Would have caught <N> places in this repo (<path:line>, …)

     2. …

   Left out: <candidate>: <why, in one line>

   Approve them on their pages: <the dashboard link rules add printed for each>. The dashboard's Overview also sends you
   to them, in Created by you. Want one changed first, or to see one's JSON? Say which.
   ```

   - Each block opens with the rule's `title` word for word, so the person approves the name they will find in Created by
     you. The rows follow Write step 6's card: name the excludes that matter in Where it runs, and the folders in Tested
     where `repository.folders` is not `[""]`. A row you are unsure of reads `Not sure`, and that rule's Once approved is
     Log.
   - **Tells the agent** is every message the rule can steer with, quoted exactly: the person approves what it says. Put
     `Reviewed: by me only` above the list when no fresh subagent reviewed the drafts.
   - Leave out the Left out line when nothing was left out. Where `rules add` printed no link (the daemon is not running),
     write `rerail manage` opens the dashboard in its place. Pass on its line saying the repo was set up, if it printed one.
   - Never run `rerail rules approve <id>` for the person: it is refused from your session, and their approval on the
     rule's page is the point.
7. **Answer what the person says.** To see a rule, print its JSON. To change one before they approve it, change it as
   [Tune](#tune) does (raise `revision`, test again), add it again, and show its block alone. Any rule can be removed
   from its page on the dashboard.
8. **Offer to show one working**, as Write step 8, once the person says they approved one set to Steer.

## Write

1. **Pin down what it means.** Restate the rule to yourself as one sentence: what is forbidden, where, and what to do instead. If
   the person's words can be read two ways, ask one question. If it cannot be checked mechanically ("write clean code", "good
   names"), say "I can't check this reliably", offer a narrower version that can be checked, and add nothing. Check
   `rerail rules export --output <tmp>/rules.json` for a rule of the person's that already does this; offer to change that one
   instead.
2. **Choose the kind.** A code rule checks files; read [references/code-rules.md](references/code-rules.md). A conversation rule
   checks a command the agent runs; read [references/conversation-rules.md](references/conversation-rules.md). Conversation
   rules are experimental and off unless `RERAIL_EXPERIMENTAL_CONVERSATION_RULES=1` is set; Rerail's latest release ships
   code rules only. Write one only when the person asks for a rule over a command, and only while `rerail rules test`
   accepts the draft: when it answers that conversation rules are experimental and off, tell the person in one line and
   offer a code rule if one would do.
   - Give it the id `local:<short-kebab-name>` and `revision` 1.
   - Set `title` to the rule in the person's own words, as they wrote or said it, cut down to the rule itself: the dashboard
     lists the rule by it, and `rules add` refuses a rule without one. With no words of theirs (a rule from evidence), write
     the sentence they would put in a list of house rules. [Writing the title](references/code-rules.md#writing-the-title)
     says how.
   - Set `mode`. Use `steer` when you are sure of the rule. Use `log` when the repo pass or the meaning leaves doubt. Never use
     `block`.
3. **Write the cases before the matcher.** You need at least two the rule must catch and at least two it must let through.
   - Include a near miss: code or a command that looks like a violation but is not.
   - Take them from this repo's own code where you can, and match its import style and aliases.
   - When the rule came from evidence, the code or command the person corrected must be a caught case.
   - Give each case its own `path` (`src/components/A.tsx`, `B.tsx`, …): the report names a case by its path only.
   - A passed case outside the scope is fine, and shows the scope works, but it is not the near miss.
4. **Test, and fix the draft until it passes:**

   ```sh
   rerail rules test <tmp>/draft.json --json
   ```

   Then apply the gates in [references/rule-quality.md](references/rule-quality.md#gates):
   - Every case comes out right. A case marked not checked (`notChecked`) is not right.
   - Read every match in `repository.matches`, and decide whether each is a real violation. A match with no `line` is the
     file's path.
   - `repository.folders` are the folders the repo pass covered: where `rules add` would have the rule run (`""` is the
     whole repo). A match outside them is not counted, since the added rule would not catch it.
   - `refusal`, when the report has one, is why `rules add` would refuse the draft as it is. Fix the draft and test it
     again before the card. `repository.notNarrowed`, when there, means the count may not be what the rule would catch:
     say so on the card.
   - `repository.leftOut` counts files and folders the repo pass did not read: hidden, secret-named, or ignored by Git.
     Its matches say nothing about those. `repository.binary` counts files in scope it skipped as binary.
   - A rule that matches hundreds of places fights the repo's convention.
   - The message passes the harmful-steering check.
   - An architecture rule passes [Proving it against history](references/architecture-rules.md#check-it-against-history):
     it catches the first draft the maintainers sent back, and stays quiet on the merge and on recent merged work.

   After about five rounds without passing, stop and tell the person what does not work.
5. **Get it reviewed.** If you can start a fresh subagent, give it the draft, the test report and
   [references/rule-quality.md](references/rule-quality.md), and ask it to find a way the rule misfires or misleads. Fix what it
   finds, then test again. If you cannot, make a separate pass yourself: go through every question under
   [Harmful steering](references/rule-quality.md#harmful-steering) against the draft, try to write one case that breaks it, and
   put `Reviewed: by me only` above the card.
6. **Read it back, in exactly this form, and ask once:**

   ```text
   Here is the rule as I understand it.

     Title            <the title, as the dashboard will list it>
     What it checks   <in plain words>
     Where it runs    When I edit a file under <folder> | When I run <command> | When you or I say <…>
     Applies to       All of <this repo's name> | This worktree only (<its folder name>)
     What happens     Steer: Rerail tells me as I work, and I fix it. I'm not stopped.
     How often        Every time | The first time only
     Tested           Would have caught <N> places in this repo | Would have caught <N> places under <folder>/

   Add this rule?
     1. Yes, add it
     2. Change something
     3. Show me the rule
     4. Cancel
   ```

   - **Rows.** Title is the draft's `title` word for word, so the person sees the name they will find in the dashboard.
     Applies to is "All of <repo>" unless the person asked for this worktree, branch or experiment only; repo is the
     name `rules test` and `rules add` use (the checkout's folder name). Where it runs names the
     excludes that matter: "When I edit a file under src/ (not src/scripts/ or tests)". How often is "Every time" for a code
     rule; for a command rule it follows `repeat.mode` (`always`: Every time, `once`: The first time only). Tested names the
     places: "Would have caught 2 places in this repo (src/api/refunds.ts:2, …)"; with none, "Matches nothing in this repo
     today: the code already follows it". Where `repository.folders` is not `[""]`, the count covers only those folders, so
     name them in place of "in this repo": "Would have caught 2 places under src/, web/ (…)". A command rule gets no repo
     pass, so its Tested row gives the cases: "Caught 4 of 4 cases, let 5 of 5 through". That alone is no reason for
     `Not sure`.
   - **Log rules.** The What happens row reads "Log: Rerail only records it, for you to review. I'm not told."
   - **Rows you are unsure of.** Mark the row `Not sure`. Below the card, show one line from this repo that the rule catches and
     one that it misses, and set the rule to Log.
   - **Architecture rules.** Put the history result above the card in one line: `Checked against history: caught the first
     draft of #123; quiet on its merge and on 15 recent merged PRs.`
   - **Conversation rules** (command or similarity). Put one line above the card: `Experimental: this kind of rule is not
     part of Rerail's latest release, and runs only while RERAIL_EXPERIMENTAL_CONVERSATION_RULES=1 is set for the daemon, or
     for Pi itself when Pi runs Rerail in process.`
   - **"Change something".** Offer Steer and Log, each with what it means; All of the repo or this worktree only (its rule
     runs nowhere else, not in the main checkout nor other worktrees); the title; and any other row the person names.
   - **"Show me the rule".** Print the rule JSON, then ask again.
7. **Add it, only after the person says yes.** For all of the repo:

   ```sh
   rerail rules add <tmp>/draft.json
   ```

   It runs in Log until the person approves it on its page in the dashboard; `rerail rules approve` is refused from your
   session. Reply in one line, `● <what it checks> is added, set to <Steer|Log>. It runs in Log until you approve it on its
   page: <the link the command printed>.` (the command prints the link when the Rerail daemon is running; without one, say
   `rerail manage` opens the dashboard). Pass on its line saying the repo was set up with this rule, if it printed one.

   For this worktree only, write the rule into the worktree's own rules folder instead; there is nothing to add:
   - Check it can run: `rerail state` must print `started` and `rerail state --set-up` must print `set-up`. Otherwise say the
     rule will not run until Rerail is started here and the repo set up (`/rerail-start` does both), and ask whether to write it
     anyway.
   - Write the draft's `rule` object alone, without `kind` or `cases`, as JSON to
     `<worktree top level>/.rerail/rules/<short-kebab-name>.json` (`git rev-parse --show-toplevel` gives the top level). Keep the
     draft file in the temporary directory.
   - Run `rerail packs list` there. The rule must be listed under "This worktree's own rules". A `left out:` line naming the
     file says why it did not load; fix the file, or delete it and tell the person. A `Not running:` line means Rerail is off
     here or the repo is not set up; pass it on as written.
   - If `git check-ignore -q .rerail/rules/<short-kebab-name>.json` fails, the file is not ignored: say that committing it puts
     the rule in every checkout of the branch, and offer to add `.rerail/rules/` to `.git/info/exclude`. Change nothing in Git
     without a yes.
   - Reply in one line, `● <what it checks> is in this worktree only, set to <Steer|Log>. It runs in Log until you approve it
     on its page in the dashboard.` (`rerail manage` opens the dashboard.)
8. **Offer to show it working (code rules only)**, once the person says they approved it. Ask: "Want to see it fire?
   I'll write a throwaway file that breaks the rule, show you Rerail's message, then delete it."
   - On yes, write the file under the rule's scope. Show the `◆ Rerail` message that follows, then delete the file. This file is
     the one exception to not writing in the repository, and it must be gone before you reply.
   - If no message comes, say so. The repo may not be started; `/rerail-start` starts it.
   - Never run a command a command rule forbids to demonstrate it.

A rule the person wrote by hand into `.rerail/rules/` can be checked as it is: `rerail rules test .rerail/rules/<name>.json
--json` validates it and runs the repo pass, with no cases. To prove or tune it, build a draft with cases around it.

## Tune

1. **Find the rule and the misfire.** A `◆ Rerail` message names the rule id and the `path:line`. Without one, ask for the code
   it fired on, or reproduce it: a probe draft with the rule and the suspect code as cases. If what the person describes does
   not reproduce, show what does fire and ask before changing anything.
2. **Find out whose rule it is.**
   - A rule in this worktree's `.rerail/rules/` (`rerail packs list` lists them under "This worktree's own rules") is the
     person's. Change its file in place; the change applies at the next edit.
   - A `local:` rule, or any rule in `rerail rules export`, is the person's. Change it here.
   - Any other rule ships with Rerail. Its file can't change, but the person can run their own copy of it in this repo:
     `rerail rules show <id>` prints it as a draft, and `rules add` of a draft that keeps its id stores the person's copy
     where the original is; approving the copy turns the original off there. Offer that, or setting it to Log or off in
     this repo from the dashboard (`rerail manage --print` gives the link). If the person wants it changed for everyone,
     `/rerail-bug-report` reports it to the Rerail team in an issue.
3. **Fix it** with the most targeted step on the fix ladder in
   [references/rule-quality.md](references/rule-quality.md#the-fix-ladder). Add the misfire as a `passed` case, and show it was
   wrong before: run it against the current rule and see it caught. Keep every earlier case: from the draft file, or, without it,
   rebuilt from the rule's description and the repo. Raise `revision` by one. A rule from before titles has none: add one, as
   Write step 2 says, and show it in the read-back.
4. **Test, read back and add** as in Write, steps 4 to 7; a worktree rule's file is written over in place. The edit waits for
   the person's approval on its page. An edit of an approved rule of theirs is saved as a copy (`<id>-copy`, which
   `rules add` names): the rule keeps running as it is until they approve the copy, which turns it off. In the read-back,
   add one line above the card, at most two sentences:
   `Changed: inline type imports (import { type User }) now pass; a mixed import is still caught.`

## Scope

The person changes only where a rule applies. Change its `scope` and nothing else: what it checks, its message, its
mode, its title and its id stay as they are. Write's review and quality gates are about those, so skip them; this takes a
handful of commands, not a new rule. `rules show` gives a rule that ships with Rerail the title its pack lists.

1. **Read the rule:** `rerail rules show <id>`. It prints the rule as a draft, as it runs in this repo; save it as
   `<tmp>/draft.json`. A line on stderr says what saving does. Go on after the first two lines below, start again with
   the rule they name after the next two, and after the rest say what the line says, and stop:
   - "is yours, added to <repo>": saving changes the person's rule where it is. If the line says it is also added to
     other repositories, saving changes it there too: the card's Saved as row names them. When a second line says it "is
     approved, so in … session rules add saves an edit of it as a copy", it is saved as that copy instead, here only.
   - "ships with Rerail": saved as the person's copy, added where it is; approving the copy turns it off there.
   - "already has your copy … Change that one": run `rules show` on the copy's id and start again with that one.
   - "is your copy of <original> for another repository": run `rules show <original>` and start again with that one.
   - "is not added to <repo>": the rule does not run here, so there is no scope to change here.
   - "is off in <repo>": a copy would run where the person has the rule off. Ask whether they want it on first.
   - A rule in this worktree's `.rerail/rules/` is not printed: read its file instead and change it in place.

   A rule with no `scope` (one about commands, not files) has no paths to change: say so, and stop. A second line, "has no
   title", means a rule of the person's from before titles: `rules add` refuses it without one, so add a `title` as Write
   step 2 says, and show it in the card's diff as `title  + "<title>"`.
2. **Change only `rule.scope`.** Turn the person's words into `include` and `exclude` globs (an exclude wins; `**/`
   matches any folder). Name what this repo really has: one listing of its files shows, for example, where tests live
   (`tests/`, `test_*.py`, `*_test.py`, `conftest.py`, `*.test.ts`, `__tests__/`). If the words can be read two ways, ask
   one question. If the scope already does what they ask, say so with the globs that do it, and stop. Raise `revision`
   by one for the person's own rule; keep it for a rule that ships with Rerail.
3. **Check the paths.** Add `cases` to the draft: for each path the change keeps or leaves out, one file at that path with
   the same content, a line the rule catches (take it from the rule's description or one of its matches). A path
   still in scope is `caught`; a path the change leaves out is `passed`. Run `rerail rules test <tmp>/draft.json --json`
   once. Every case must come out right; a left-out path reads "(outside the files this rule covers)". Count
   `repository.matches` for the card; do not review them. The repo pass covers only the folders the rule runs in
   (`repository.folders`), so the count is what the saved rule will catch. Where they are not `[""]`, the Tested row
   names them in place of "in this repo": "Would catch 2 places under src/, web/".
4. **Show the change, in this form, and ask once:**

   ```text
   Change where <id> applies?

     scope.exclude  + "**/tests/**"
                    + "**/conftest.py"

     Still checked  src/app/models.py, scripts/sync.py
     Left out now   tests/test_api.py, conftest.py
     Tested         Would catch <N> places in this repo | Would catch <N> places under <folder>/
     Unchanged      What it checks, its message, <Steer|Log>
     Saved as       Your copy of <id>; the original turns off where the copy is added once you approve it
                    | <id>, revision <n>, here (and in <other repositories>)

     1. Yes, save it
     2. Change something
     3. Cancel
   ```

   The diff lists only the globs added (`+`) and removed (`-`). If the person already said yes in this conversation to
   exactly these globs, every case passed, and that earlier preview said what saving does (what the Saved as row says),
   that yes stands: show the card without its question and save. Otherwise ask.
5. **Save it, only after the person says yes.** `rerail rules add <tmp>/draft.json`. The change runs in Log until the
   person approves it on its page in the dashboard, and a copy's approval turns the original off. A rule in
   `.rerail/rules/` is saved by writing its file, and waits for approval the same way. Reply in one line:
   `● <id> applies to <the person's words> once you approve it on its page: <the link rules add printed>.`

Anything else you notice on the way (a message you would word differently, a match that looks wrong) is one line after
the reply, offered as its own change. Never fold it into this one.

## Approve

Read [references/approving-rules.md](references/approving-rules.md) first.

1. **See what waits:** `rerail rules approve` with no name lists every rule and pack waiting here, each with where it comes from.
2. **Audit each one you did not write in this session**: read all of its content, and check what it would steer the agent with,
   as the reference says. You are looking for anything a reviewer would not expect a code rule to say or do.
3. **Tell the person with the reference's card**, one per rule or pack, with every flag you found. Recommend keeping it in Log
   when you found anything.
4. **The person approves it** on its page in the dashboard. `rerail rules approve` is refused from your session: give them
   the page's link (`rerail manage --print` prints the dashboard's; a rule's page is `#rule/<id>` on it), or tell them
   `rerail manage` opens the dashboard.

## Never

- Add, change or remove a rule the person has not confirmed in this session, except adding the rules
  [Suggest](#suggest) found, tested and listed for approval, unapproved, so they run in Log.
- Approve a rule or pack, or import a rules dump: `rerail rules approve` and `rerail rules import` are the person's, and
  refused from your session. Never get around that: not by unsetting the variable that marks your session, nor through
  the dashboard's API or Rerail's database.
- Offer "every repo". A rule from here belongs to this repo only, or to this worktree only.
- Use `block` mode.
- Invent evidence, cases or test numbers, or quote the person beyond what the evidence shows.
- Write instruction files, rule files or tests into the repository, or leave any file there. The exceptions: a rule the person
  keeps to this worktree, written to `.rerail/rules/` after their yes, and Write step 8's throwaway file, deleted before you
  reply.
