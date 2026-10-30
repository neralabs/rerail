---
name: rerail-create
description: Suggest, write, test, tune and approve Rerail rules for the repository you are in. Suggests rules from this repo's own sessions (the person's corrections, risky commands, which rules already steer) and its written standards, drafts each rule as a code rule or a command rule, tests it against cases and the whole repo, reads it back, and adds it only after the person says yes. Also writes architecture rules (where code belongs, which way dependencies run) from the repo's design docs and maintainers' review comments, proved against the repo's history. Audits a rule or pack that waits for approval and asks the person before approving it. Use for /rerail-create, "suggest rules", "what rules should we have", "make a rule that…", "architecture rules", "layering rules", "this rule is wrong", or "approve the rules" / a rule waiting for approval.
---

# Create a Rerail rule

A Rerail rule checks the coding agent's work while it works, and tells the agent ("steers" it) when the work breaks the rule. A
good rule catches what a reviewer of this repo would catch, and is right every time it fires. One that steers the agent to break
working code is worse than no rule.

You do four things here. You suggest rules from evidence, you write and test one, you tune a rule that misfires, and you audit a
rule that waits for approval. The person decides; nothing is added or approved until they say yes.

A rule Rerail did not publish (one created by you, one in a worktree's `.rerail/rules/`, a third party's or a folder's pack)
steers no one until a person approves its content; until then it runs in Log. An edit waits again.

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
- **No words, or "suggest", "what rules should we have":** go to [Suggest](#suggest), then Write each rule the person picks.
- **"Architecture rules", layering, or where code belongs:** read
  [references/architecture-rules.md](references/architecture-rules.md), then go to [Suggest](#suggest) or [Write](#write) as
  above. It adds evidence to gather in Suggest and a history check in Write.
- **A rule misfires** ("this rule is wrong", "stop telling me X", a `◆ Rerail` message the person disputes): go to [Tune](#tune).
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
     [Where the intent is written](references/architecture-rules.md#where-the-intent-is-written) lists.
2. **Turn the evidence into candidates.** Write one checkable sentence each, in the person's own words. Never turn a "should" into
   a "must". Score each candidate with the value model and drop the ones it says to skip.
3. **Show the shortlist:** at most five, best first, in the card format the reference gives, and ask which to write. With no
   sessions, a shortlist from the standards and the code is still a shortlist; its header says where it came from. If nothing
   is worth a rule, say so in one line and ask what the person finds themselves correcting.
4. **Write each picked rule** in turn, one read-back and one yes each.

## Write

1. **Pin down what it means.** Restate the rule to yourself as one sentence: what is forbidden, where, and what to do instead. If
   the person's words can be read two ways, ask one question. If it cannot be checked mechanically ("write clean code", "good
   names"), say "I can't check this reliably", offer a narrower version that can be checked, and add nothing. Check
   `rerail rules export --output <tmp>/rules.json` for a rule of the person's that already does this; offer to change that one
   instead.
2. **Choose the kind.** A code rule checks files; read [references/code-rules.md](references/code-rules.md). A conversation rule
   checks a command the agent runs; read [references/conversation-rules.md](references/conversation-rules.md). Conversation
   rules are in internal testing: Rerail 0.1.0 releases code rules only.
   - Give it the id `local:<short-kebab-name>` and `revision` 1.
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
   - Read every repository match the report shows, and decide whether each is a real violation.
   - `repository.leftOut` counts files and folders the repo pass did not read: hidden, secret-named, or ignored by Git.
     Its matches say nothing about those.
   - A rule that matches hundreds of places fights the repo's convention.
   - The message passes the harmful-steering check.
   - An architecture rule passes [Proving it against history](references/architecture-rules.md#proving-it-against-history):
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

     What it checks   <in plain words>
     Where it runs    When I edit a file under <folder> | When I run <command> | When you or I say <…>
     Applies to       All of <this repo's name> | This worktree only (<its folder name>)
     What happens     Steer: I'm told in the session and fix it. I'm not stopped.
     How often        Every time | The first time only
     Tested           Would have caught <N> places in this repo

   Add this rule?
     1. Yes, add it
     2. Change something
     3. Show me the rule
     4. Cancel
   ```

   - **Rows.** Applies to is "All of <repo>" unless the person asked for this worktree, branch or experiment only; repo is the
     name `rules test` and `rules add` use (the checkout's folder name). Where it runs names the
     excludes that matter: "When I edit a file under src/ (not src/scripts/ or tests)". How often is "Every time" for a code
     rule; for a command rule it follows `repeat.mode` (`always`: Every time, `once`: The first time only). Tested names the
     places: "Would have caught 2 places in this repo (src/api/refunds.ts:2, …)"; with none, "Matches nothing in this repo
     today: the code already follows it". A command rule gets no repo pass, so its Tested row gives the cases: "Caught 4 of 4 cases,
     let 5 of 5 through". That alone is no reason for `Not sure`.
   - **Log rules.** The What happens row reads "Log: Rerail keeps a record. I'm not told."
   - **Rows you are unsure of.** Mark the row `Not sure`. Below the card, show one line from this repo that the rule catches and
     one that it misses, and set the rule to Log.
   - **Rules from the shortlist.** Put one line above the card: `From: <the evidence, e.g. "you corrected this 3 times">`.
   - **Architecture rules.** Put the history result above the card in one line: `Checked against history: caught the first
     draft of #123; quiet on its merge and on 15 recent merged PRs.`
   - **Conversation rules** (command or similarity). Put one line above the card: `Internal testing: this kind of rule is not
     part of Rerail 0.1.0 yet.`
   - **"Change something".** Offer Steer and Log, each with what it means; All of the repo or this worktree only (its rule
     runs nowhere else, not in the main checkout nor other worktrees); and any other row the person names.
   - **"Show me the rule".** Print the rule JSON, then ask again.
7. **Add it, only after the person says yes.** The person's yes to the card is also their approval of what it says: you wrote
   the rule and they read it. For all of the repo:

   ```sh
   rerail rules add <tmp>/draft.json
   rerail rules approve <id>
   ```

   Reply in one line, `● <what it checks> is on, set to <Steer|Log>.`. Add the dashboard link if the command printed one (it does
   when the Rerail daemon is running), and pass on its line saying the repo was set up with this rule, if it printed one.

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
   - Approve it: `rerail rules approve <id>` (it reads the file there).
   - Reply in one line, `● <what it checks> is on in this worktree only, set to <Steer|Log>.`
8. **Offer to show it working (code rules only).** Ask: "Want to see it fire? I'll write a throwaway file that breaks the rule,
   show you Rerail's message, then delete it."
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
   - Any other rule ships with Rerail. You cannot change it. Offer to set it to Log or turn it off in this repo from the dashboard
     (`rerail manage --print` gives the link). If the person wants it changed for everyone, offer a line they can send to the Rerail
     team: what fired, the code, and why following it is wrong.
3. **Fix it** with the most targeted step on the fix ladder in
   [references/rule-quality.md](references/rule-quality.md#the-fix-ladder). Add the misfire as a `passed` case, and show it was
   wrong before: run it against the current rule and see it caught. Keep every earlier case: from the draft file, or, without it,
   rebuilt from the rule's description and the repo. Raise `revision` by one.
4. **Test, read back and add** as in Write, steps 4 to 7; a worktree rule's file is written over in place. The edit waits for
   approval again, and the person's yes to the read-back gives it. In the read-back, add one line above the card, at most two
   sentences:
   `Changed: inline type imports (import { type User }) now pass; a mixed import is still caught.`

## Approve

Read [references/approving-rules.md](references/approving-rules.md) first.

1. **See what waits:** `rerail rules approve` with no name lists every rule and pack waiting here, each with where it comes from.
2. **Audit each one you did not write in this session**: read all of its content, and check what it would steer the agent with,
   as the reference says. You are looking for anything a reviewer would not expect a code rule to say or do.
3. **Ask with the reference's card**, one per rule or pack, with every flag you found. Recommend keeping it in Log when you found
   anything.
4. **Approve only after a yes:** `rerail rules approve <id or package>`. Approve exactly what you read; if it changed since, read
   it again.

## Never

- Add, change or remove a rule the person has not confirmed in this session.
- Approve a rule or pack the person has not said yes to in this session, or one whose content you have not read in full.
- Offer "every repo". A rule from here belongs to this repo only, or to this worktree only.
- Use `block` mode.
- Invent evidence, cases or test numbers, or quote the person beyond what the evidence shows.
- Write instruction files, rule files or tests into the repository, or leave any file there. The exceptions: a rule the person
  keeps to this worktree, written to `.rerail/rules/` after their yes, and Write step 8's throwaway file, deleted before you
  reply.
