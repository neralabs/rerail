# Branch coordination: comparison with Amp

> **Historical.** This note compared the decision-sharing design, since removed (Git history keeps it under the tag
> `merge-assist-capture-experiment`); what Merge Assist delivers is in the PRD's
> [Delivered scope](merge-assist-prd.md#delivered-scope).

Spike note for Merge Assist ([PRD](merge-assist-prd.md)). It compares Amp (ampcode.com) with the four candidate designs: landing gate, continuous radar, intent ledger and speculative integration. Amp docs were read on 2026-11-02 (docs index: [idx]). Unmarked statements about Amp are documented and cited. `[INFERENCE]` marks our own reasoning. "Not found" means we searched all 64 markdown pages listed in [idx] for `conflict|overlap|collid|collision|lock|same files|semantic|incompatib|rebase|stale`, plus the Chronicle news list [news], and found nothing beyond what is cited here.

## Summary

- Amp handles parallel work by isolating it. Each thread gets its own orb (a cloud VM) or its own runner worktree, and coordination is left to the human, to Puck, or to agent-to-agent messages [orbs][a2a][puck][runners].
- Amp deals with conflicts only at **ship time**. Ship commits, fetches, rebases onto the base branch, runs the full test suite and pushes. When the push races another change, the agent resolves the conflict and asks you about substantive ones [ship].
- Amp has no documented detection of overlap or design incompatibility between concurrent threads or branches. It never compares one in-flight branch with another; it only compares a branch with the base branch it lands on (not found; see method above).
- Amp is ahead on one-click landing, agent-run conflict resolution, CI and PR follow-up, and team-visible, file-searchable threads [ship][gh][find].
- Rerail differs in four ways. It works across agent hosts. It is local-first, with no hosted service. It judges with the user's own agent CLI. It compares unpublished local branches and teammates' PRs pairwise **before** landing.

## Amp's model of parallel work

- **Threads**: one thread per task [threads]. A thread runs in the CLI on your machine, in an orb, or on a runner [threads].
- **Orbs**: remote sandboxed VMs. Each thread gets a fresh, isolated environment with a clone of the repo [orbs][orbsx]. Orb compute is provided by e2b [sec]. Each user can start 20 orbs in a burst, then one more every 5 minutes [orbs].
- **Runners**: your own machines. "When several threads should work in the same repository at the same time, give each one its own Git worktree" [runners].
- **Subagents** (Task, Read Thread) work in isolation and "can't communicate with each other" [models].
- **Agent to agent**: threads can start other threads, message them and exchange files. Each has a separate working copy. The docs advise: "Keep work in the current thread when tasks … edit the same files" [a2a].
- **Puck**: a meta-agent that starts, steers and monitors agents. It "can split a request across several agents and gather their reports". It can send saved prompts such as `ship`, `update-base` and `review` to any thread [puck].
- **Visibility**: in a workspace, threads are shared by default [ws]. `find_thread` finds threads "that touched a specific file", which the announcement calls "`git blame`, but for Amp" [find][prompting].
- **Chronicle** is Amp's news and notes feed, not a product feature [news].

## What Amp does about conflicts, and when

| Moment | Mechanism |
|---|---|
| Plan | Nothing documented. Running `find_thread` by file is manual and pull-based [find]. |
| Continuous | Nothing for overlap between threads. GitHub subscriptions follow your own PR and its CI, and "CI messages steer an active agent" [gh]. |
| Ship | Default flow: commit, `fetch origin`, rebase onto `origin/main`, run the full test suite, then push to base. If the base branch moved, rebase again and resolve. A substantive conflict leads to "checks with you". A failing test is ignored only if it "also fails on the base branch" [ship]. |
| Ship (variants) | **Push to Branch** pushes a feature branch and reports the PR URL. **Custom Ship** runs a free-text prompt, for example one that uses `gh pr create` or a merge queue [ship]. **Restack** tidies commits [ship]. |
| Review | Changes pane, and **Compare With Branch…**, which diffs against another remote branch's merge base and is manual [threads]. The `review` prompt [puck]. Per-directory **Checks** in `.agents/checks/` run one agent per check [review]. |

Amp's own team pushes straight to `main` with no pull requests [soc2]. `[INFERENCE]` With frequent rebases onto trunk, cross-branch conflict mostly turns into "rebase and retest at land time". Amp has no feature that reasons about design incompatibility between concurrent units of work.

## Amp next to each Rerail design

| Rerail design | Closest Amp feature | Difference |
|---|---|---|
| 1. Landing gate | The Ship flow [ship] | Both act late. Ship compares the branch with the **landed base** only. The gate compares it with **other unlanded** branches and PRs, and judges design as well as text. |
| 2. Continuous radar | None. Nearest are CI and PR subscriptions that steer the live agent [gh] | Amp watches its own PR. Radar watches every pair of branches. Amp is a precedent for steering a session with findings mid-work. |
| 3. Intent ledger | Shared, searchable threads [ws]; Read Thread [models]; thread URL in the PR body and the `Amp-Thread-ID` commit trailer [gh][threads] | Amp keeps the raw intent (transcripts) and the links, but has no structured claims and no overlap check at plan time. |
| 4. Speculative integration | Rebase plus full tests at ship. A failure counts only if base is green [ship] | That is speculative integration against `main`, one branch at a time. Rerail merges pairs of in-flight branches before either one lands. |

## Where Amp is ahead

- Landing is integrated and executes end to end. The agent fetches, rebases, tests, resolves, pushes or opens a PR, and follows CI and reviews afterwards [ship][gh].
- Work has an audit trail across the team. Threads are workspace-visible and searchable by file [ws][find]. Commits carry `Amp-Thread-ID` [threads]. PRs that link a thread show their status in that thread, including merge-queue state [gh].
- Work runs in isolated compute, and agents can message each other [orbs][a2a].

## Where Rerail's design is differentiated

- **Works across agent hosts.** Rerail observes omp, pi, claude-code, codex and cursor sessions. Amp coordinates only Amp threads [a2a][puck].
- **Local-first.** Amp Server stores threads, including partial code, in GCP in the US. Inference is routed by Amp's "thread actors" using credentials the server issues [sec]. Rerail makes no network requests of its own.
- **Judges with the user's own tokens through their agent CLI.** Amp supports BYOK and subscriptions with no Amp fee, or Amp credits with no markup [pricing][routing][free]. `[INFERENCE]` From [sec], BYOK requests still pass through Amp's backend.
- **Sees unpublished local branches from any agent, and teammates' PRs**, including PRs not written with Amp. Amp sees its own threads' working copies, plus PRs that link a thread [gh].
- **Judges design incompatibility before landing.** Amp only detects textual conflicts, and failures its test suite catches against base.

## Ideas worth borrowing

1. **A baseline differential.** Amp ignores a failing test only after confirming it also fails on base [ship]. Use the same rule as the verdict in design 4: red on the merged tree but green on each branch alone.
2. **Escalate substantive conflicts to the human** ("checks with you"), and let trivial ones pass. This fits the gate's split between pass, steer and block [ship].
3. **A file-touch index per session.** Amp offers `find_thread` by file [find]. Rerail's `events` table already records tool calls. Indexing the files each session touches gives a free tier-0 overlap signal that includes uncommitted work.
4. **Auto-link session to commit.** Copy the `Amp-Thread-ID` trailer [threads] to fill `work_links` automatically and to feed intent cards for local branches.
5. **Expose coordination as a named action.** Amp's `ship`, `update-base` and `review` are saved prompts that any thread or Puck can trigger [puck]. Rerail could expose `coordinate` the same way, as an MCP operation and a steer.
6. **Scoped invariants, one judge each.** Amp runs one agent per check from `.agents/checks/` [review]. Use the same pattern for the unimplemented `llm` rule clause.
7. **Suggest consolidation.** Follow Amp's advice to keep same-file work in one thread [a2a]: when a high-overlap pair is found, steer one session to hand off to the other.

## Open unknowns

- The text of Amp's built-in `update-base` prompt, and whether anything triggers it automatically. It is named but not described [puck].
- Whether Checks and `amp review` are still current. They were announced in Feb 2026 [review], but neither appears in the current 64-page docs index (searched for `.agents/checks`, `code_review` and `amp review`).
- Whether Puck or the workspace feed ever warns proactively about overlapping threads. Neither is documented [puck][ws].
- Whether BYOK traffic transits Amp servers. [sec] implies it does, but this is not stated explicitly.

[idx]: https://ampcode.com/docs.md
[news]: https://ampcode.com/chronicle
[orbs]: https://ampcode.com/docs/orbs
[orbsx]: https://ampcode.com/notes/orbs-explained
[ship]: https://ampcode.com/docs/orbs/shipping
[gh]: https://ampcode.com/docs/github
[a2a]: https://ampcode.com/docs/orbs/agent-to-agent
[puck]: https://ampcode.com/docs/puck
[runners]: https://ampcode.com/docs/cli/runners
[models]: https://ampcode.com/docs/models-and-subagents
[threads]: https://ampcode.com/docs/threads
[prompting]: https://ampcode.com/docs/prompting#finding-threads
[find]: https://ampcode.com/news/find-threads
[ws]: https://ampcode.com/docs/collaborate/workspaces
[review]: https://ampcode.com/news/liberating-code-review
[soc2]: https://ampcode.com/notes/thats-not-soc-2-compliant
[sec]: https://ampcode.com/security
[pricing]: https://ampcode.com/docs/pricing
[routing]: https://ampcode.com/docs/customize/model-routing
[free]: https://ampcode.com/news/free-agent
