# Rule candidates from Jean-François Gagné's pull requests

An analysis for Jeff, not a product document: it lists review findings that keep coming back on JF's pull
requests, and the Rerail rules that could steer his coding agents away from them at edit time. No rule is written
yet and no code is changed. The method follows [packs/rerail/HISTORY.md](../packs/rerail/HISTORY.md): a candidate
counts only with real occurrences behind it, cited by pull request, commit and file:line.

## Sources

| Source | What was read |
|---|---|
| Pull requests | All 170 by `JeanFrancoisGagne` (#498–#1126, 2026-11-09 to 2026-11-26): 110 merged, 55 closed unmerged, 5 open |
| Review bot | 317 `neralabs-reviewer[bot]` rounds: 494 findings in their summaries (39 blockers, 258 follow-ups, 197 nits), 475 of them new rather than carried over; 401 line comments hold the findings' full text |
| Human reviewers | 284 reviews and comments by Moti-Velz, gdgiangi-nera and jeffreysnell |
| Fix-up commits | 3,396 branch commits; 752 fix-ups (599 that JF's replies name by SHA as the fix for a finding, and 153 later commits with review-type messages), each read as a diff |
| Follow-ups on `main` | All 298 commits by people (not bots) on `main` since JF's first; every changed line traced back to its author: 45 commits by others and 83 by JF touched lines JF wrote |
| Identities | `Jean-François Gagné` (GitHub squash merges), `JF Gagne` and `Jean-Francois Gagne` (`jfgagne50@gmail.com`, branch commits) |

"Today on main" is `origin/main` at `989a882edd`. GitHub API responses and the analysis files are cached under
`/tmp/jf-mining/` on Jeff's machine, not in the tree. An LLM sorted each finding and fix-up into a theme. Every shape
and count in the candidate sections was then checked against git, by pattern or by blame. The fix-up counts are
approximate: a commit may carry fixes for several themes and is counted under its main one.

The repository enables two packs (`.rerail/packs.json`): `complexity` with ceiling 10, and `packs/rerail`. The
`typescript`, `javascript` and `node` packs are not enabled, and there is no ESLint or Biome for TypeScript, so rules
those packs hold do not run here.

## Ranking

Score = (3 × blockers + 2 × follow-ups + 1 × nits + 3 × human findings + 0.5 × fix-up commits, plus 3 for each
flake-fix pull request the shape caused), multiplied by a checkability factor:

- 1.0 for a code rule that sees the whole shape.
- 0.8 for a code rule with a known false-positive share.
- 0.5–0.7 for a code rule that sees only part of the theme.
- 0.4 for a conversation rule that can only remind.
- 0.2 for reviewer-only.

The weights are a judgement call. The ordering changes little under other weights, except that the conversation rule
stays first whatever factor it gets.

## Summary

| # | Candidate | Rule | Findings B/F/N/H | PRs | Fix-ups | Main today | Checkable | Covered by | Mode | Score |
|--:|---|---|---|--:|--:|--:|---|---|---|--:|
| 1 | [`rerail-refresh-pr-text-before-review`](#rerail-refresh-pr-text-before-review) | Before asking for a review round, bring the PR description, the `.changelog/` note and the changed functions' doc comments up to the head | 0/27/41/29 | 61 | 123 | — (commands) | conversation rule on `gh pr comment … @neralabs-reviewer review`, `gh pr ready` | none | log (block once enforced) | 97 |
| 2 | [`rerail-test-no-real-time`](#rerail-test-no-real-time) | A test waits on an event, a stepped clock or counted work, never on a fixed sleep or a wall-clock bound | 0/5/2/1 + 4 flake-fix PRs | 24 | 41 | 120 (JF 90) | code rule, TS AST | none | log | 38 |
| 3 | [`rerail-no-silent-catch`](#rerail-no-silent-catch) | Product code never turns every error into a fallback with no narrowing and no log line | 0/9/9/0 (family: 2/29/15/2) | 17 | 8 (family 29) | 353 (JF 177) | code rule, TS AST | none (`eslint-no-empty` not enabled, `.js` only) | log | 25 |
| 4 | [`rerail-test-bounded-waits`](#rerail-test-bounded-waits) | A test's wait on a program or server ends when it fails (listen without `error`, unraced `once(stdout)`, sync PowerShell without `timeout`) | 0/3/9/0 | 17 | 16 | 29 (JF 22) | code rule, TS AST | none | log, then steer after a one-commit sweep | 18 |
| 5 | [`rerail-process-time-limit`](#rerail-process-time-limit) | Product code bounds every program it starts with a `timeout`; the daemon's thread never waits synchronously | 1/3/0/6 | 10 | 4 | 4 (JF 3) | code rule for the start; reviewer for the daemon thread | none (`check-owned-builtins` checks no time) | log, steer after 4 lines | 17 |
| 6 | [`rerail-powershell-module-qualified`](#rerail-powershell-module-qualified) | A cmdlet in PowerShell script text in `src/` is named with its module | 0/0/0/5 | 3 | 2 | 10 (7 real) | code rule, regex | partly: `cmdletsWithoutModule` test on 3 files | log, then steer | 13 |
| 7 | [`rerail-row-parse-guard`](#rerail-row-parse-guard) | A stored column or line parsed per row is parsed inside that row's own guard | 0/7/0/0 | 7 | 7 | 6 (JF 0) | code rule (relational) for `JSON.parse` in a loop; Drizzle json columns: reviewer | none | log | 8.8 |
| 8 | [`rerail-comment-width`](#rerail-comment-width) | A comment line stays within the file's 120-column wrap (flag >130) | 0/0/5/1 | 7 | 5 | 322 >140 (JF 36) | code rule, line regex | none | log (steer on added lines only) | 8.4 |
| 9 | [`rerail-parse-unknown`](#rerail-parse-unknown) | Parsed data is typed `unknown` and narrowed, never `JSON.parse(x) as T` | 0/2/3/0 | 5 | 5 | 15 (JF 3) | code rule, TS AST | none | log | 7.6 |
| 10 | [`rerail-test-exact-assertions`](#rerail-test-exact-assertions) | No early `return` in a test body, no bare `toBeInstanceOf(Error)`, no `toBeTruthy()` | 0/0/2/1 | 3 | 3 | 4 | code rule, TS AST | none (`vitest-*` in `typescript` pack, not enabled, other shapes) | log, then steer after 4 lines | 6.5 |
| 11 | [`rerail-test-cleanup-in-finally`](#rerail-test-cleanup-in-finally) (assertion-in-`finally` clause only) | A test's `finally` never asserts (`expect`, `expect.poll`, `vi.waitFor`) | 0/1/1/0 of the theme's 0/4/11/0 | 13 | 2 of 12 | 3 | code rule, TS AST; the rest of the theme by vitest config | none | log, then steer after 3 lines | 4 |
| 12 | [`rerail-test-home-and-folders`](#rerail-test-home-and-folders) | A test moves the home with `stubUserHome`, never `vi.stubEnv("HOME")` | 0/0/0/1 | 2 | 2 | 13 (JF 7) | code rule, regex | CONTRIBUTING.md prose only | log, then steer | 4 |
| 13 | [`rerail-changelog-security-type`](#rerail-changelog-security-type) | A `.changelog/` entry describing a closed vulnerability is typed `security-` | 0/0/0/1 | 3 | 2 | 0 | code rule, text regex | none (`core-changelog-check.mjs` checks the type exists) | steer | 4 |
| 14 | [`rerail-rustfmt-before-commit`](#rerail-rustfmt-before-commit) | Run `check-rustfmt.mjs --write` before committing Rust | 0/0/0/0 (CI catches it) | 7 | 16 | 0 | conversation rule; better a pre-commit line | CI only | prefer the pre-commit line | 3.2 (8 as a pre-commit fix) |

Not proposed, with the numbers behind each, are in [Considered and not proposed](#considered-and-not-proposed):
`rerail-monotonic-durations`, `rerail-log-shape`, complexity (already a Steer rule), shadowed imports, import
hygiene, short reads, delete-before-replace, unhandled rejections and the wasm32 arm.

## Cheaper than a rule

Five fixes outside Rerail would each remove a class of finding outright:

| Fix | Removes | Evidence |
|---|---|---|
| `vitest.config.ts`: `restoreMocks: true, unstubEnvs: true, unstubGlobals: true`, and `afterEach(() => vi.useRealTimers())` in `test/support/setup-rerail-home.ts` | Shapes (a)–(c) of `rerail-test-cleanup-in-finally`: 122 sites on main where a stub, spy or fake clock can leak into later cases | #1118 N2, #1095 F2, the timer half of #1104 N3; fix-ups 8dd354c710, d78c55d048. Breaks only `test/engine/git-moved.test.ts:83` (`stubEnv` in `beforeAll`) |
| `scripts/check/pre-commit.sh`: run `node scripts/check/check-rustfmt.mjs` | 16 rustfmt-only fix-up commits in 7 PRs, each a CI round trip (#812 alone has six) | `rerail-rustfmt-before-commit` |
| Require a time limit in `src/engine/child-process.ts`'s types | The start half of `rerail-process-time-limit`, enforced by the compiler instead of a rule | `check-owned-builtins.mjs` already routes every start through that module |
| `tsconfig.json`: `noUnusedLocals` | About 14 unused imports on main, and the unused-import nits (#679, #881) | `mechanical-minor` below |
| Make sure JF's agent sessions run Rerail's hooks with this repository's packs | The 15 complexity blockers and 21 complexity fix-ups, all from a rule that is already in Steer mode | All came before #864 raised the ceiling to 10; none since. Whether his sessions run the hooks is not visible from GitHub (see [Open questions](#open-questions)) |

## Reviewer-only themes

These recur at least as often as the candidates, but no pattern sees them. They stay with the reviewer bot and the
people. Counts are bot plus human findings (items / PRs) and fix-up commits (commits / PRs).

| Theme | Findings | Fix-ups | What recurs |
|---|---|---|---|
| Doc drift | 59 / 35 | 91 / 40 | A comment, docstring, threat-model or reference paragraph still describes the old behaviour. The re-read is part of candidate 1's steer |
| Vacuous tests | 32 / 24 | 56 / 35 | A test passes with the product broken: a stale module copy, identical variants, a check that a call happened rather than what it did. Only 9 of 41 name a matcher (candidate 10) |
| Missing tests | 27 / 21 | 68 / 37 | No case for a new fallback, platform arm or limit |
| Cancel and race | 32 / 16 | 48 / 22 | Work continues after `close`/`stop`, or state is taken before an `await` and lost when a close lands in between |
| Unbounded | 18 / 12 | 22 / 19 | A file read whole with no size or type check, or a module-level `Map` never pruned |
| Failure taken as an answer | 28 of the 46 in candidate 3's family | — | An OS or Git probe's error read as "local", "no-pipe" or "running"; a transient timeout cached as permanent |
| One damaged row fails the read, through Drizzle | 3 of 7 in candidate 7 | — | A `mode: "json"` column or `.mapWith(…)` parses every row, out of a syntactic rule's sight |
| Domain logic | 155 / 56 | 214 / 51 | One-off bugs in the feature at hand |

Human-only themes in the same vein: duplicated guard logic ("two earlier rounds found the copies drifting", Moti, 5
PRs), a different design asked for (#674, #676, #921, #530), and synchronous waits on the daemon's thread (Moti and
gdgiangi, 4 PRs; part of candidate 5).

## Process issues (not rules)

| Issue | Evidence |
|---|---|
| Fan-out, then mass close | 53 PRs were opened on 2026-11-12. 52 of them were closed the same day after a median 0.9 h open, with Moti's note that they were "generated in a way that makes it hard to review … split from related work, opened alongside many others". JF's close notes said each would join one of 7 capability PRs, and none of those was opened. On #565 JF's agent reopened a PR "around your process" (JF 6841372527). 21 closed PRs have a proven successor (e.g. #578→#658, #606→#665) |
| Approach rejected late | #674 (+6,012 lines, 16 reviews, 115 commits): jeffreysnell, "It routes around the cause … adds a second source of truth" (6988914797). #676 (15 reviews): gdgiangi chose the smaller fix, which landed as #753. Moti on #530: "carry only what fixes #550" (6857711163) |
| Long descriptions | Every PR description is at least 3,623 characters: median 16,272, p90 41,946, max 82,667 (#677). Maintainers edited JF's descriptions 29 times in 12 PRs. Stale or overclaiming descriptions are part of candidate 1 |
| Review rounds | 317 bot rounds on 112 PRs (median 2, max 11 on #964). JF asked for a new round 254 times, and 52 PRs got new commits after the bot had approved. 62 of the bot's 709 reviews (counting the empty ones that only carry line comments) raised a description, release-note or doc finding, and in 16 rounds those were the only new findings |
| Merging main | 324 "Merge main" commits, and 56 more "Merge branch 'main'", across 84 PRs; #964 has at least 29 (the API lists at most 250 commits per PR), #921 and #904 15 each. On #674 the bot reviewed only the PR's own diff, because the full diff was mostly main's changes (review 5953011590) |
| Size | Additions: median 360, p90 2,506, max 20,641 (#503, 455 files). 10.9 lines added per line deleted; 57% of added lines are tests |
| Flaky tests | 19 later PRs only fix flaky tests. Tests from at least 7 of JF's PRs are among them: #679→#1021, #904→#935, #978→#1060/#1114, #677→#961/#988, #739→#986, #648→#1055, #503/#680→#1109. Candidate 2's shape caused 4 of the flake fixes (#1021, #1114, #1055, #806). Red CI was rerun rather than fixed on #565, #648, #657, #812 and #1053 |
| Follow-ups pushed later | gdgiangi on #1006: "please try to resolve follow-ups in the original PR rather than spin out more works to do, with agent driven work the follow up backlog gets massive super quick" (7114438370); also #801 (6988908866). Moti on #912 and #756: "Address the followups". On #997 and #987 humans re-raised bot findings still open at the head |
| Platforms nobody ran | Humans ran the macOS tests that JF's descriptions list as "never run" (#997, #987, #978, #921, #739). One failed only on macOS, a `tmpdir` path under a symlink (#978) |
| Maintainers on the branch | "Pull before your next push": maintainers pushed fixes to #680, #657, #739 and #503 |
| Held for the engine merge | gdgiangi moved #677, #756, #812, #813, #816 and #826 to draft during the 0.2.0 engine merge (#508), and each needed revalidation |
| Decision reports | 186 commits on `JeanFrancoisGagne/pr-reports` cover 109 PRs, 30 of them written before the PR had a number |

## Candidate details

Ranked as in the summary.


<a id="rerail-refresh-pr-text-before-review"></a>

### 1. `rerail-refresh-pr-text-before-review`: before you ask for review, bring the PR description, the release note and the changed functions' doc comments up to the head

- **Pattern**: JF's agent asks for a review round (`@neralabs-reviewer review` 254×; `gh pr ready` 120×, which asks CODEOWNERS' three people) after pushing code, but the description still names an earlier head, the `.changelog/` entry claims more than the diff does or has the wrong type, and doc comments of changed functions still say what they did before. **Why it costs**: "The release note is the only place people upgrading will read about this … 'no longer start' is stronger than what the code does" (Moti, #978, 4670038959). "The description said 'It no longer fixes #623', which GitHub reads as the closing keyword … so merging would have closed #623" (gdgiangi, #665, 6963629675).
- **Evidence**: 97 findings in 61 PRs: 68 bot (0 B / 27 F / 41 N; doc-drift 45, stale description 12, release note 11), which is 14% of the 475 new bot findings, and 29 human (release note 10, doc 10, overclaim 6, stale 3) in 17 PRs. Also 8 fixes made by humans themselves, and 29 edits of JF's PR descriptions by Moti or gdgiangi in 12 PRs. 123 fix-up commits in 45 PRs (doc 91, release note 32, 32 of them under `.changelog/`).

  | PR | Ref | Commit | File:line | Code / finding |
  |---|---|---|---|---|
  | 943 | bot 4647712361 (F1) | head a53166a1d | test/engine/code-rule/worktree-walks/worktree-walks.test.ts:543 | "pr.md says the head is 5c9e5e247 … 'costs 35 added and 9 removed lines' … The diff is +61 -25." The description was edited after the head commit and still described the earlier one. |
  | 673 | bot 4597838817 (F1) | head 2a037a75b | .changelog/changed-hooks-start-the-daemon.md:1 | "The PR body's checklist still says 'it edits `.changelog/added-status-says-when-nothing-is-recorded.md`' … CI … still given at a4843928b". The last description edit was on Nov 15; the head commit was Nov 17. |
  | 665 | JF 6962918159; gdgiangi 6963629675 | (description) | PR body | `It no longer fixes #623, which stays open.` A closing keyword in a negated sentence. gdgiangi reworded the description (2 edits). |
  | 978 | Moti 4670038959 (F1) | fix d149386ef2 | .changelog/fixed-git-calls-start-no-repository-program.md:1 | `…no longer start a filter or signature program only a repository's config names…`: filed as `fixed-` although it closes a vulnerability, and it overclaims. Refiled as `security-…` with "unless the config changes while the call runs". |
  | 1118 | bot 4707349062 (N1) | fix dc8a1b9221 | .changelog/fixed-walks-after-a-suspend-or-clock-change.md:1 | `On every system, setting the system's clock no longer delays or hurries those walks.` The 10-minute reread is still timed on `Date.now()`. |
  | 676 | bot 4568551050 (N1) | fix 59eb16859d | src/runtime/repository-start.ts:172 | `` `workspace`'s start state without side effects: no identity file, database, or schema is created ``, but on Windows it now migrates the file and copies rows. |

- **Today on main**: no code shape to count, because the rule reads commands. On JF's 170 PRs it would have fired on 374 commands (254 trigger comments, which are exactly the text `@neralabs-reviewer review` with no trailing newline, so `gh pr comment N --body …` or `gh api …/issues/N/comments -f body=…`; plus 120 `gh pr ready`). Others: jeffreysnell posted 23 trigger comments and 25 team requests, Moti 4 comments, gdgiangi 1 comment and 4 team requests. Body edit history (GraphQL `userContentEdits`, cached in extra/edits/) shows JF had edited the description after his last commit before 212 of 254 triggers (83%) and before 112 of 120 `ready` events. Of the 12 bot stale-description findings, 5 came with no edit since the head commit and 7 with an edit that left the old text. Refreshing is already a habit, so the steer has to say what to check, not only "edit the body".
- **Checkable**: conversation rule (Bash `input.command`, regex). It cannot check the description; it can only remind at the handoff. **Shape**: `sample {source: "tool:*", boundary: "tool-call"}`, `all[` `R1` = `(?:^|[;&|(\n\x60]|\$\()\s*(?:[A-Za-z_]\w*=\S*\s+)*gh\s+(?:pr\s+(?:comment|ready|edit|review)|api)\b`, `R2` = `@neralabs[-/]reviewer\s+review\b|--add-reviewer[=\s]+(?:\S*,)?neralabs/reviewer\b|\bgh\s+pr\s+ready\b|/requested_reviewers\b`, `not ^\s*(?:rg|grep|egrep|git\s+grep|sed|awk|cat|less)\s|\bgh\s+pr\s+ready\b[^\n]*\s--undo\b` `]`, plus an `any` branch for MCP comment tools (`tool.name` matching `(?i)comment`, and `input` matching `@neralabs[-/]reviewer\s+review\b`); `repeat: always`. I checked 14 cases with Python `re`: all 8 FLAG and 6 PASS cases came out as expected, including a heredoc `--body-file -`, `review full`, `help`, `--undo`, `rg` and `git commit -m`. **Steer**: "Before you ask for review, re-read the PR description, the `.changelog/` entry and the doc comments of every function `git diff origin/main...HEAD` changes: name the current head and its CI, claim only what the diff does and say its limits, type the entry by `.changelog/README.md` (`security-` for a closed vulnerability, `changed-` for behaviour that now differs), and never put a closing keyword in a negated sentence (`no longer fixes #N` closes #N)."
- **Already covered**: partly. `scripts/release/core-changelog-check.mjs` (CI lint job) refuses an entry with no type, one over 60 words, and one that names a ticket, PR or commit. It does not check whether the type is right or whether the claim is true. No rule in packs/* matches `gh pr comment|ready` or the reviewer trigger. `rerail-pack-data-through-publish` only lists `gh pr …` as an exception. The deterministic part of the release-note check is split out as `rerail-changelog-security-type` (separate file). A negated closing keyword is not worth its own rule: #665 is the only case, and none of the 57 commits on main that carry a closing keyword uses one, so it stays in this steer.
- **False positives**: by design, it fires on every legitimate request: 83% of JF's triggers already followed a description edit. It also fires in a maintainer's session that re-asks the bot on someone else's PR (jeffreysnell: 23 trigger comments and 25 team requests on JF's PRs; Jeff's bulk-pr-review skill re-tags each new head), where the agent does not own the description. Only a regex sees the command: a body passed in `--body-file` is never read. JF's descriptions are 16 kB on median and 120 of 170 name a head SHA, so a body is probably never inline and its text cannot be checked [INFERENCE: JF's session transcripts are not on this machine]. Rerail samples the call (`host-runtime.ts`, `kind === "call"`), and `block` is not enforced yet, so the comment is posted and the bot job starts before the agent reads the steer. For the round being asked, the steer is late.
- **Mode**: log, because every match is a sanctioned command (packs/rerail steers only shapes main never shows), and conversation rules are still `experimental-off`. Move it to `block` once block enforcement lands: holding the trigger until the agent re-reads its texts is the only form that saves the round. Reviewer cost it targets: 62 of 709 bot reviews (8.7%) raised at least one of these findings. In 16 rounds they were the only new findings (7 not approved), and 7 of those 16 were followed by another trigger.


<a id="rerail-test-no-real-time"></a>

### 2. `rerail-test-no-real-time`: A test waits on an event, a stepped clock or counted work, never on a fixed sleep or a wall-clock bound

- **Pattern**: a test paces itself on the real clock: `await sleep(250)` / `await new Promise(r => setTimeout(r, 500))` before it acts or asserts, a `Promise.race` with a literal timer, or `expect(performance.now() - started).toBeLessThan(1_000)`. A busy CI runner (Windows above all) misses the margin. **Why it costs**: "A fixed 250 ms sleep stands in for 'the server has the connection'" (bot, #652 F2); "Lifecycle tests rely on tight real-time margins … On a slow runner these can flip, and this one test costs over 10 s of wall time per run" (bot, #665 4568501850/F4). The fix-up message for #806 0cb70bb85e (Timo) says: "measured the time from the stop to the move's end against 100 ms … on a loaded Linux runner it took 122 to 290 ms".
- **Evidence**: 8 findings (0 B / 5 F / 2 N / 1 human) in 24 of JF's PRs; 41 fix-up commits: 31 by JF while the PR was open (theme `test-real-time-margin`, 20 PRs) and 10 later fixes on main of JF's merged tests (7 by JF himself, 3 by Timo). Of the 12 flake-fix PRs that fixed JF's tests, 4 had this exact shape (#1021, #1114, #1055, #806). 4 more were timing races the regex cannot see: #961 and #988 (a real PowerShell child raced against a 1 s product limit), #1109 (`vi.waitFor`'s default 1 s), and #1058. The other 5 were readiness races, process counts or clean-up (#935, #1060, #824, #986, #1075).

  | PR | Ref | Commit | File:line | Code / finding |
  |---|---|---|---|---|
  | #652 | F2@6bd876ba6 | intro e2b2dc9b2c, fix 4a5a40d1c8 | test/runtime/engine-service/engine-service-concurrency.test.ts:123 | `await sleep(250);` (5 such lines, replaced by diagnostics-channel events) |
  | #648→#806 | followup 0cb70bb85e | intro fdb46ef136 | test/engine/audit/audit-rule-provenance.test.ts:1357 | `expect(endedAt - stoppedAt).toBeLessThan(100);` |
  | #648→#1055 | followup c57506aab6 | intro fdb46ef136 | test/engine/audit/audit-rule-provenance.test.ts:1061 | `expect(later.readMs).toBeLessThan(20);` (now counts rows) |
  | #978→#1114 | followup c10a645a1c | intro 9de4fcc419 | test/engine/git-guard/submodules.test.ts:1555,1560 | `await new Promise(settle => setTimeout(settle, 500));` … `expect(now() - began).toBeLessThan(3_000);` |
  | #679→#1021 | followup 3b9b967d5c | intro 5492ad7882 | test/engine/git.test.ts:359 | `Promise.race([read, new Promise(settle => { late = setTimeout(settle, 5_000, "still waiting at 5 s"); })])` |
  | #672 | 4578634296/N1 | intro 8dcb850dcc, fix 92078022a8 | test/daemon/daemon-tls.test.ts:322 | `expect(performance.now() - started).toBeLessThan(1_000);` (now counts bytes) |

  Line numbers: rows #806 and #1055 cite the introducing commit; the other rows cite the parent of the fix.

- **Today on main**: 120 lines in 59 files: 61 fixed sleeps, 9 literal-timer races and 50 wall-clock bounds. Blame on every line: JF wrote 90 and others 30 (Timo 17, Gabriel 9, Jeff 4). The run used `git grep -nP` on `origin/main` over `test/** integration/** *.test.{ts,mts,mjs}`, skipping string-literal lines. Patterns, with LIT = `(?:[\d_]+|[A-Z][A-Z0-9_]*)`:
  - sleep: `^(?!.*\b(while|for)\s*\().*(await\s+(sleep|delay|setTimeout|pause)\(\s*LIT\s*[,)]|await\s+new Promise(<[^>]*>)?\(\s*\(?\s*\w+\s*\)?\s*=>\s*\{?\s*setTimeout\(\s*\w+\s*,\s*LIT\s*\))`
  - race: `setTimeout\(\s*\w+\s*,\s*LIT\s*,\s*["`]|Promise\.race\(\[.*setTimeout\([^,]+,\s*LIT\s*[,)]`
  - bound: `expect\(` whose argument holds `(performance.now|Date.now|hrtime.bigint)() -`, `xAt - y`, `…Ms`, `ms`, `longest`, `elapsed` or `took`, then `.toBeLessThan(OrEqual)?(` a literal, a CONST or a `…Ms` name.

  The same patterns flag a removed line in 11 of the 41 fix-up commits. Examples: `test/runtime/daemon-control/daemon-home.test.ts:203 await delay(7_000);` (JF 93b6cd9143) and `test/runtime/engine-service/engine-service-runtimes.test.ts:807 expect(performance.now() - asked).toBeLessThan(1_000);` (JF 2d142dec54). Rust `thread::sleep` in native tests has about 56 lines, 54 of them by Timo, so it is not JF's pattern and stays out of scope. JF's Rust case (#756 F6, `sleep 1; echo done` in a shell child at native/src/git_start/run.rs:402) is a string the regex does not see.
- **Checkable**: code rule (AST, or regex as prototyped). **Shape**: TypeScript/JavaScript. It matches four forms:
  - an `await` of `sleep|delay|setTimeout|pause(<number|CONST>)`, or of `new Promise(r => setTimeout(r, <number|CONST>))`;
  - `Promise.race` with a literal timer;
  - `setTimeout(resolve, N, "message")`;
  - `expect(<a subtraction involving performance.now()/Date.now()/hrtime, or a *Ms/elapsed name>).toBeLessThan*(<literal|CONST>)`.

  Scope is `test/**`, `integration/**` and `**/*.test.{ts,mts,mjs}`. Exceptions: calls inside a `while`/`for`/`do` body (a poll interval, e.g. `until` in test/support/processes/daemon-home.ts:22), inside a function passed as an option or callback to a stand-in (it makes a fake slow), and inside string or template literals (child scripts). Steer: "A fixed sleep or a wall-clock bound fails on a busy runner. Await the event that decides it (a promise, an exit or diagnostics_channel event, `until(fact, ms, what)` from test/support/processes/daemon-home.ts), step a clock (`vi.useFakeTimers`/`vi.setSystemTime`, or an injected `clock`/`now`/limit option such as `countedClock` in submodules.test.ts), or count the work (rows, bytes, reads)."
- **Already covered**: none. No rule in packs/*/rules or packs/rerail/rules, scripts/check, test/support or vitest.config.ts covers this. The `typescript` pack has only vitest-expect-expect, no-disabled-tests and no-focused-tests, and that pack is not enabled. The `python` pack has the ruff ASYNC sleep rules, which are Python only and also not enabled. `vitest.config.ts:44` only raises the timeouts on Windows (30 s per test, 60 s per hook).
- **False positives**: expected about 15–25%, judged from a 14-line random sample of sleep and race matches. Real cases:
  - a poll interval in a helper whose loop starts on an earlier line (test/support/processes/daemon-home.ts:22 `await delay(20);`);
  - a deliberately slow stand-in (test/api/mcp-bridge.test.ts:540 `setTimeout(resolve, 30_050)` inside the fake's callback);
  - absence checks with no event to await (test/runtime/daemon-control.test.ts:675 `await delay(100); expect(existsSync(stateFile)).toBe(false)`);
  - the negative windows from #964 (`setTimeout(resolve, 4_000, "no check within 4 s")`, test/daemon/daemon.test.ts:1330);
  - deliberate cost checks (integration/daemon/hook-write-lock.test.ts:200 `toBeLessThan(free.pre.ms + 2_000)`, test/engine/native/native-code-rule-dsl-scale.test.ts:99).

  Bounds that assert a product limit plus slack (test/daemon/daemon-tls.test.ts:461 `10_000 + 2_000 + 1_000`) are borderline.
- **Mode**: log, because main has 120 matches (90 of them JF's own) and the shape has legitimate uses (negative windows, deliberately slow stand-ins, cost checks). The rule can steer only after the negative-window and stand-in exceptions are in place and the remaining sleeps move to `until`, fake timers or injected limits.


<a id="rerail-no-silent-catch"></a>

### 3. `rerail-no-silent-catch`: product code never turns every error into a fallback answer with no narrowing and no log line

- **Pattern**: a TypeScript catch in `src/**` that has no binding (or never reads it) and is empty or only `return <literal>`, or `.catch(() => <literal>)`, `.then(ok, () => <literal>)` or `.on("error", () => undefined)`, so every error becomes the fallback value. Bugs, transient errors and the expected `ENOENT` all get the same answer, and nothing is written to the log. **Why it costs**: "removeQuietly swallows every error, not just Windows holding the file … retried at every pass with nothing in daemon.log" (#964, 4679548941). "copyKeysWaitingIn turns every error, bugs included, into 'keys wait'" (#676, 4598422292). Moti's review of #963 (review 6041654659#2) said of a silent fallback: "nothing tells the user".
- **Evidence**: the lexical shape has 18 bot findings (0 blockers, 9 follow-ups, 9 nits) in 16 PRs. 8 of JF's fix-up commits in 7 PRs narrow or log such a catch: 57a2959edd, a8929cb565, 89d2021095, fdfb21aefe, 95d12d69f5, 8020b27663, 4cb4821e37 and 160e8b1d68. There are 0 human findings in this shape. The wider family is 46 bot findings (2 blockers, 29 follow-ups, 15 nits) in 24 PRs, 2 human findings (jeffreysnell #672 6866504436; Moti #963) and 29 swallowed-error fix-ups in 22 PRs. The other 28 bot findings have no lexical shape: an OS return code used as the answer (`remote == 0`, `== WAIT_OBJECT_0 else "running"`), a catch that narrows too widely, a cached or sticky transient failure, or `?? true`. Only a reviewer can find those.

  | PR | Ref | Commit | File:line | Code / finding |
  |---|---|---|---|---|
  | 964 | 4679548941 (nit) | fix fdfb21aefe | src/engine/storage/untaken-events.ts:458 | `function removeQuietly … try { rmSync(path, { force: true }); } catch { // Windows holds it: the next pass tries again. }` |
  | 973 | 4652528038 (nit) | fix 89d2021095 | src/engine/code-rule/checkout-code-rule-router.ts:170 | `try { roots = this.#aheadRoots(sessionId); } catch { return; }` "Build-ahead failures are swallowed without a log line" |
  | 680 | 4573392870 (nit), gdgiangi 4590229858 | fix 8020b27663 | src/engine/audit/owner-lock.ts:58 | `lock = loadNativeBinding().tryLockExclusive(path); } catch { return NO_LOCK; }` "Falling back to no lock is silent" |
  | 658 | 4568492964 (follow-up) | fix 160e8b1d68 | src/harness/hook-failures.ts:390 | `readLogFile`: `descriptor = openSync(path, "r"); } catch { return null; }` "An unreadable `.1` counts as missing" |
  | 1076 | 4692688253 (follow-up) | fix a8929cb565 | src/engine/git-starter/git-starter.ts:180 | `stream.on("error", () => undefined);` "An output stream error … reads as a clean end" |
  | 920 | 4644197266 (follow-up) | intro 22cd0b429b, fix 65cc2091a9 (Moti, on the PR branch) | src/engine/storage/ledger-paths.ts:197 | `await lstat(records, { bigint: true }).catch(() => undefined)` "swallows every error" |

- **Today on main**: 353 matches in `src/**/*.ts` (tests and `.d.ts` excluded). The prototype is a Python scan of `git cat-file --batch` on origin/main. For each `catch\s*(\(\s*(\w+)?…\))?\s*\{`, it matches the body by brace-counting, which skips strings and comments. A body that is empty after comments are removed counts (47; one, `src/daemon/daemon.ts:1312`, is `catch {}` inside a template-literal child script, which an AST rule would not see). So does a body of `return( undefined|null|false|true|[]|{}|0|""|IDENT)?;` (206). A catch with a binding that reads it never matches. Three regexes find the promise and stream forms: `\.catch\(\s*(async\s*)?\(\s*\w*\s*\)\s*=>\s*(undefined|null|false|true|void 0|\{\s*\}|\[\]|0)\s*\)` (75), `\.then\([^;]*?,\s*\(\s*\)\s*=>\s*(…same…)\s*\)` (20) and `\.(on|once)\(\s*["']error["']\s*,\s*\(\s*\w*\s*\)\s*=>\s*(undefined|\{\s*\}|null|void 0)\s*\)` (5). `git blame origin/main` ran on all 353 lines: **177 are JF's (34 squash commits) and 176 are others'** (Timo, Jeff Snell, Gabriel Giangi and others). By the call in the try block: 122 are file probes (stat, access, realpath, readdir, readFile, open), 58 parse (JSON.parse, URL, Number), 33 remove, kill or close, 13 call Git and 119 are other. Examples: `src/api/change-stream.ts:20` (JF 221152067f) `catch { return undefined; }`, `src/cli/setup-cli.ts:170` (Timo 042d82ebb2) `realpath(…).catch(() => undefined)`, `src/daemon/daemon.ts:1361` (JF a08de26a99) `pipe.on("error", () => undefined);`. A wider clause, an unbound catch with any body but no log or throw, adds 48 more (19 JF).
- **Checkable**: TypeScript code rule (AST), reviewer only for Rust. **Shape**: TypeScript.
  - It flags a `catch_clause` with no parameter, or whose parameter is never referenced in its body, when the `statement_block` holds only comments or a single `return_statement` of a literal, `undefined`, an identifier or an arrow that returns a literal.
  - It also flags a `call_expression` on `.catch`, the second argument of `.then`, or `.on`/`.once("error", …)`, when the argument is a parameter-less arrow whose body is such a literal or `{}`.
  - Scope: include `src/**/*.ts`; exclude `**/*.test.ts`, `test/**`, `**/*.d.ts` and generated files.
  - Exception: a try block that only parses (JSON.parse, `new URL`, `Number`) with no IO in it.
  - Steer: "This catch turns every error into the fallback with no trace: narrow it to the errors you expect (`heldOnWindows(error)`, `(error as NodeJS.ErrnoException).code === "ENOENT"`, an engine-refusal class) and rethrow, or write a `rerail:` line with the error (its stack in daemon code), for the rest."
  - Rust is out of scope. Its 4 findings (#921 `worktree_stat.rs:519`, #975 `process.rs:409`, #1051 `pipe_server.rs:214`, #756 `git_start/run.rs`) map Win32 or libc return codes to answers, and none is `Err(_) => Ok(())`. On main, `Err(_) => …` (17), `let _ = <fallible>` (108) and `fs::…().ok()` (96) are mostly deliberate.
- **Already covered**: no. `javascript/eslint-no-empty` is not enabled. It covers only `*.js/jsx/mjs/cjs`, and it passes any block that holds a comment, so it would fail none of main's 46 real empty catches, which all carry a comment. `javascript/eslint-no-useless-catch` (not enabled) flags the opposite case, a catch that only rethrows. Nothing in `scripts/check/*`, the test guards or REVIEW.md states this convention.
- **Repo convention for deliberate catches**:
  - A one-line reason comment is in all 46 real empty catches and 15 of the 206 return-fallbacks. It is not a safe key: the flagged catches in #964 ("Windows holds it…") and #987 `recordHolder` ("Unrecorded, the holder holds the home all the same.") both had one.
  - `*Quietly` helpers are not a safe key either: `closeQuietly`, `removeQuietly` ×2, `readQuietly` and `rollbackQuietly` (5 on main), and `removeQuietly` was the #964 finding.
  - The fix shape is a safe key: the catch binds the error and reads it. That is `heldOnWindows` (`src/engine/file-replace.ts:60`, 8 uses), `isErrno` (`cursor-transcript.ts:48`), `isMissing` (`harness-registration.ts:182`), `errorCode` (`repository-identity.ts:755`), `refusedByEngine` or 38 inline `.code === "E…"` checks. The matcher already leaves these alone.
- **False positives**: many of the matches are fine.
  - Expected broken pipes, such as `src/engine/git.ts:1202` (Gabriel 55305840fc) `child.stdin.on("error", () => undefined);` with the comment "the broken pipe is no failure of the read".
  - Close or terminate during cleanup, such as `git.ts:335` `thread.terminate().catch(() => undefined)` and `change-stream.ts:143` "Already closed".
  - Existence probes, such as `rule-evidence.ts:276` `access(path).then(() => true, () => false)`.
  - Parsing of hand-edited records.

  A cleanup exception would also drop 2 of the 18 real findings (#964 and #987 were both `rmSync` catches). A parse exception without the no-IO condition would drop `home-daemon-lock.ts:105` `holderRecord`, part of #987. Expect most matches to be fine, or to need only an `ENOENT` narrowing.
- **Mode**: log. Main matches 353, and 295 after the parse exception, split evenly between JF and others, so it cannot steer. The steer text belongs at edit time on new catches. Keep it in Log until most of the 122 file-probe sites are narrowed, then revisit.


<a id="rerail-test-bounded-waits"></a>

### 4. `rerail-test-bounded-waits`: a test's wait on a program or server ends when that program exits or fails, not at the runner's timeout

- **Pattern**: test code waits on something that may never come: a listen wrapped as `new Promise(resolve => s.listen(…, resolve))` with no `error` handler, `await once(child.stdout, "data")` not raced against the child's exit, a `withResolvers` promise awaited alone, or a `spawnSync`/`execFileSync` of PowerShell or a script with no `timeout`. **Why it costs**: "spawnSync blocks the worker's event loop, so vitest's 60 s case timeout can't stop it … holds the job until the CI job timeout and never shows up as a test failure" (#1058, 4689199560); "the case hangs to vitest's timeout and fails with a bare "Test timed out". The child's collected `stderr` never shows" (#974, 4676583399).
- **Evidence**: 12 bot findings (0 B / 3 F / 9 N; 0 human) in 12 PRs; 16 fix-up commits in 15 PRs (10 themed test-no-timeout, 4 that add a program bound, plus 21bb046a3 and 719006237f). 17 PRs in all. The "64 fix diffs added a `timeout:` in 26 PRs" figure does not hold as a hang count: 56 fix-up commits in 26 PRs add a `timeout:` line in a test file (40 net-new, 24 PRs), but 24 of them set `vi.waitFor`/`expect.poll` budgets and 21 set per-case `{ timeout }`. Those are margins (test-real-time-margin, see `rerail-test-no-real-time`). Only 6 commits in 5 PRs bound a started program (6ab17a48b3, 721b735e8b, fd18fd0b90, cdbd2f8d33, 725c064b77, 19e315f52d).

  | PR | Ref | Commit | File:line | Code / finding |
  |---|---|---|---|---|
  | 1117 | 4708424985 (nit) | fix 6ab17a48b3 | test/scripts/check/test-selection.test.mjs:195 | `const run = spawnSync(process.execPath, ["scripts/check/ci/test-selection.mjs", list, "--github"], { cwd: ROOT, encoding: "utf8" });` |
  | 1058 | 4689199560 (follow-up) | fix 721b735e8b | test/runtime/daemon-control.test.ts:3281 | `const { timeout: _bound, ...unbounded } = options; const result = runSync(program, …, unbounded);` |
  | 650 | N2@da71fac2e (nit) | fix 135ddbe1c5 | integration/rerail-without-daemon.test.ts:68 | `await new Promise<void>(resolve => daemon.listen(0, "127.0.0.1", () => resolve()));` |
  | 677 | N2@e2124df8b (nit) | fix d214e34938 | test/runtime/engine-service/engine-proof.test.ts:63 | `const [line] = await once(child.stdout!, "data") as [Buffer];` |
  | 974 | 4676583399 (nit) | fix 1d4193c3bf | test/runtime/engine-service/engine-service-power-cut.test.ts:138 | `await openedFor(event.session.sessionId).promise;` (no timer, not raced against `exited`) |
  | 865 | 4634136830 (nit) | fix 719006237f | integration/claude-hook-rules.test.ts:201 | `void closed.then(([code]) => { failed(new Error(…)); });` (a spawn error leaves the wait for `held` unsettled) |

  It recurs after the fix: #650 fixed the listen shape on 2026-11-13. #964 (221152067f, merged 2026-11-24) then added it 5 more times: event-stream.test.ts:234, hook-failures.test.ts:545 and :580, hook-steer-receipt.test.ts:70, and test/support/hook-events.ts:93.
- **Today on main** (origin/main 989a882edd, `git grep -P` over `test/`, `integration/` and `**/*.test.*`; blame `-w` on every match, last author):
  - (a) listen with no reject: `new Promise(<[^>]*>)?\(\s*\(?\s*resolve\s*\)?\s*=>.*\.listen\(` has **25** matches, none with an `"error"` handler in the 8 lines before it. JF wrote 20 and others 5. Examples: claude-hook.test.ts:206, engine-service-socket.test.ts:94, and test/support/fake-sink.ts:70 (Jeff).
  - (b) unraced once: `\bonce\([\w.]+\.(stdout|stderr|stdin)\s*,\s*"(data|line|readable)"\)` has 10 matches. 4 have no `race`/`exited`/`reject` in the 4 lines before them or on the line: file-lock.test.ts:245, daemon-control.test.ts:586 and :608, and remove-folder.test.ts:37. JF wrote 2 and others 2.
  - (c) PowerShell start with no bound: `execFileSync`/`spawnSync` of PowerShell with no `timeout` key has 4 matches, all JF's: session-records-gitignore.test.ts:112, backup-privileges.ts:42, console-probe.ts:114, and a fixture string in check-owned-builtins.test.mjs:153.
  - (d) broad shape, rejected: any `spawnSync|execFileSync|execSync(` without `timeout` in its argument text has **499** of 528 calls, in 193 files. 339 of them start `git`. JF wrote 205 and others 294.
  - (e) `withResolvers` awaits: `await x.promise` outside a `race` has 237. Most are in-process gates the test opens itself.
  - (f) `vi.waitFor` (278 calls) cannot hang: Vitest's default timeout is 1000 ms. It is a margin question and out of scope here.
- **Checkable**: code rule (AST, TypeScript/JavaScript) for (a)+(b)+(c); (d) and (e) are for a reviewer only. **Shape**: scope `test/**`, `integration/**`, `**/*.test.{ts,mjs}`.
  - (a) `new_expression` `Promise` whose executor arrow has exactly one parameter and whose body calls `*.listen(`. Exception: the server has `.on|once("error", …)` in the same function.
  - (b) `await`/`.then` on `once(<member>.stdout|stderr, "data"|"line")` that is not an argument of `Promise.race`.
  - (c) `spawnSync`/`execFileSync`/`execSync` whose program matches `/powershell|pwsh/i` and whose options object has no `timeout` pair and no spread.
  - Steer: "Make this wait end when what it waits on fails: reject on the server's `error`, race the read against the child's `exit`, and give a synchronous PowerShell a `timeout` below the case's, so the case fails with the cause instead of hanging to the runner's limit."
- **Already covered**: none. packs/* hold no timeout or listen rule. vitest.config.ts:44 raises `testTimeout`/`hookTimeout` on win32 only, and cannot stop a synchronous child. scripts/check/check-owned-builtins.mjs covers `src/` only, and checks `windowsHide`/path, not time.
- **False positives**: (a) a listen on a path the test just made fresh (most of the 25) rarely fails, but the fix is mechanical: `(resolve, reject) => s.once("error", reject).listen(…)`. (b) mcp-bridge.test.ts:60 creates `once` and races it 3 lines later, which a sibling-statement window must allow. (c) the fixture string at check-owned-builtins.test.mjs:153 (a template literal) must be excluded.
- **Mode**: log, because main holds 29 matches for (a)+(b), 22 of them JF's. After a one-commit sweep of those 29 mechanical lines it can steer. Clause (c) can steer once its 3 real matches are bounded.


<a id="rerail-process-time-limit"></a>

### 5. `rerail-process-time-limit`: product code that starts an external program bounds it with a time limit, and the daemon's thread never waits synchronously on another process

- **Pattern**: `src/` starts icacls, PowerShell, cmd.exe, a VACUUM child or `xcode-select` with no `timeout` and no kill timer of its own. Or it waits synchronously on the daemon's thread, which answers every hook: a `BEGIN IMMEDIATE`/`.immediate()` under the connection's 5 s `busy_timeout`, a flush, or a sync child. **Why it costs**: "Hooks, `rerail` commands and an older daemon all wait at most their 5 s busy timeout … then fail with `database is locked`" (#648 blocker 4568519127). "`openSession` … flushes the waiting writes on the daemon's thread with `busy_timeout` up to their 5 s deadline … holds every hook up to 5 s once" (Moti, #964 4670090435).
- **Evidence**: 4 bot findings (1 B / 3 F / 0 N) and 6 human ones (Moti 5, gdgiangi 1; 3 of the 6 are fixes Moti pushed himself) in 7 PRs. 2 fix-up commits (aaeb5db4ae #963, 678f76e97e #812) and 2 later fixes by JF to his own merged code (221152067f, fixing #680's 2d142dec54; bf24c778f4, fixing #816's a978367603). 10 PRs in all.

  | PR | Ref | Commit | File:line | Code / finding |
  |---|---|---|---|---|
  | 648 | 4568519127/B1 (blocker) | intro 89fc97be79 | src/engine/storage/vacuum-process.ts:16 | `client.exec("VACUUM");`, a child with no limit, holding the write lock |
  | 920 | 4644197304 (follow-up) | intro fe08657639 | src/engine/storage/windows-acl.ts:139 | `await icaclsLater(windowsSystemProgram("icacls.exe"), ownerOnlyArguments(path, true), { encoding: "utf8" });` |
  | 963 | Moti review 6041654659#3 + bot 4674768439 | fix aaeb5db4ae | src/setup/harness-registration.ts:2032 (main) | `spawn(path, ["/d", "/c", "exit", "0"], { stdio: "ignore" })`; "cmd.exe that never exits holds daemon (and dashboard request) for 10 s" |
  | 680→964 | follow-up 221152067f | orig 2d142dec54 | src/engine/storage/database.ts:88 | `database.$client.exec("BEGIN IMMEDIATE");` on the daemon thread, under `busy_timeout = 5000` |
  | 816→905 | follow-up bf24c778f4 | orig a978367603 | src/runtime/merge-assist/service.ts:658,685 | `this.#keepTaken(waitMs > 0);` → `if (waitForLock) keep();` with `keep = () => this.#db.transaction(…, { behavior: "immediate" })`; "waits up to the connection's five-second busy timeout" before it stops hook commands |
  | 964 | Moti 4670090435 | — | src/runtime/runtime.ts:2406 | `openSession` → `#recorded()` flush on the daemon thread (not fixed in the PR) |

- **Today on main** (origin/main 989a882edd, scope `src/**` without tests, `src/engine/child-process.ts` left out; argument text read with paren matching; blame `-w`):
  - (A) a start with no limit: `(?<![\w.])(?:\w+\.)?(spawnSync|execFileSync|execSync|execFile)\(` plus `promisify(execFile)` aliases, with no `timeout` in the options. 6 raw matches; **4** after the exceptions. They are `xcode-select -p` at git.ts:225 (Timo, #222), and git.ts:256 and :294 (JF, 7ff56e15e1); and windows-acl.ts:252 (JF), the icacls of #920, which was kept unbounded on purpose and documented. The exceptions are git-start.ts:175 (`...limits` carries the timeout) and hook-launcher.ts:83 (it is the hook's own process, which the coding agent's hook timeout bounds). Async `spawn(` with no `timeout` key: 17 (JF 9, others 8). Most are long-lived children (the daemon, the CLI with `inherit`) or carry their own kill timer (harness-registration.ts:2032), so the option is the wrong test there.
  - (B) a sync wait on the daemon thread: `BEGIN IMMEDIATE|\.immediate\(\)` with no `busy_timeout = 0` in the same function has 3 matches. remote-cursors.ts:70 (JF beb0d5dbc5) runs inside `openSession`, the same shape as #964 F2. database.ts:376 (Timo) runs migrations at open. rule-provenance-move.ts:647 (JF) runs on a worker, so it is a false positive. `Atomics.wait(` in `src`: 5 (JF 4, Timo 1), all deliberate short pauses or worker hand-offs.
- **Checkable**: (A) is a code rule (AST, TypeScript). (B) is for a reviewer only: which thread runs a function is not in its file. The Rust half (#621, gix opens under `GIT_READ_TIME_LIMIT`) is reviewer-only too. **Shape (A)**: on `src/**/*.ts`, excluding `**/*.test.ts`, `src/engine/child-process.ts` and `src/harness/hook-launcher.ts`, match a `call_expression` whose function is `spawnSync|execFileSync|execSync|execFile` (bare or as a member), or a local bound to `promisify(execFile)`, whose options `object` has no `timeout` pair and no `spread_element`. Exception: a `// no time limit:` comment on the line above (windows-acl.ts:252). Steer: "Give this program a `timeout` (and read `killed`/`ETIMEDOUT` as a failure): a PowerShell, icacls or cmd.exe that never exits otherwise holds the session or the daemon's thread with it."
- **Already covered**: none. scripts/check/check-owned-builtins.mjs forces every start through src/engine/child-process.ts (`windowsHide`, absolute path) but checks no time limit. That wrapper is also where the limit could be required by type, a stronger fix than a rule. No pack rule names `timeout`.
- **False positives**: deliberate unbounded runs (windows-acl.ts:252, whose doc says "with no time limit"), and options passed by variable or spread (git-start.ts:175). `xcode-select -p` returns at once in practice, so 3 of the 4 main matches are low-risk, but they are cheap to bound.
- **Mode**: log, because main has 4 matches (3 of them JF's) and one is a documented choice. Steer once `xcode-select` is bounded and the icacls exception is marked. (B) goes in the reviewer prompt and REVIEW.md, not into a rule.


<a id="rerail-powershell-module-qualified"></a>

### 6. `rerail-powershell-module-qualified`: A cmdlet in PowerShell script text in `src/` is named with its module (`Microsoft.PowerShell.Management\Get-Process`, `CimCmdlets\Get-CimInstance`), never bare or by alias.

- **Pattern**: PowerShell script text built in a TS template string calls `Get-Process`, `Test-Path`, `Get-CimInstance` … bare. **Why it costs**: "Get-Process/Stop-Process/Wait-Process bare; must be Microsoft.PowerShell.Management\ qualified" (Moti-Velz, #975 4670103891); PowerShell then reads other installed modules first (slow start, shadowing), and under `$PSModuleAutoLoadingPreference = 'ModuleQualified'` (MODULE_QUALIFIED_ONLY) a bare cmdlet from an unloaded module fails.
- **Evidence**: 5 findings (0 bot + 5 human: Moti-Velz 3, gdgiangi-nera 1, jeffreysnell 1) in 3 PRs (#967 ×3, #975, #1006); 2 fix-up commits (08e398ae92 #975, fd18fd0b90 #1006). Two findings are about the test helper missing lowercase names and aliases (#967 review 6026293244#3, #1006 review 6029648890#F1-F2).

  | PR | Ref | Commit | File:line | Code / finding |
  |---|---|---|---|---|
  | 975 | 4670103891 | fix 08e398ae92 | src/engine/process-liveness.ts:192 | removed: `` const script = `$p = Get-Process -Id ${pid} -ErrorAction Stop; $null = $p.Handle; ` `` |
  | 967 | review 6027046585#F1 | not fixed (main e3f1c52cf5, Timo) | src/setup/harness-registration.ts:562 today | `` return `$LASTEXITCODE = 1; if (Test-Path -LiteralPath ${node}) { & ${node} ${hook} } … ` `` |
  | 967 | review 6027046585#N1 | not fixed (main c219472639, JF #677) | src/runtime/daemon-control.ts:2483 today | `` `powershell -NoProfile -Command "(Get-CimInstance Win32_Process -Filter 'ProcessId=${pid}').CommandLine"` `` |
  | 1006 | review 6029648890#F1-F2 | fix fd18fd0b90 | test/support/platform/powershell-commands.ts:16 | "cmdletsWithoutModule misses aliases before `{` and lazily-exported aliases; script reads as clean" |
  | 967 | review 6026293244#3 | — | test/support/platform/powershell-commands.ts | "only catches capitalised Verb-Noun; misses lowercase and aliases" |

- **Today on main**: `cmdletsWithoutModule` (test/support/platform/powershell-commands.ts:52) is applied to the spawned script in three suites only: test/activation/device-keystore.test.ts:80, test/engine/process-liveness.test.ts:184/211/260, test/engine/storage/windows-acl.test.ts:170 — i.e. src/activation/device-keystore.ts, src/engine/process-liveness.ts, src/engine/storage/windows-acl.ts, the only three src files that name `Microsoft.PowerShell`. Prototype over `src/**/*.ts` on origin/main: Verb-Noun `(?<![\w\\.-])(Get|Set|Test|New|Remove|Stop|Wait|Start|ConvertTo|Select|Where|Invoke|…)-[A-Z][A-Za-z]+\b` on non-comment lines with a quote: 10 bare cmdlets on 8 lines in 5 files, none in the three tested files. Blame: JF 7 (harness-registration.ts:387, 388, 575 from 53f5b92a1c #503; rerail-commands.ts:222 53f5b92a1c; daemon-control.ts:2483 c219472639), Timo 3 (setup-cli.ts:536 `Remove-Item`, harness-registration.ts:562 `Test-Path`, shell-path.ts:295 `Where-Object`).
- **Checkable**: code rule (regex, line; or AST on string/template fragments). **Shape**: TypeScript; match a Verb-Noun token from PowerShell's approved verbs, or a POWERSHELL_ALIASES word in command position, inside a string/template literal, not preceded by `\`; scope `src/**/*.ts`, exclude tests; exceptions: `String.raw` regexes that recognise commands older Rerail versions wrote (harness-registration.ts:387-388), prose shown to an agent as an example. Steer: "Name this cmdlet with its module (`Microsoft.PowerShell.Management\Test-Path`, `CimCmdlets\Get-CimInstance`), as process-liveness.ts does: a bare cmdlet makes PowerShell search other modules first and fails under ModuleQualified."
- **Already covered**: partly, by test: `cmdletsWithoutModule` on the three files above. Nothing covers harness-registration.ts (the hook commands written into Codex/Cursor settings), daemon-control.ts's hint, setup-cli.ts or shell-path.ts; no pack rule.
- **False positives**: the 3 lines of legacy-command regexes (harness-registration.ts:387-388) and agent-facing instruction text (rerail-commands.ts:222); Verb-Noun words in user-facing prose strings. Of 10 hits, 7 are real (both #967 findings still on main).
- **Mode**: log, because main has 10 hits (3 deliberate); steer once the 7 real ones are qualified and the legacy regexes carry an exception. An edit-time rule adds the files the tests do not reach.


<a id="rerail-row-parse-guard"></a>

### 7. `rerail-row-parse-guard`: a stored column or file line parsed per row is parsed inside the row's own guard, so one damaged row costs only itself

- **Pattern**: `JSON.parse` (or a Drizzle `mode: "json"` column / `.mapWith(jsonColumn)`, which parses for you) inside a `.map`/`.flatMap`/`for` over rows or lines, with no `try`/`safeParse` per row, so one damaged `*_json` value or ledger line throws out of the whole read. **Why it costs**: "`discoveryGapPayloadSchema.safeParse(JSON.parse(payloadJson))`: `JSON.parse` throws before `safeParse` can drop the row … the bridge then starts empty" (#922, 4642697109); "Alias lookup … still fails on a damaged aliases_json … Drizzle's JSON.parse throws" (#1113, F1@6068784475).
- **Evidence**: 7 findings (B0/F7/N0/human 0) in 5 PRs (#1113 ×3, #1103, #989, #922, #816); 6 JF fix-up commits (ddfcc69f63, 82090990af, b2fc725f0b, 0b2b87cd97, 970067fa3b, 05c8faadbb) plus 1 later main fix (7f01ce04f5, #989, of JF's own 383fc99aae from #530): 7 PRs in all.

  | PR | Ref | Commit | File:line | Code / finding |
  |---|---|---|---|---|
  | 922 | 4642697109 | fix b2fc725f0b | src/runtime/session-store.ts:342 | `const payload = discoveryGapPayloadSchema.safeParse(JSON.parse(payloadJson));` |
  | 1113 | F1@6068784475, F3/F4@6069987164 | fix 0b2b87cd97, 970067fa3b | src/engine/catalog/repository-registry.ts:54 | `…where(ne(repositories.aliases, [])).all().find(row => row.aliases.includes(key))` (json-mode column parsed per row) |
  | 1103 | 4701978635 | fix ddfcc69f63 | src/cli/support-bundle.ts:534 | `entry = JSON.parse(line) as typeof entry;` — one malformed delivery reports the whole ledger missing |
  | 989 | 4657869523 | fix 82090990af | src/engine/audit/audit-store.ts:198 | `const failures = read("failures") as AuditLists["failures"];` — `[null]` fails every page |
  | 530→989 | followup | orig 383fc99aae, fix 7f01ce04f5 | src/engine/audit/audit-store.ts:95 | `…audits.rule_provenance_json)`.mapWith(audits.ruleProvenance);` — one damaged audit failed every page |
  | 816 | 4627278688/F4 | none | src/runtime/merge-assist/conflict-hook.ts:95 | `const unheardSchema = z.array(z.object({ event: ..., conflict: storedConflictSchema }));` — whole list dropped on one entry |

- **Today on main**: AST scan (TypeScript compiler over `git archive origin/main src`: `JSON.parse` call whose nearest enclosing loop body or `.map/.flatMap/.filter/.forEach/.reduce/.find/.some/.every` callback has no `try` between it and the call) → 11 hits; 6 parse a stored column or line: audit-store.ts:699 and :836 (`JSON.parse(row.payload)`), remote-outbox.ts:664 (`scan.document_json`) and :666 (`scan.rules_json`), repository-start.ts:321 (`JSON.parse(ruleIds) as string[]` in `flatMap`), runtime.ts:1051 (ledger `line`). Blame (all 6): JF 0, Timo 3, Jeff Snell 3. Missed syntactically: support-bundle.ts:465 (JF, per-row mapper in a `Map`), claude-status-store.ts:99 (per settings file, no try). Plus 20 Drizzle `mode: "json"` columns in schema.ts that parse on every select. Guarded helpers already on main: `parsedList` (audit-store.ts:168), `parsedRow` (session-store.ts:370), `readState` (merge-assist/state.ts:30, try + `safeParse`).
- **Checkable**: code rule (AST, relational: call inside loop/iterator callback, no `try` ancestor inside it). **Shape**: TypeScript; match `call_expression` `JSON.parse` with an `arrow_function`/`function_expression` argument of a `.map|.flatMap|.filter|.forEach|.reduce|.find` call, or a `for_in_statement`/`for_statement` body, as nearest ancestor, and no `try_statement` between; scope `src/**/*.ts`; exclude tests/scripts; exceptions: argument is a string literal or `JSON.stringify(...)`, or the call is inside `schema.safeParse(...)`'s own try helper. Steer: "Parse each stored row or line on its own (`parsedRow`, `parsedList`, `readState`, or try/`safeParse` per item) and skip and log the damaged one, so one bad row does not fail the whole read."
- **Already covered**: none (no pack rule or scripts/check guard on JSON.parse placement).
- **False positives**: ruff-config.ts:294 (`JSON.parse(part) as string`, literal decode), engine-service.ts:678 (lane key this process built), code-rule-enforcer.ts:1869/2244 and rule-draft.ts:350 (native result pages, where a bad page should fail the whole run). The Drizzle json columns and helper-indirected parses (support-bundle.ts:465, #1113's `.find(row => row.aliases…)`) are false negatives: the commonest shape in JF's findings is invisible to a syntactic rule.
- **Mode**: log, because 6 real matches stay on main (none by JF) and the rule misses JF's own commonest form (Drizzle json columns); the deciding question — "is a whole-read failure acceptable here?" — needs a reviewer.


<a id="rerail-comment-width"></a>

### 8. `rerail-comment-width`: A comment or doc line you write or edit fits the file's 120-column wrap; rewrap the paragraph instead of leaving one long line.

- **Pattern**: an agent edits a sentence inside a wrapped doc comment (or Markdown paragraph) in place and leaves one line at 140-230 columns, while the rest of the file wraps at 120. **Why it costs**: "Paragraph line ~230 chars where rest of file wraps at 120" (gdgiangi-nera, #963 4669467783); "mayStillStart doc comment line left unwrapped at about 190 columns" (bot nit, #884 4636623934). Each one is a review round and a fix-up commit.
- **Evidence**: 6 findings (0 B / 0 F / 5 N bot + 1 human) in 6 PRs; 5 fix-up commits (7 PRs in total). Not counted: #916 missing blank line and the #657 doubled `/**` (other shapes).

  | PR | Ref | Commit | File:line | Code / finding |
  |---|---|---|---|---|
  | 884 | 4636623934 | fix 126e0fa6ea | src/runtime/daemon-control.ts:1369 | removed line 184 cols: ` * the start leaves it running, and \`rerail status\` shows it running once it answers. It names no second start…` |
  | 648 | 4572331929 | fix e111a35b12 | src/engine/storage/rule-provenance-move.ts:201 | removed line 191 cols: ` * another connection (\`whenFree\`): how many it moved, and whether none is left. The first batch takes…` (plus 16 lines at 121-123) |
  | 974 | 4688212126 | fix dfba31e4c0 | src/daemon/history-sync.ts:83, test/daemon/history-sync.test.ts:3 | removed line 189 cols: `// what came before the sync started, and after it an idle daemon does no disk work; …` |
  | 881 | 4635096269 | fix ff59b15b8d | docs/threat-model.md:589 | removed line 139 cols: `first, since the ledger keeps only the hash and size. A finding in a secret-like file…` |
  | 921 | (CI/self) | fix ce1f587800 | .github/workflows/ci.yml:1322 | removed line 152 cols: `# pass). The \`.rerail\` library's canonical path is joined with \`/\` by \`bootstrap.rerail\`…` |
  | 963 | 4669467783 (human) | none | docs/how-setup-works.md:400 | "Paragraph line ~230 chars where rest of file wraps at 120" |

- **Today on main**: the convention is 120 (README/HISTORY, CONTRIBUTING and the fix-ups all rewrap to ≤120; no rustfmt.toml/.editorconfig/prettier states it; rustfmt's default `max_width` 100 is for code and does not wrap comments). Comment-line lengths in TS peak at 117-119 but spill softly past 120 (121: 1275 lines, 122: 608, 123: 344), so 120 itself is not a usable threshold: 3,606 TS + 547 Rust comment lines are >120. The flagged lines were all ≥139, so >130 separates them. Script over `git cat-file --batch origin/main:<file>` (vendor/fixtures/upstream/generated/forest excluded), comment line = `^\s*(//|/\*|\*)`, single-token (URL) bodies skipped: **>130**: TS/JS 572 lines/234 files, Rust 156/109; **>140**: TS/JS 226/121, Rust 96/76. Markdown: only files that wrap at 120 (p95 of prose lines ≤125, ≥5 lines at 101-125: 360 of 679 files; fences, tables, HTML, link refs skipped): 237 lines >130, 154 >140; the other files (docs/final-shape/notes, docs/rule-engine-direction, packs/*/README.md, CHANGELOG.md) write one paragraph per line. Blame of every >140 line: TS 36 JF / 190 others (Timo 122, Jeff Snell 30, Gabriel Giangi 30), Rust 0/96 (all Timo), MD 7/147. JF's are mostly one-line `/** … */` docs, e.g. src/engine/code-rule/code-rule-bridge.ts:2155 (156 cols, 622a9b11a4), src/engine/git-guard/submodule-state.ts:637 (154, 9de4fcc419).
- **Checkable**: code rule (regex, line). **Shape**: TS/JS `^\s*(?:\/\/|\/\*|\*)(?=.{128,}$)(?=.*\S\s+\S)` (line >130 with a space in it); Rust `^\s*//(?=.{128,}$)`; scope `src/**`, `test/**`, `integration/**`, `scripts/**`, `native/src/**/*.rs`, `native/crates/**/*.rs`; exclude `**/vendor/**`, `**/fixtures/**`, `packs/*/upstream/**`, `forest/**`, generated. Markdown only with an explicit list of 120-wrapped docs (`docs/*.md` minus `docs/final-shape/**`, `docs/rule-engine-direction/**`, `CHANGELOG.md`, `packs/*/README.md`) and fences/tables skipped, which a line regex can't see: leave MD to a later relational rule. Steer: "This comment line runs past the 120 columns the file wraps at: rewrap the whole paragraph to 120, wording unchanged."
- **Already covered**: none (no rule in packs/*/rules or scripts/check on line width; check-rustfmt does not wrap comments).
- **False positives**: lines over 130 that are deliberately long: a long code span or path inside a comment (`* {@link …}` chains), tables in comments, eslint/ts directives; the 226+96 existing lines on main would all fire if the rule judged the whole file rather than the edited lines.
- **Mode**: log, because main holds ~320 TS/Rust lines >140 (~730 >130), 85% by others; steer only on lines the edit adds, after the existing ones are rewrapped or baselined.


<a id="rerail-parse-unknown"></a>

### 9. `rerail-parse-unknown`: data read from a file, a child, a socket or a stored column is typed `unknown` and narrowed, never cast to the type it should have

- **Pattern**: `JSON.parse(x) as T` / `as Partial<T>` / `read(...) as T[]` on parsed data, and a parsed number checked only with `typeof v === "number"`. **Why it costs**: "`readLists` only checks `Array.isArray` and then casts (`read("failures") as AuditLists["failures"]`) … So `[null]` throws a TypeError for every audit on the page" (#989, 4657869523); "`startedAt: value => typeof value === "number"`. `JSON.parse` reads `1e400` as `Infinity`… the stop and a hook's start terminate whatever signalable process holds the recorded pid" (#975, F4@6009215324).
- **Evidence**: 5 findings (B0/F2/N3/human 0) in 5 PRs; 5 JF fix-up commits. (#1051 `as PipeServer`, #1058 `as unknown as` are casts of non-parsed values: not counted.)

  | PR | Ref | Commit | File:line | Code / finding |
  |---|---|---|---|---|
  | 989 | F4@6013905906 | fix 8bb050c10f | src/engine/audit/audit-control.ts:44 | `(failures as AuditFailure[]).every(item => isScanLimitMarker(item) …)` throws on a failure with no string `message` |
  | 975 | F4@6009215324 | fix adbdf59f77 | src/runtime/daemon-control.ts:438 | `startedAt: value => typeof value === "number",` → `isPositiveFinite` |
  | 987 | 4657838785 (nit) | fix bf2a2c41c1 | src/engine/native/home-daemon-lock.ts:84 | `logOffset: typeof logOffset === "number" ? logOffset : 0` → `Number.isSafeInteger` |
  | 1103 | 4703074512 (nit) | fix b459c77a8f | src/cli/support-bundle.ts:534 | `entry = JSON.parse(line) as typeof entry;` (steerDelivery promises strings it never checks) |
  | 658 | 4568492991 (nit) | fix 0b256407a4 | src/harness/hook-failures.ts:623 | `JSON.parse(line) as Partial<HookFailure & PartStart>` → `Partial<Record<keyof …, unknown>>` |

- **Today on main** (src/, ui/): 129 `JSON.parse` calls; 19 go straight into a zod `.parse/.safeParse`, 43 are cast to an unknown-shaped type (`unknown`, `Record<string, unknown>`, `{ pid?: unknown }`, `Partial<Record<K, unknown>>`). **15 cast to a trusted type**, pattern `git grep -P 'JSON\.parse\(.*\)\)?\s*as\s+' origin/main -- 'src/**/*.ts' 'ui/**'`, then dropping targets `unknown|Json|JsonObject|Record<string, unknown>|Partial<Record<…, unknown>>|{ all fields unknown }` and the 6 FPs below. Blame of all 15: JF 3 (support-bundle.ts:465 `JSON.parse(ruleIds) as string[]`, rule-configuration.ts:227 `as Tuple[]`, hook-adapter.ts:243 `as Partial<LockOwner>`), others 12 (Timo 6: scrub-cli.ts:79-80 `as CodeRuleViolation[]`/`as AuditFailure[]`, audit-store.ts:836, rr-child.ts:96, rr-child-process.ts:113, session-store.ts:894; Jeff Snell 4: remote-outbox.ts:666/872, repository-start.ts:321, remote-config.ts:92 `as Partial<RemoteConfig>`; Gabriel 2: rule-draft.ts:1312, support-bundle.ts:502). `as unknown as` in src: 40, none on parsed data (zod `ZodType` plumbing, regex tuples) → leave out. Loose numbers: 24 `typeof … === "number"` lines without `Number.is*`; ~9 check parsed data (JF 6, e.g. file-lock.ts:131 pid, daemon-control.ts:1093, process-liveness.ts:253, hook-failures.ts:352/364).
- **Checkable**: code rule (AST) for the cast; loose-number check reviewer only. **Shape**: TypeScript; match `as_expression` whose expression is (parenthesized) `call_expression` `JSON.parse(...)` and whose type is not `unknown`/`Json*`/`Record<string, unknown>`, not an object type whose every property is `unknown`, and not `Partial<Record<…, unknown>>`; also `as Partial<T>` on a `JSON.parse`. Scope `src/**/*.ts`, `ui/**/*.{js,ts}`; exclude `test/**`, `integration/**`, `e2e/**`, `scripts/**`, `benchmark/**`, `conformance/**`. Exceptions: argument is `JSON.stringify(...)` (deep clone) or a string/template literal; target `string` with a quoted-literal argument. Steer: "Type parsed data as `unknown` and read it with a zod schema (`schema.safeParse`, as `readState` in src/runtime/merge-assist/state.ts does) or a type guard (`isSteerRule`, `isClaudeStatusLine`), and check numbers with `Number.isFinite`/`Number.isSafeInteger`, not `typeof`."
- **Already covered**: none. typescript pack's `typescript-eslint-consistent-type-assertions` only forbids angle-bracket assertions (pack not enabled); no `no-unsafe-*` type-aware rule; no scripts/check guard.
- **False positives**: string-escape decodes (neon-format.ts:141, ruff-config.ts:294, rule-pack-matcher.ts:221, json-duplicate-keys.ts:131 `as string`), engine-service.ts:678 (key this process built with `JSON.stringify`), runtime.ts:537 (`JSON.parse(JSON.stringify(value)) as T`) — all excluded by the exceptions. Remaining debatable: own package.json (remote-outbox.ts:872), `json_group_array` over own columns (rule-configuration.ts:227, has a comment), in-process native binding output (rule-draft.ts:1312). Drizzle `text(..., { mode: "json" }).$type<T>()` (20 columns in schema.ts) is the same cast, invisible to this matcher.
- **Mode**: log, because 15 matches remain on main (12 by others), several reading trusted own output; steer only after they are converted or annotated.


<a id="rerail-test-exact-assertions"></a>

### 10. `rerail-test-exact-assertions`: a Vitest case does not exit early with a pass, and does not accept any `Error` or any truthy value

- **Pattern**: an `it`/`test` body that `return`s on a condition (it reports a pass instead of a skip), `toBeInstanceOf(Error)` with no name or message check after it, and `toBeTruthy()`. **Why it costs**: "`if (Number(major) * 100 + Number(minor) < 242) return;` reports a pass. **Fix:** Use `it.skipIf(...)` or `ctx.skip()`" (#621 N1, 4619143613); "`else expect(start).toBeInstanceOf(Error);` … any thrown error, a TypeError included, would pass" (#673 N1, 4568472535); "`expect(root).toBeTruthy()` can't fail" (#997, Moti-Velz 4670264198).
- **Evidence**: the kept shapes have 3 findings (0/0/2 bot nits, 1 human) in 3 PRs, and 3 JF fix-up commits. The whole theme is larger: test-vacuous plus new-test-unclear/opaque-failure has 41 findings (34 bot, 5 human, 7 follow-ups), and 56 fix-up commits in 35 PRs. Only 9 of those findings name a matcher or shape. The other 32 are about a test that never reaches the code under test (stale module copy, identical variants, a simulation that does not refuse), and only a reviewer can judge them.

  | PR | Ref | Commit | File:line | Code / finding |
  |---|---|---|---|---|
  | 621 | bot N1 4619143613 | 68d04b8e97 (in), 4d217db31e (fix) | test/engine/native/git-access.test.ts:67 | `if (Number(major) * 100 + Number(minor) < 242) return;` |
  | 673 | bot N1 4568472535 | 352668892c (fix) | test/hosts/claude/claude-hook.test.ts:149 | `else expect(start).toBeInstanceOf(Error);` → `toMatchObject({ name: "DaemonUnreachableError", … })` |
  | 997 | human 4670264198 | ab097e9b20 (in), 08d929e10b (fix) | test/engine/catalog/checkouts/checkout-placement.test.ts:146 | `expect(root).toBeTruthy();` |
  | 1021 | bot N2 4679216391 (dropped shape) | 7907b3021f (fix) | test/engine/git.test.ts:562 | `expect(endProcessTree).toHaveBeenCalledOnce();` |
  | 865 | bot N1 4636187758 (dropped shape) | ed08306b57 (fix) | test/engine/code-rule/code-rule-bridge.test.ts:1689,1713 | `expect(answered).toHaveBeenCalledTimes(3);` |
  | 1058 | bot N1 4689199577 (dropped shape) | c885e89a6c (fix) | test/runtime/daemon-control.test.ts:3325 | `expect(boundMs).toBeLessThanOrEqual(10_000);` |

- **Per matcher: findings that named it vs lines on main** (tests are `test/**`, `integration/**` and `*.test.ts`, read from `git archive origin/main`; the author of each line is from `git blame -w` on every matched line):

  | Shape | Findings | Fix-ups | Main (JF/others) | Verdict |
  |---|---|---|---|---|
  | `if (…) return;` directly in an `it`/`test` body | 1 nit | 1 | 3 → 2 after the exception (1 JF / 1 others) | keep |
  | `toBeInstanceOf(Error\|TypeError)` | 1 nit | 1 | 5 → 1 with no name/message check in the next 2 lines (Timo) | keep |
  | `toBeTruthy()` | 1 human | 1 | 1 (Timo) | keep |
  | `toHaveBeenCalledOnce()` | 1 nit | 1 | 16; 14 with no argument check within ±12 lines (14 others) | drop, 1/14 |
  | `toHaveBeenCalledTimes(n>0)` | 1 nit | 1 | 124; 81 with no argument check (50 JF / 31 others) | drop, 1/81 |
  | `toBeLessThanOrEqual` | 1 nit | 2 (c885e89a6c, 02cb99e3af) | 48 (35 JF / 13) | drop, 1/48 |
  | `toHaveBeenCalled()` (not `.not.`) | 0 | 0 | 6 (2 JF / 4) | drop, no finding |
  | `toBeDefined()` | 0 | 0 | 77 (47 JF / 30) | drop |
  | `toThrow()` / `rejects.toThrow()` with no argument | 0 (#865 quoted it for cleanup) | 0 | 55 / 21 | drop |
  | `toContain` on a whole output | 0 matcher-named (f5f85a53d0 tightened one) | 1 | 1309 | drop |
  | Rust `assert!(matches!(x, Err(E::Failed(_))))` | 1 (#1048 4687563683) | 0 | 18 | drop |
  | `stdio: "ignore"` in tests (opaque failure) | 2 follow-ups (17245df6dd, f634d61f3b) | 2 | 108 | drop |

- **Today on main**: 4 matches. Early return: `test/cli/support-bundle.test.ts:366` `if (label.length < 4) return;` (Gabriel Giangi, 4ea997179f) and `test/setup/harness-registration.test.ts:1835` `if (run.error !== undefined) return;` (JF #503, 53f5b92a1c). Bare instance check: `test/runtime/merge-assist/service.test.ts:821` `expect(await scanning).toBeInstanceOf(Error);` (Timo). Truthy: `test/support/host-adapter-contract.ts:48` `expect(profile.host).toBeTruthy();` (Timo). The patterns used: the line regex `^\s*if \(.*\) return;\s*$`, where the nearest less-indented line opens `it(`/`test(`; `\.toBeInstanceOf\((?:Error|TypeError)\)`; `\.toBeTruthy\(\)`.
- **Checkable**: code rule (AST, TypeScript). **Shape**: (a) an `if_statement` whose consequence is a bare `return` (or `return undefined`) and whose parent is the body of the function passed to a `call_expression` with callee `it`/`test`/`it.*`/`test.*`; (b) `.toBeInstanceOf(Error|TypeError)` where none of the next 2 statements reads `.message`, `.name`, `String(x)` or `toMatchObject` on the same subject; (c) `.toBeTruthy()`. The scope is `test/**/*.ts`, `integration/**/*.ts` and `**/*.test.ts`. The exception for (a): the `if` narrows a value that an `expect` on the line just before has already pinned (`expect(validation).toMatchObject({ ok: false }); if (validation.ok) return;`, test/engine/rules.test.ts:43). Steer: "Pin the value: skip with `ctx.skip()`/`it.skipIf` instead of returning a pass, and assert the error's name or message instead of any `Error`, and the exact value instead of `toBeTruthy()`."
- **Already covered**: none. The typescript pack has only `vitest-expect-expect`, `vitest-no-disabled-tests` and `vitest-no-focused-tests`, and it is not enabled here (only `complexity` and `packs/rerail` are). No pack has a `prefer-called-with`, `require-to-throw-message` or valid-skip rule. ruff PT011 ("raises too broad") is the Python counterpart, in the python pack.
- **False positives**: (a) type-narrowing returns after an assertion (1 on main, handled by the exception). A return after a helper that reports its own skip is possible but not seen. (b) a message check that comes more than 2 statements later, or that goes through a helper. (c) none seen.
- **Mode**: log, because main has 4 true matches (2 early returns, 1 bare `Error`, 1 `toBeTruthy`). Once those 4 lines are fixed, the rule can steer: it is a cheap fix with no false positives seen. The evidence is thin (3 findings, all nits or a human nit), so this is a small rule. The rest of test-vacuous stays with the reviewer.


<a id="rerail-test-cleanup-in-finally"></a>

### 11. `rerail-test-cleanup-in-finally`: a test undoes what it set up however it ends, and a clean-up never asserts

- **Pattern**: a test sets up a fake clock, env or global stub, spy, child process, daemon, socket or temp folder before its `try` (or never registers it), restores it as a plain statement that a failure skips, or asserts (`expect`, `expect.poll`, `vi.waitFor`) in the `finally` so the clean-up's error replaces the test's. **Why it costs**: "If anything between them throws, the faked `Date`/`performance` or the `setTimeout` spy stays active for the rest of the file" (#1118 N2, 4707349082); "If `fixture()` rejects, every later test in the file runs against the main store path, and their failures point away from the cause" (#1095 F2, 4698640859); "the error from the `finally` replaces the original assertion error in the report" (#652 N1, 4572276896).
- **Evidence**: 15 bot findings (0 B / 4 F / 11 N; 2 of them the finally-masks twin, plus the masking half of #1095 F1) in 10 PRs (652, 742, 865, 884, 935, 1051, 1095, 1104, 1117, 1118); 0 human findings (gdgiangi-nera fixed #742 N2 himself, 26be8b3da); 12 JF fix-up commits; 4 later main commits fixing JF's merged tests (04ad4184ea, 322b64de7c, 70892dfacf, b7c6398307; JF code from #503, #920, #977). Only 3 of the 15 are about vi state (#1118 N2, #1095 F2, #1104 N3); the other 12 are processes, daemons, sockets, bridges and folders.

  | PR | Ref | Commit | File:line | Code / finding |
  |---|---|---|---|---|
  | 1118 | N2 4707349082 | fix 8dd354c710 | test/engine/code-rule/worktree-walks/worktree-walks.test.ts:1423,1437 (at 8dd354c710^) | `vi.useFakeTimers({ toFake: ["Date", "performance"] });` before the `try`; `timed.mockRestore();` inside it, not in `finally` |
  | 1095 | F2 4698640859 | fix d78c55d048 | test/engine/code-rule/code-rule-enforcer-mechanical.test.ts:619-623 (at d78c55d048^) | `vi.stubEnv("HOME", home); … const { enforcer } = await fixture({ audits: recorder }); try {` — `vi.unstubAllEnvs()` only in that `finally` |
  | 1104 | N3 4702941169 | fix 96604d1e86 | test/engine/native/git-ignore-answers.test.ts:29-30 (at 96604d1e86^) | afterEach: `for (const cleanup of cleanups.splice(0).reverse()) await cleanup();` then `vi.useRealTimers();` — a rejected clean-up skips the restore |
  | 652 | N1 4572276896 | added 2d0b58a8af, fix 294dd84034 | test/runtime/daemon-control.test.ts:324 | `finally { … await vi.waitFor(() => expect(runningIn(home)).toEqual([]), { timeout: 5_000, interval: 10 });` (added as the fix of N4) |
  | 865 | N2 4634136846 | fix 719006237f | integration/claude-hook-rules.test.ts:206-207,328 (at 7190062^) | `release()` = `expect((await closed)[0]).toBe(0); expect(said).toContain("released\n");`, called from `finally { await release(); }` |
  | 977→986 | follow-up | 1609ac3a30 → fix 04ad4184ea | test/engine/catalog/outside-git.test.ts:449 | `} finally { writer.kill(); }` — exit not awaited, afterEach's folder removal refused |

- **Today on main** (AST prototype over 684 tracked `test/**`, `integration/**` .ts/.mts/.js/.mjs files; `/tmp/jf-mining/extra/cleanup/scan.cjs`; author by `git blame origin/main` on every hit): 1,059 `vi.useFakeTimers|stubEnv|stubGlobal|spyOn` calls; 602 sit in test bodies of files whose `afterEach`/`afterAll`/`onTestFinished` restores that kind (the per-file version of the config below, mostly JF's). Shapes: (a) setup in an `it` body, a statement that awaits or calls between it and the `try` whose `finally` restores it: **34** (17 JF, 17 Timo), e.g. test/runtime/audit-control-polling.test.ts:176 `vi.useFakeTimers(...)` then `await idleRuntime([])` then `try`; (b) restored only by a plain statement, no `finally`/hook: **41** setups (36 JF), e.g. test/engine/code-rule/code-rule-runtime-lifecycle.test.ts:1413 `vi.useRealTimers();`, test/runtime/merge-assist/service-stop.test.ts:804 `stderr.mockRestore();`; (c) never restored, no hook: **47** (9 JF, 20 Timo, 11 Gabriel, 7 Jeff), e.g. test/cli/cli-dispatch.test.ts:68 `vi.stubEnv(\`RERAIL_EXPERIMENTAL_${feature}\`, "0")`; (d) assertion statement directly in a `finally`: **3** (1 JF git.test.ts:368 `await vi.waitFor(...)`; Timo conflict-delivery.test.ts:396; Gabriel runtime-notes.test.ts:70 `expect(...)` before `await runtime.dispose()` — added by follow-up 322b64de7c); (e) `x.kill()` in a `finally` with no await after it: **6** (3 JF), e.g. test/engine/process-liveness.test.ts:244.
- **Config alternative** (instead of a rule for a–c): vitest.config.ts sets none of `restoreMocks`, `unstubEnvs`, `unstubGlobals` (`clearMocks` defaults on); setup-rerail-home.ts has only an `afterAll` (stranded daemons on POSIX, real agent settings, its own env/folders). Vitest 5 (main pins 5.1.0; read in 5.0.0's runner) runs `restoreAllMocks`/`unstubAllEnvs`/`unstubAllGlobals` in `onBeforeTryTask`, before `beforeEach`, so `restoreMocks: true, unstubEnvs: true, unstubGlobals: true` plus `afterEach(() => vi.useRealTimers())` in setup-rerail-home.ts make all 122 sites of a–c harmless. Breaks only test/engine/git-moved.test.ts:83 (`vi.stubEnv("PATH", …)` in `beforeAll`; move to `beforeEach`); 0 module-level spies or mock overrides. It would have prevented #1118 N2 and #1095 F2 and the leak half of #1104 N3 (not its `vi.mock` fs automocks, not the order: setup-file afterEach runs after the file's); fix-ups 8dd354c710, d78c55d048 (env half), 96604d1e86 (timer half). It prevents none of the 12 process/daemon/socket/folder findings or the masking twin.
- **Checkable**: a–c → config, not a rule. (d) code rule (AST, TS/JS). **Shape**: `call_expression` whose callee root is `expect` (incl. `expect.poll`), `assert*`, or `vi.waitFor`/`vi.waitUntil`, as an expression statement (or awaited one) directly inside a `finally_clause`, stopBy function boundaries; scope `test/**`, `integration/**`; exception: the call's promise has `.catch(` chained. Steer: "An assertion in `finally` replaces the test's own failure; check after the `try`, or wait in `onTestFinished`/afterEach and rethrow only when the body passed." (e) regex/AST possible (`finally` with `.kill(` and no later `await`) but 6 hits with benign cases → reviewer. Unregistered daemons/bridges/holders, helpers that leak on handshake failure: reviewer only.
- **Already covered**: none for the shape. `eslint-no-unsafe-finally` (packs/javascript, not enabled) flags only return/throw/break in `finally`. Partial runtime nets: setup-rerail-home.ts afterAll (stranded daemons, POSIX), `check:folder-removal` + removeFolder (names a process still in the folder), run-folder.ts globalSetup (temp folders die with the run, since #638).
- **False positives**: (d) 3/3 on main are real maskers. Missed: assertions inside a helper called from `finally` (#865 `release()`). For a–c the prototype also counts restores that the setup's own block does before any throwing statement (setupJustBeforeTry, 90 sites, excluded above).
- **Mode**: config change for a–c (no rule); (d) `log`, because main has 3 hits — fix them (3 lines) and switch to `steer`.


<a id="rerail-test-home-and-folders"></a>

### 12. `rerail-test-home-and-folders`: a test moves the user home with `stubUserHome`, never `vi.stubEnv("HOME"|"USERPROFILE")` or `process.env.HOME =`

- **Pattern**: a test stubs `HOME` (often without `USERPROFILE`) by hand instead of `stubUserHome` (test/support/user-home.ts), which moves both. **Why it costs**: "CONTRIBUTING asks a test that moves the home to call `stubUserHome` … This stubs both variables by hand, which works, but the helper keeps it in line if the helper ever changes" (Moti-Velz, #975 N2, 4670103938); CONTRIBUTING.md:390-391: "never `vi.stubEnv("HOME")` alone: on Windows that leaves `os.homedir()` on your real" profile, where a test that registers hooks rewrites the person's `~/.claude/settings.json` (user-home.ts:44).
- **Evidence**: 1 human finding (0 bot) in 1 PR (#975); 2 JF fix-up commits (01200fcc99 in #975; 6df73457ab in #503's follow-up branches, 6 `vi.stubEnv("HOME", …)` lines in 5 files). Also seen, not flagged: #1095 F2's code hand-stubs both (`vi.stubEnv("HOME", home); vi.stubEnv("USERPROFILE", home);`, still on main). The folder half (`rmSync(…, { recursive })` → `removeFolder`) had 4 JF fix-ups in #503 (2a23ec4791 added the check, c5975516d3, 1bc95dbd94, 6f5b0b55d9) and is now enforced, so it is not part of this candidate.

  | PR | Ref | Commit | File:line | Code / finding |
  |---|---|---|---|---|
  | 975 | N2 4670103938 (Moti-Velz) | fix 01200fcc99 | test/cli/daemon/older-daemon.test.ts:45 | `for (const name of ["RERAIL_HOME", "HOME", "USERPROFILE"]) vi.stubEnv(name, root);` → `stubUserHome(root);` |
  | 503 | self | fix 6df73457ab | test/harness/setup-picker.test.ts, setup-cli.test.ts, api-http.test.ts, repository-setup-*.test.ts | 6 `vi.stubEnv("HOME", …)` lines replaced: "a moved user home moves USERPROFILE too, which Windows reads" |
  | 1095 | (F2 context) | 3d1646e34f | test/engine/code-rule/code-rule-enforcer-mechanical.test.ts:623-624 (main) | `vi.stubEnv("HOME", home); vi.stubEnv("USERPROFILE", home);` |
  | (main) | — | 53f5b92a1c | test/harness/setup-picker.test.ts:261 (main) | `vi.stubEnv("HOME", join(setup.root, "user"));` — HOME alone, same file 6df73457ab fixed |

- **Today on main**: 13 matches of `vi\.stubEnv\(\s*["'](HOME|USERPROFILE)["']` or `process\.env(\.|\[["'])(HOME|USERPROFILE)\b.*=[^=]` in `test/**`, `integration/**` outside test/support/user-home.ts and setup-rerail-home.ts (AST scan + blame of every hit): 7 JF (kept-remote.test.ts:165, code-rule-enforcer-mechanical.test.ts:623-624, git-ignore-matchers.test.ts:38, main-instance.test.ts:138-139, setup-picker.test.ts:261), 5 Timo (ignore-rule-files.test.ts:13, git.test.ts:749, main-instance.test.ts:121-122, follow-gitignore.test.ts:49), 1 Jeff (setup-json.test.ts:285). 7 stub HOME alone: 5 for Git's global config next to `XDG_CONFIG_HOME`/`GIT_CONFIG_GLOBAL`, and 2 set-up tests (setup-json.test.ts:285, setup-picker.test.ts:261), which are the case CONTRIBUTING warns about: on Windows set-up then reads the real profile. 0 `process.env.HOME =` outside the setup file. A dynamic name (`for (const name of […]) vi.stubEnv(name, …)`, the #975 shape) is not matched.
- **Checkable**: code rule (regex or AST, TS/JS). **Shape**: `call_expression` `vi.stubEnv` whose first argument is the string `HOME` or `USERPROFILE`, and assignment to `process.env.HOME`/`USERPROFILE`; scope `test/**`, `integration/**`; exclude `test/support/user-home.ts`, `test/support/setup-rerail-home.ts`, `test/support/user-home.test.ts`. Steer: "Move the test's home with `stubUserHome` (test/support/user-home.ts), which moves HOME and USERPROFILE together; `vi.unstubAllEnvs()` restores both."
- **Already covered**: CONTRIBUTING.md:390 (prose only). setup-rerail-home.ts afterAll `watchRealAgentSettings` catches only the damage (a temp folder written into the person's real agent settings), after the fact. Folder half: `check:folder-removal` (scripts/check/check-folder-removal.mjs, in `npm run check`, enabled) already forbids `rm`/`rmSync` with `recursive`, `rmdir`, rimraf in tests.
- **False positives**: a test of home resolution itself that must stub one variable (none on main outside test/support); Git-config tests that only mean "Git's home" (5 above) — `stubUserHome` serves them too, so the steer still holds.
- **Mode**: `log`, because main has 13 matches; they are one-line swaps, after which it can `steer`.


<a id="rerail-changelog-security-type"></a>

### 13. `rerail-changelog-security-type`: a `.changelog/` entry that describes a closed vulnerability is typed `security-`, not `fixed-` or `changed-`

- **Pattern**: JF's release notes describe a hole closed against another user, a planted link or a program that a repository's config names, but the file is named `fixed-…`. **Why it costs**: "`.changelog/README.md` gives a closed vulnerability its own type (`security-`), and this is one by the threat model's own boundary: rename the file" (Moti, #978, 4670038959).
- **Evidence**: 1 human finding (#978) and 2 fix-up commits that refile an entry (d149386ef2 in #978, f7e8d96cc3 in #503), in 3 PRs (#503, #920, #978). One more entry merged untyped as security: #920's, released under "Fixed". The other release-note-mismatch cases (overclaims, and #635's `fixed-` that should be `changed-`) are not lexical, so this rule does not reach them.

  | PR | Ref | Commit | File:line | Code / finding |
  |---|---|---|---|---|
  | 978 | Moti 4670038959 (F1) | added 4210576976, fix d149386ef2 | .changelog/fixed-git-calls-start-no-repository-program.md:1 | `…no longer start a filter or signature program only a repository's config names…` |
  | 503 | (fixed by JF before review) | added c48aeb36c6, fix f7e8d96cc3 "a security entry for the Windows fixes" | .changelog/fixed-windows-scrub-never-follows-a-link.md:1 | ``On Windows, `rerail scrub` no longer rewrites the file that a symbolic link planted at `daemon.log` points to.`` |
  | 920 | none (merged, released under Fixed) | 11e1228099 (from ee03580667) | .changelog/fixed-windows-session-records-private-whoever-made-the-folder.md:1 | `…session records outside your profile folder are now private to you even when .rerail\sessions was made before…` |

- **Today on main**: 0 of 8 pending entries match. Pattern (`git grep -i -P` on `.changelog/{fixed,changed,added,removed,deprecated}-*.md`): `\b(?:vulnerab\w*|attacker|malicious|exploit\w*|GHSA-|CVE-\d|another (?:local )?user|other users|any local user|untrusted|planted|private to you|a repository(?:'s)? config names|committed to a repository)\b`. History: 354 entry files ever added on main (JF wrote 115 `fixed-`, 15 `changed-`, 3 `security-`, 2 `added-`). Every match among typed non-security entries is JF's #920 entry. On the released `CHANGELOG.md`, 1 match among 151 Fixed + 59 Changed + 29 Added items, the same #920 item. On every branch (`git log --all -p` over those globs) there are 3 files, all JF's, all listed above. Of the 12 `security-` entries on main, 6 match the words, so the vocabulary is real.
- **Checkable**: code rule (path and text regex, `docs/path-and-text-rules.md`). **Shape**: `{ "regex": { "source": "file.content", "pattern": "(?i)<pattern above>" } }`, scope `.changelog/{fixed,changed,added,removed,deprecated}-*.md`, no exceptions. **Steer**: "This entry describes a closed vulnerability: name it `security-…` (`.changelog/README.md`), and say what still gets through." `.changelog/` is a hidden folder, so `rerail rules test`'s repository pass does not read it (as with the `.github/` workflow rules). Count it on a copy; live checks read it.
- **Already covered**: no. `core-changelog-check.mjs` (`changelogEntryTypeRefusal`, CI, enabled) only requires one of the six types.
- **False positives**: none in history. Possible ones: a `fixed-` entry about a bug report that mentions "other users" without a security sense, and a `changed-` entry that loosens a permission on purpose. The word list misses vulnerabilities written in other words ("a hook sends nothing to a pipe Windows names as another process's" in #1051's `security-` entry does not match).
- **Mode**: steer, because main has 0 matches and the 3 historical matches are all real. The yield is small (about 1 finding per 170 PRs), so build it only as a cheap add-on to the conversation rule.


<a id="rerail-rustfmt-before-commit"></a>

### 14. `rerail-rustfmt-before-commit`: Before committing Rust under `native/`, run `node scripts/check/check-rustfmt.mjs --write`; the pre-commit hook does not.

- **Pattern**: JF's agent writes Rust (mostly test `assert!`/`assert_eq!` calls, closures and `use` lists over rustfmt's 100 columns), commits and pushes; CI's "Rust" job (`npm run check:native` → check-rustfmt.mjs) fails; a fix-up commit applies rustfmt's layout, often one file or one hunk at a time. **Why it costs**: one CI round trip per fix-up; #812 alone has six rustfmt-only commits on 2026-11-19 (da345bfc3c, b7d08a584b, 3fa9a2163b, 5ba180f506, f679b25138, 0a705d88f4). No reviewer comment: CI catches it.
- **Evidence**: 0 review findings; 16 fix-up commits in 7 PRs (#812 ×6, #921 ×4, #975 ×2, #756, #904, #973, #1051).

  | PR | Ref | Commit | File:line | Code / finding |
  |---|---|---|---|---|
  | 812 | fix-up | 0a705d88f4 | native/src/git_ignore.rs:1508 | `assert!(!(within && tells), "a stamp read within its change time's step is kept");` → 4 lines |
  | 921 | fix-up | 5ed6439d2e | native/src/git_workspace.rs:2083 | `assert_eq!(source_alias_hashes(), 1, "restore {restore} hashes the file");` → 5 lines |
  | 1051 | fix-up | 6f84a7f9c7 | native/src/pipe_server.rs:127 | `use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_READ_ATTRIBUTES, OPEN_EXISTING};` → 3 lines |
  | 973 | fix-up | 9717e734fa | native/src/code_rule_dsl/mod.rs:4779 | `contain_compile_panic(\|\| {` … collapsed onto one line |
  | 756 | fix-up | 39e72b3240 | native/src/git_start/run.rs:375 | `.args([case, "--exact", "--ignored", "--nocapture", "--test-threads=1"])` → one per line |
  | 921 | fix-up | 74ba6cab3d | scripts/check/check-rustfmt/files.txt:365 | three new Rust files added to the rustfmt list |

- **Today on main**: 0 by construction: CI's rust-clippy job runs `npm run check:native` (ci.yml:658), so main's 366 listed files are rustfmt-clean. The pre-commit hook (scripts/check/pre-commit.sh, installed by `npm ci`/`prepare`) runs `just --fmt`, the changelog check, `npm run check`, `check:ui-tokens`, `check:reference`, `check_dev_features.py`; `npm run check` is tsc plus the scripts/check node checks; `check-rustfmt.mjs` is only in `check:native`. A regex stand-in (code line >100 in a listed file) is useless: 1,798 such lines on main, string literals rustfmt leaves alone. 1,144 native/src .rs files are unlisted (the engine line keeps its author's layout), so a rule must follow files.txt.
- **Checkable**: conversation rule, or (better) no rule: add `node scripts/check/check-rustfmt.mjs` to pre-commit.sh (it formats each listed file on stdin; no cargo build). **Shape** (if a rule): on a Bash command matching `^git\s+(commit|push)\b` when `git diff --name-only` (staged or HEAD..) has a path in scripts/check/check-rustfmt/files.txt or a new `native/**/*.rs`; steer: "Run `node scripts/check/check-rustfmt.mjs --write` and commit its layout first: CI's Rust job fails on any listed file rustfmt would change, and a new Rust file written for this repository goes on that list."
- **Already covered**: CI only (`check:native` in rust-clippy job). Not in pre-commit.sh; no Rerail rule (no pack has a conversation rule, none mentions rustfmt).
- **False positives**: commits touching only unlisted engine-line files (rule must read files.txt); `--no-verify` commits bypass the hook option. Running rustfmt needs the pinned toolchain on PATH (pre-commit already assumes node/npm).
- **Mode**: steer if built as a conversation rule (main has 0 violations; the check is exact); preferred fix is the one-line pre-commit addition, not a rule.


## Considered and not proposed

Candidates that were checked against the history and not proposed as new rules.


<a id="complexity-ceiling"></a>

### `complexity-ceiling`: (no new rule) keep functions within the complexity pack's ceiling at edit time, not at review.

- **Pattern**: a function JF adds or edits goes over the cyclomatic ceiling; the reviewer's Rerail scan reports it as a blocker; a fix-up splits it into helpers. **Why it costs**: "`holdMissedFiles` … has cyclomatic complexity 7, which is over the enforced ceiling of 6" (bot blocker, #865 4634875736); in #657 the maintainer did the split himself (58c567cd1b, gdgiangi-nera). JF, #974: "N1 and N4 edit functions over the complexity ceiling at `main` (13 and 15), which the rules review counts as new; fc5c90bf6 splits both".
- **Evidence**: 21 findings (15 B / 3 F / 0 N bot + 3 human) in 5 PRs (#529 9, #657 4+3 human, #920 3, #530 1, #865 1); 21 fix-up commits in 15 PRs (#503 ×3, #529 ×2, #638 ×2, #652, #657, #672, #676, #677, #826, #909, #912, #963, #974 ×2, #975 ×2, #997).

  | PR | Ref | Commit | File:line | Code / finding |
  |---|---|---|---|---|
  | 865 | 4634875736 (B1) | fix 345a067d01 | src/engine/code-rule/code-rule-bridge.ts:437-453 | `holdMissedFiles` CCN 7 > 6 |
  | 920 | 4646361216 (B4), 4646361227 (B5) | — | src/runtime/runtime.ts:993, 1032 | `ignoreInGit` CCN 10, `foundGitignore` CCN 10 > 6 |
  | 529 | B1-B8@2ea7b4b4a | bd4e7cfaff, e6baebd6f0 | native/src/code_rule_dsl/compile/ast_rule.rs:397 | `compile_ast_rule` CCN 96 > 6 (Timo's function, edited): fix "Restored main's text in over-ceiling functions so complexity findings stay existing" |
  | 657 | B1-B2@4ebefddd7, 6927307399 (human) | 58c567cd1b (maintainer) | src/cli/cli.ts:849, src/daemon/daemon.ts:1271 | `main` 15, `runRerailDaemon` 15 |
  | 530 | B1@e1e4ee354 | — | src/engine/storage/database.ts:301 | `applySchema` CCN 8 > 6 |
  | 974 | (self, JF reply) | 0ae0b3559a | src/api/operation.ts:150 | "…helpers to keep complexity at 6" |

- **Why it still reached review**: (1) every finding and every fix-up predates #864 (e9d9020b48, 2026-11-24 19:21 UTC), which set `.rerail/packs.json`'s `ceiling` to 10; until then the pack's default 6 applied (rules `complexity-cyclomatic-{typescript,javascript,rust}` are `"mode": "steer"`; Rust is covered, so #529's 9 Rust findings were the same rule at 6). The last fix-ups (0ae0b3559a, 4aa419ec35, 051af17706) are 3-4 h before #864. Since #864 JF's PRs (#1048-#1118) have no complexity finding. (2) About half the findings are functions already over at main that JF's edit touched (#529's eight engine functions CCN 11-96; #974's 13 and 15; #657's `main`); the scan counts an edited over-ceiling function as new, so any edit to one costs a split. (3) It is caught by the reviewer workflow's Rerail module (.github/workflows/reviewer.yml `modules: rerail`), after push. The pre-commit hook (scripts/check/pre-commit.sh) runs no Rerail scan, and [INFERENCE] JF's agent sessions did not run with Rerail's edit-time hooks on this checkout (no reply or commit mentions a steer; fixes always follow review).
- **Today on main**: lizard 1.24.0 (the counting the pack copies) over `git archive origin/main src scripts native/src native/crates native/build.rs`, tests/vendor/generated excluded: >10: TS 168, JS 45, Rust 2,715; >6: TS 489, JS 119, Rust 4,711. Blame of each TS/JS function's first line (>10, n=213): JF 16, Timo 144, Jeff Snell 36, Gabriel Giangi 16, minh-nera 1 (e.g. src/engine/code-rule/checkout-code-rule-router.ts:348 `walkView` 39, src/setup/registered-hooks.ts:54 24). Rust sample of 60: 60 Timo.
- **Checkable**: code rule, exists (complexity pack, AST). **Shape**: no change to the rule. Recommend: run JF's sessions with Rerail's hooks on (the repo pack list already loads it), so the steer fires at the edit; and steer text for an edited function already over at main: "this function was over the ceiling before your edit; keep your change out of it or split it first" (pack-level, not a new rule).
- **Already covered**: `complexity-cyclomatic-typescript`/`-javascript`/`-rust` (pack `complexity` 0.1.9, ceiling 10), enabled, mode steer.
- **False positives**: functions already over at main (2,900 at ceiling 10, mostly Rust engine code by Timo) fire on any edit inside them; the cost in #529 and #974 was such edits.
- **Mode**: steer (as today); no new rule.


<a id="rerail-monotonic-durations"></a>

### `rerail-monotonic-durations`: Time a duration, a deadline or an in-process order on `performance.now()` (or a fact), not on `Date.now()`

- **Pattern**: product code reads the wall clock where a step of the time of day changes the answer:
  - an in-process ordering on wall-clock values (`owner.updatedAt < performance.timeOrigin`);
  - a deadline or elapsed time taken from `Date.now()`;
  - a stored "when", taken from the wall clock, that is later compared with other stored times.

  **Why it costs**: "A backward clock step makes every sweep end this daemon's own live lockless sessions … After the clock steps back (NTP, a VM resume) … the next minute's sweep … ends those live sessions" (bot, #680 4573392859/F8); "An interrupted check can get a finishedAt before its startedAt … a negative duration, shipped in support bundles" (bot, #974 4654106699).
- **Evidence**: 8 findings (1 B / 5 F / 1 N / 1 human) in 6 PRs (#680, #974, #648, #1104, #812, #1118); 3 fix-up commits (5e8d1609de, b8218efc9f, 3b1685caf4). The findings share a theme but not a shape:
  - **in-process duration or order on the wall clock**, which a matcher can see: 2 findings, #680 F8 and #1118 N1. The #1118 code is Timo's 71e0822ce6, and JF's #1118 moved the pause onto `performance.now()`.
  - **a stored wall time chosen or compared wrongly**: 3 findings, #974 ×2 and #648.
  - **file times against the machine clock**: 2 findings, #1104 (skew) and #812 B2 (the order of the clock read).

  Two later fixes on main (221152067f file-lock.ts:146 and f634d61f3b home-daemon-lock.ts:105) replaced JF's `deadline = Date.now() + ms; … await sleep(POLL_MS)` loops with waits on the deciding event. They were fixed as "waited on clocks where a fact could decide", not for clock steps.

  | PR | Ref | Commit | File:line | Code / finding |
  |---|---|---|---|---|
  | #680 | 4573392859/F8 | fix 5e8d1609de | src/engine/audit/audit-control.ts:142 | `return owner.processId === process.pid && owner.updatedAt < performance.timeOrigin;` |
  | #974 | 4653828964 | fix b8218efc9f | src/engine/audit/audit-control.ts:484 | `const at = Date.now();` → `finishedAt: at` for an interrupted check |
  | #648 | 4580933111 | PR head refs/jf-mining/pr/648 | src/engine/storage/migrations.ts:320 | `const appliedAt = Date.now();` then compared with audits' `started_at` |
  | #1104 | 4702941164 | still on main | src/engine/native/git-ignore-answers.ts:148 | `const now = BigInt(Date.now()) * 1_000_000n;` against index file times |
  | #503→#964 | followup 221152067f | intro 53f5b92a1c | src/engine/native/file-lock.ts:146 | `const deadline = Date.now() + (request.waitMs ?? 0);` + poll |

- **Today on main**: 37 lines in `src/**/*.ts`, leaving out tests and `src/ui`. JF wrote 24 of them (blame on every line), Timo 10, Gabriel 2 and Jeff 1. `performance.now()` appears 48 times in the same scope. Patterns (`git grep -nP`):
  - elapsed: `Date\.now\(\)\s*-\s*[\w.#]+` (15 lines);
  - deadline: `(const|let)\s+\w*(deadline|until|end\w*|due\w*|expires?\w*|giveUp\w*)\s*=\s*Date\.now\(\)\s*\+` (12 lines);
  - deadline check: `Date\.now\(\)\s*[<>]=?\s*\w*(deadline|until|due)` (9 lines);
  - timer: `setTimeout\([^;]*-\s*Date\.now\(\)` (1 line).

  Examples: src/runtime/daemon-control.ts:2339-2340 `const deadline = Date.now() + ms; while (alive() && Date.now() < deadline) await sleep(POLL_INTERVAL_MS);` (JF 53f5b92a1c) and src/engine/code-rule/worktree-walks.ts:756 (Timo).
- **Checkable**: only partly. A code rule (regex/AST) can flag `Date.now()` used as a deadline or elapsed time inside one process. Stored-time ordering (#974, #648) and file-time comparisons (#1104, #812) need a reviewer. **Shape**: TypeScript, `src/**`, leaving out tests. It matches `Date.now() ± x` assigned to a deadline-like name or compared with one, and `setTimeout(…, x - Date.now())`. It does not match values that are stored, sent or compared with file or process times (`ctimeMs`, `uptime`, rows). Steer: "A duration or deadline inside one process should be timed on `performance.now()` (or end on the event that decides it), since a clock step stretches or cuts a `Date.now()` wait."
- **Already covered**: none. No `Date.now` or monotonic rule exists in any pack, and none is enabled here.
- **False positives**: high. Wall-clock ages are right when they are compared with file or process times or with values from another process:
  - src/engine/git-crlf-reader.ts:94 `Date.now() - Math.floor(info.ctimeMs)`;
  - src/engine/process-liveness.ts:347 (uptime);
  - src/runtime/daemon-control.ts:2078 (a claim another process wrote);
  - src/cli/cli.ts:425 (a log window);
  - src/daemon/daemon.ts:874 (`uptimeMs` reported).

  About a third of the 37 lines are of this kind.
- **Mode**: log, if it is built at all. Only 2 of the 8 findings fit a matchable shape (one of them in code JF did not write), and main has about 25 in-process uses after the obvious exceptions. Verdict: not a rule now. Leave it to the reviewer, and fold the `Date.now()` deadline-plus-poll loops into a "wait on the deciding event" reviewer note.


<a id="rerail-log-shape"></a>

### `rerail-log-shape`: a failure log line carries the whole error (its stack in daemon code) and goes to the injected stream, not `process.stderr`

- **Pattern**: a failure log line has one of two faults.
  - It writes only `error instanceof Error ? error.message : String(error)` where daemon.log needs the stack.
  - It writes to `process.stderr` inside code that was handed a `stderr` (support bundle, CLI context).

  **Why it costs**: "Move failure logs only the message, not the stack" (#648, 4568519161). "Log line goes to `process.stderr`, not the support-bundle command's injected stderr" (#989, 4657869525). #676 F7 (4598422292) says the catch "keeps only the message".
- **Evidence**: 3 bot findings (0 blockers, 1 follow-up, 2 nits) in 3 PRs and 3 fix-up commits (611a488ab2, 82090990af, 167b173adc). There are no human findings. #975's fix-up cc812c38df (N1) changed what a log line says, not its form, and is not counted. The 2 sub-shapes differ: a missing stack, and the wrong stream.

  | PR | Ref | Commit | File:line | Code / finding |
  |---|---|---|---|---|
  | 648 | 4568519161 (nit) | fix 611a488ab2 | src/daemon/daemon.ts:553 (569 at fix parent) | `process.stderr.write(\`rerail daemon: moving audit rule provenance stopped: ${error instanceof Error ? error.message : String(error)}\n\`)` |
  | 989 | 4657869525 (nit) | fix 82090990af | src/engine/audit/audit-store.ts:133 (174 at fix parent) | `process.stderr.write(\`rerail: ${database.$client.name}: audit ${row.auditId} reads with no ${name}, …` inside a reader the support bundle calls with its own `stderr` |
  | 676 | 4598422292 (follow-up) | fix 167b173adc | src/engine/storage/database.ts:280 | `return \`the keys an earlier build stored on ${…} wait: ${sqliteReason(error)}\`` (the message only, and the hook path drops it; the fix adds an injected `say`) |

- **Today on main**:
  - There are 55 log lines in `src/**/*.ts` that interpolate only the message. Pattern: `(process\.stderr\.write|console\.error|\.write)\(\`[^\`]*\$\{\s*(\w+) instanceof Error \? \2\.message : String\(\2\)\s*\}`.
  - By blame on all 55: 28 are JF's and 27 are others'. By folder: runtime 31, engine 12, cli 6, harness 4, daemon 2.
  - Only 2 of the 55 are `rerail daemon:` lines: `src/daemon/daemon.ts:487` (Gabriel Giangi) and `src/daemon/heap-ceiling.ts:97` (JF).
  - Only 2 places use `error.stack ?? …`.
  - Main also has many `errorText`/`messageOf` helpers that keep only the message (setup-cli, history-sync, native-session-client, rr-child, record-writes), which the regex misses.
  - For the injected stream: 5 files take a `stderr: Pick<NodeJS.WriteStream…>`-style input (mcp-bridge, rule-draft, show-me, support-bundle, hook-adapter), and **0** of them also call `process.stderr.write`. The #989 break was across files: a reader called from the bundle wrote to the global stream.
- **Checkable**: reviewer only. A regex can find message-only interpolation, but most CLI and user-facing lines should leave the stack out, so the rule cannot tell right from wrong without knowing that the line ends in daemon.log. The wrong-stream case needs a call graph (a module reached from a caller that injects `stderr`), which is outside what a relational rule can express here. A possible sketch: TypeScript, `process.stderr.write` template containing `.message : String(` under `src/daemon/**` and `src/runtime/engine-service.ts`. Its steer would be: "daemon.log gets the error's stack: write `error.stack ?? error.message`."
- **Already covered**: none (no pack has a stack or stream rule; `javascript/eslint-no-console` is not enabled and is a different shape).
- **False positives**: nearly all 55 regex matches are correct message-only lines for a user, for example the CLI refusals in `src/runtime/**`. The daemon-scoped sketch has 2 matches, and the stack is arguable even for those.
- **Mode**: log at most. It is too thin (3 findings, 2 sub-shapes) to build. Fold "with its stack in daemon code" into `rerail-no-silent-catch`'s steer text.


<a id="mechanical-minor"></a>

### Mechanical shapes with fewer than 3 occurrences each

Counts: findings B/F/N bot + human; fix-ups = JF fix-up commits; main = matches on origin/main with the pattern given (scripts over `git cat-file --batch origin/main:<file>`, vendor/fixtures/upstream excluded), JF/others by `git blame origin/main` of every match.

| id | findings (B/F/N/human) | PRs | fix-ups | main (JF/others) | coverage | verdict |
|---|---|---|---|---|---|---|
| shadow-import | 0/0/2/0 | 2 | 0 (fixed in-branch: ab097e9b20 #997, `resolve: settle` in #1095 head) | 43 sites in 19 files bind `resolve` where the file imports `resolve` from node:path; 7 destructure it from `Promise.withResolvers` (0 JF / 7: Timo 6, Jeff Snell 1) | typescript-eslint-no-shadow (typescript pack, steer, not enabled) | not a rule here; if anything enable no-shadow in log |
| import-hygiene | 0/0/3/1 | 4 (+#503 fix-up) | 2 (d4d507b459 #881, c5975516d3 #503) | ~14 unused imports in 12 files (JF 2: src/runtime/session-store.ts:6 `isNull` 622a9b11a4, test/hosts/omp/extension.test.ts:1 `existsSync` 53f5b92a1c; others 12); relative import above a `node:`/package import in 29 of 1,080 TS files | typescript-eslint-no-unused-vars (steer, not enabled; tsc runs without noUnusedLocals); eslint-sort-imports (javascript pack, steer, not enabled) sorts by member syntax then name, not node:-first, so it would misfire on this repo | not a rule; one-shot: turn on `noUnusedLocals` in tsconfig (14 fixes) |
| new-short-read | 0/1/1/0 | 2 | 0 (f2376a1 #978 bot fix) | 5 statement-level `readSync(…)` with the count dropped: src/cli/support-bundle.ts:598 (Gabriel Giangi 4ea997179f), src/engine/native/home-daemon-lock.ts:298 (JF c219472639, a 1-byte probe), 3 in tests reading stdin to block; #1104's git-ignore-answers.ts site is fixed on main (`const read = readSync(…)`:122) | none (node pack has n-no-sync only) | not a rule (2 hits; a "result ignored" regex `^\s*(fs\.)?readSync\(` is cheap but ~0 value); reviewer only for loop-until-full (#978) |
| new-delete-before-replace | 0/1/1/0 | 2 | 0 (d100d92f85 #657 bot fix) | 1: test/setup/harness-registration.test.ts:746 `await rm(nodePath)` then `rename(…aside, nodePath)` (a test putting a file back, intended) | none | not a rule |
| new-unhandled-rejection (+ new-unhandled-error) | 0/1/4/0 | 5 (#665, #674, #865, #987, #1051) | 0 (bot fixes 7190062, 4c3dd37, 21bb046a3) | 32 `void p.then(` lines, ~22 with no second handler on the same line (crude; many are `void settled.then(() => set.delete(…))` on promises that never reject) | none (no no-floating-promises/catch-or-return rule in packs) | reviewer only: whether `p` can reject or the handler can throw needs types/dataflow; 5 findings are 4 different shapes |
| platform-gap (wasm32-wasip1 arm) | 1/0/0/0 | 1 (#812 B1, 4635513359) | 1 (f6b3c324c1) (+#912's 8628762c66 for the wasm link order) | 14 of 18 native files with both `#[cfg(unix)]` and `#[cfg(windows)]` have no wasi/`not(any(unix, windows))` arm, and main builds: file-level matching is all false positives | CI: `rust-wasm` job ("Rust engine (browser build)", ci.yml:674), added by JF in #912 (d69d7ab68c, 2026-11-20), runs `cargo check --target wasm32-wasip1` on same-repo PRs whenever the change reaches the engine crates; #812 predates it | not a rule: CI now catches it |

Notes
- shadow-import: the repo's own fix is to rename (`const { promise, resolve: settle } = Promise.withResolvers()`, 10+ sites on main) or use `node:timers/promises`' `setImmediate()`. no-shadow would also fire on the 36 idiomatic `new Promise(resolve => …)` sites, so log at most.
- import-hygiene sub-shapes are all different: unused import (#679 `execFile` in src/engine/git.ts:6, N1@d7aa8b2c5; d4d507b459 `inArray`), order (#1116 4705226276, fixed 34a3ca8a7e), unused re-export (#987 human 4670037130, src/runtime/index.ts:22), unused test field `link: "conf"` (#881 4635096278, fixed b4bd7d94d0), missing import (`rm`, c5975516d3; tsc already covers it, test/** is in tsconfig's include).


## Open questions

1. **Do JF's agent sessions run Rerail's hooks with this repository's packs?** The complexity rules were in Steer
   mode yet produced 15 blockers and 21 fix-ups, all found by the review bot's rules pass after a push. Either his
   sessions do not run the hooks, or the steer came too late to change the edit. Nothing on GitHub tells which. If the
   hooks run, each candidate above acts twice: at the edit, and in the bot's rules pass. The rules pass reported the
   `complexity` pack's findings; that it runs `packs/rerail` the same way (#1123) is an inference, not checked here.
   Without the hooks, only the rules pass sees a new rule.
2. **Conversation rules.** Candidate 1 is a conversation rule. Those are still off by default (experimental), and
   `block` is not enforced, so in Log it only records. It saves a review round only once it can hold the
   `@neralabs-reviewer review` command until the agent has re-read its texts.
3. **Sweep before Steer.** Candidates 4, 5, 6, 10, 11 and 12 match only a few lines on `main` today (3 to 29, mostly
   mechanical). One commit fixing those lines lets each ship in Steer, as `packs/rerail` did. The others (2, 3, 7, 8,
   9) have to start in Log.
