---
name: rerail-bug-report
description: Diagnose a problem with Rerail, then fix it or report it. Works out whether it is an engine bug (Rerail itself), a bug in a rule that ships with Rerail, or a bug in the person's own rule. Helps fix their own rule, and offers to report the other two in an issue on neralabs/rerail. Nothing is posted without the person. Use when Rerail crashes, hangs or stops steering, or when a rule flags fine code, misses bad code or steers badly.
---

# Report a Rerail bug

Someone thinks Rerail got something wrong. You find out what kind of problem it is, say so in plain words, and take the path
for that kind. The person decides; nothing leaves their computer until they say so.

| Kind | What it is | What you do |
|---|---|---|
| **Not a bug** | Rerail is off here, the rule is on Log, or it works as documented | Say why, and how to get what they wanted |
| **Engine bug** | Rerail itself: a crash, a hang, set-up, the daemon, the dashboard, a lost steer, a rule it could not run | [Report](#report) with the Bug report form |
| **Shipped rule bug** | A rule from a pack that ships with Rerail flags fine code, misses bad code or steers badly | Offer a way around it, then [Report](#report) with the Rule misfire form |
| **Own rule bug** | The same, in a rule the person or their team wrote | [Help fix it](#fix-their-own-rule) |

## Before you start

- **A vulnerability does not go in an issue.** If this is a security vulnerability, tell the person to follow
  [SECURITY.md](https://github.com/neralabs/rerail/blob/main/SECURITY.md), and stop.
- **The launcher.** Use the one your `/rerail-bug-report` command named. Without one, use `~/.rerail/bin/rerail` if it exists,
  else `rerail`. Below it is written `rerail`. Paste it in full each time; it may start with an environment assignment, which
  does not survive being put in a shell variable.

## Diagnose

1. **Get what happened.** Take it from the person's words and this session: what they did, what happened, what they expected.
   A `◆ Rerail` message names the rule id and the `path:line`. Ask once, and only for what is missing.
2. **Write the support bundle and read it.** It is a local file; writing it sends nothing.

   ```sh
   rerail support-bundle --json
   ```

   It reports this repository's latest session. If the problem was in an earlier one, ask for its id, which is in its dashboard
   address (`sessionId=…`), and run `rerail support-bundle --json --session <id>` instead. Read the file the output's `path`
   names. It can wait about 12 s for a busy daemon (a 2 s health check, then 10 s more), so give it a minute; if it is
   stopped while it waits, read the file its waiting message names, which already holds a bundle. If the command fails, its
   message is your evidence: the kind is an engine bug, and there is no bundle to attach.
3. **Check the version.** Run `rerail update check`. If a newer version exists, say so and suggest `/rerail-update` and trying
   again before reporting. The person can still report it.
4. **Work out the kind**, in this order, and stop at the first that fits:

   **Not a bug**, when one of these explains it:

   - `repository.state` is not `started`: Rerail is not on here. `/rerail-start` starts it.
   - Nothing steered, and the rule's `on` is false or its `mode` is `log`: on Log, Rerail keeps a record and the agent is not
     told. A Log rule that recorded fine code as a violation is still a rule bug.
   - The rule is not in `repository.rules`: it is not on in this repository, or its pack is not installed
     (`rerail packs add <id>`).
   - The rule is right and the person does not want it here. That is a choice, not a bug: offer Log or off from the dashboard
     (`rerail manage --print` gives the link), and never change it yourself.
   - [Troubleshooting](https://github.com/neralabs/rerail/blob/main/docs/troubleshooting.md) describes it: follow that.

   **Engine bug**, when no rule is at fault:

   - The bundle says so: `daemon.engine.ok` is false, `problems` is not empty, `rerail.skippedPacks` or
     `repository.unavailableRules` shows something that should have loaded, or `session.audits[].failures[]` shows a rule that
     could not run.
   - A rule should have fired and nothing recorded it: `session.gaps` counts dropped events or unchecked boundaries.
   - No rule is involved: a crash, a hang, set-up, an update, the dashboard, or a command that does other than its docs say.

   **A rule**, when it fired wrongly or should have fired. Find it in `repository.rules`:

   - `pack` is `worktree`: the rule is in this worktree's `.rerail/rules/`, so it is the person's: an **own rule bug**.
   - `pack` is a pack Rerail did not publish (`rerail packs list` names a third party's pack by its package name, outside
     `@neralabs/`, and one from a folder with `(from the folder <path>)`): not Rerail's to fix. If the person built it, it is
     theirs: help them fix it in the pack's source. Otherwise offer Log or off from the dashboard, and point them to its
     publisher; never report it on neralabs/rerail.
   - Any other `pack`: a **shipped rule bug**. Do not try to split the pack's rule from the engine behind it; the Rule misfire
     form reaches the same maintainers.
   - `pack` is null: an **own rule bug**. Its id is also in the output's `ownRules`.

5. **Say what you found**, then take that path:

   ```text
   ● <Not a bug | Engine bug | Shipped rule bug | Own rule bug>
     Why    <the evidence, in one line: the field, the error or the rule id>
     Next   <what you suggest>
   ```

   If you cannot tell, say so and what you checked, and treat it as an engine bug: a maintainer can sort it.

## Fix their own rule

Read [../rerail-create/SKILL.md](../rerail-create/SKILL.md) and follow its Tune section: reproduce the misfire as a case, fix the
rule, test it, read it back, and add it only after the person says yes. Never change the rule before that yes.

If reproducing it shows the rule is right and Rerail is not, this is an engine bug: a case marked `not checked` by
`rerail rules test`, or code the [rule language](../rerail-create/references/code-rules.md) says should match and does not. Tell
the person, and go to [Report](#report).

A fixed rule needs no issue. If the person still wants to raise something with the Rerail team, go to [Report](#report).

## Report

For a shipped rule bug, first offer the way around it while a fix comes: Log or off in this repository, from the dashboard
(`rerail manage --print` gives the link). It is the person's choice.

1. **Look for it first.** With `gh`, run `gh issue list --repo neralabs/rerail --state all --search "<words>"`; without it, give
   them https://github.com/neralabs/rerail/issues. Search with a shipped rule's id or the words of an error, never with an own
   rule's id, a path or a project's name. If one matches, show it, and offer to add to it instead.
2. **Check the bundle with them**, because the issue is public. Prompts, replies, tool input and output, source code and rule
   text are left out by design, and known secrets and paths are replaced. Patterns cannot catch everything, so look through what
   is still free text, and whatever else could identify someone:

   - `session.steers[].text`: a steer can quote the person's code.
   - `daemonLog` and `problems`: log lines and errors can hold names, addresses or values Rerail doesn't know are private.
   - `session.audits[].violations[].path` and `failures[]`: a file path can name a client or a project.
   - `repository.rules`: the ids in the output's `ownRules` were written by the person's team.
   - `session.timeline[].tool`: an MCP tool's name can name an internal server.

   Look for secrets (tokens, keys, passwords, connection strings), personal data (names, emails, usernames, IP addresses), and
   anything private to their work (client and project names, internal hostnames and URLs, proprietary code). Tell them, briefly,
   what the file holds and each thing you found, quoted, with where it is. Say plainly if you found nothing, and that they should
   still read it themselves. If they want something out, replace that value in the file with `<redacted>`, keep the JSON valid,
   and remove nothing else: a maintainer reads the rest.
3. **Draft the report** and show it. Each form's text fields take their value from a query parameter named after the field:

   | Form | `template` | `title` | Fields |
   |---|---|---|---|
   | Bug report | `1-bug.yml` | what went wrong, in a line | `rerail-version`, `agent-version`, `what-happened`, `reproduction`, `output` |
   | Rule misfire | `2-rule-misfire.yml` | `[rule] <rule id>: ` and what went wrong | `rule-id`, `pack`, `rerail-version`, `code`, `expected`, `steer` |

   `rerail-version` is the bundle's `rerail.version`. `pack` is `<id>@<version>` from `rerail.installedPacks`. Write `code` as the
   smallest file that shows it, made up for the report: never paste the person's source unless they say yes to that text. Leave
   out of the draft anything they asked to keep private. `steer` can quote their code, so check it the same way.
4. **Ask once:**

   ```text
   Report this on neralabs/rerail? Issues are public.
     1. Yes, give me the form, filled in
     2. Yes, file it for me
     3. Not now
   ```

   Offer 2 only if `gh auth status` says they are signed in.

   - **The form.** Build the address, percent-encoding every value, and reply with this. Keep it under 4000 characters, since
     GitHub answers a longer one with `414 URI Too Long`: leave out the longest field and put its text in a fence to paste.

     ```text
     ● Your report is ready. Nothing has been sent anywhere.
       Bundle    <path>
       Form      https://github.com/neralabs/rerail/issues/new?template=<template>&title=<title>&<field>=<value>
       You do    pick the dropdowns, tick the checks, drag the bundle into Support bundle, and submit
     ```

   - **File it for them.** Show the exact title and body, get a yes to that text, then write the body to a temporary file outside
     the repository and run the command below. It leaves the form's labels to the maintainers. It cannot attach a file, so reply
     with the issue's link and tell them to drag the bundle into a comment. Never paste the bundle's contents into the body.

     ```sh
     gh issue create --repo neralabs/rerail --title "<title>" --body-file <tmp>/issue.md
     ```

   - **Not now.** Say that nothing was sent, where the bundle is, and that the form is at
     https://github.com/neralabs/rerail/issues/new/choose if they change their mind.

## Never

- Upload, post, send or paste the bundle or its contents anywhere, or open an issue, unless the person asks you to in this
  session.
- Put in the bundle anything the command did not write, other than `<redacted>`.
- Change or turn off a rule the person has not confirmed in this session.
- Call something a bug in Rerail without saying what evidence points there.
