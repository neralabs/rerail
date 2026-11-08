# Governance

Who takes part in Rerail, who maintains it, and how decisions are made. For day-to-day contribution, see
[CONTRIBUTING.md](../CONTRIBUTING.md).

Nera Software Inc. develops Rerail under the Apache License 2.0 and holds the copyright ([NOTICE](../NOTICE)).
Contribution is gated:

- Anyone can use, fork and redistribute the code, and propose a change from their fork.
- Code lands only through a pull request from a branch here, opened by contributors or the project's automation.

[Reviewing the gate](#reviewing-the-gate) covers how the gate is reviewed.

## Roles

Four roles take part, from anyone opening an issue to the maintainers who merge.

| Role | Who | What they do |
| --- | --- | --- |
| Anyone | Anyone | Opens and comments on issues: bugs, rule misfires, feature and integration requests, questions. Proposes changes from a fork, as a pull request or a branch linked from an issue, with no CI ([Proposing a change from your fork](../CONTRIBUTING.md#proposing-a-change-from-your-fork)) |
| Contributor | Nera Software employees, and outsiders the maintainers approve ([the list](../CONTRIBUTING.md#the-list)) | Pushes branches here and opens pull requests from them, with CI. Claims `good first issue` and `help wanted` issues |
| Automation | Dependabot, and the GitHub Actions release and upstream bump workflows ([How contribution works](../CONTRIBUTING.md#how-contribution-works)) | Opens dependency update, release and upstream bump pull requests, reviewed and merged like any other |
| Maintainer | Code owners ([.github/CODEOWNERS](../.github/CODEOWNERS)), appointed by Nera Software | Reviews and merges, ports fork proposals, triages issues, agrees the approach for issue-first work, keeps the contributor list, enforces the [Code of Conduct](../CODE_OF_CONDUCT.md) |

A fork pull request is a proposal. A maintainer who takes it ports it to a branch here in their own pull request,
credits the author with `Co-authored-by:`, and closes the original with a link.

## How decisions are made

Nera Software sets policy; maintainers decide within it, in the open.

- **Final say.** Nera Software sets this policy and appoints maintainers. Within it, maintainers decide by consensus,
  or by majority if they cannot agree ([Disagreement](#how-decisions-are-made)).
- **In the open.** Decisions on code, rules, packs or docs are recorded, with the reason, in the issue or pull request
  before anyone acts.
- **Everyday changes.** A pull request merges with one approval from someone other than its author and **CI result**
  passing. It is squash-merged. Code owners review, and the automated reviewer (`reviewer.yml`) may approve: its
  approval counts, since main requires no code-owner review. It never approves a pull request that touches a high-risk
  path in [REVIEW.md](../REVIEW.md#high-risk-areas); a code owner approves those.
- **Issue first.** For anything [Start with an issue](../CONTRIBUTING.md#start-with-an-issue) lists, a maintainer
  agrees the approach in the issue before coding. Large changes write a short design there first
  ([Large changes](../CONTRIBUTING.md#large-changes)).
- **Disagreement.** Any maintainer can object with a reason; the change waits while the issue or pull request discusses
  it. If disagreement persists, a majority decides and the issue records the outcome and why.
- **Saying no.** A maintainer closes as `wontfix` or `duplicate` with a one-line reason, linking the original for a
  duplicate.
- **Asking again.** Dispute a decision on your issue, pull request or contributor request there. A maintainer other
  than the one who decided answers.
- **Releases.** Core and packs publish only after one of the `release` environment's required reviewers, all
  maintainers, approves the run ([docs/release.md](release.md)).

## Issues that wait on their reporter

When a maintainer cannot act on an issue without more from its reporter, a version, a reproduction, the rule it
concerns, they ask for it in the issue and label it `needs-info`. From then on the issue is the reporter's turn, and
[.github/workflows/stale.yml](../.github/workflows/stale.yml) keeps it from waiting for ever:

- A comment from the reporter swaps `needs-info` for `needs-triage`. The issue is back in the maintainers' queue, and
  the [response times](../CONTRIBUTING.md#response-times) apply from that comment.
- After 14 days with no activity, the bot asks once more and labels the issue `stale`. Any comment or edit removes
  `stale` at the bot's next daily run; a comment from the reporter removes it at once.
- 7 days after that, it closes the issue as not planned, saying how to get it reopened: the reporter comments there;
  anyone else, or a reporter whose details have changed, opens a new issue. The reporter's comment reopens it and puts
  it back in `needs-triage`. An issue a maintainer closed by hand stays closed.

Nothing else is closed for age. An issue a maintainer has confirmed (`bug`, `rule-misfire`, `known-issue`,
`enhancement`) stays open until it is fixed or declined with a reason ([Saying no](#how-decisions-are-made)), however
old it is. One still in `needs-triage` is waiting on a maintainer, not its reporter, and the response times are the
measure of that. An assigned issue, an issue on a milestone, and a pull request are never touched.

## Who the maintainers are

Nera Software appoints and removes maintainers internally; [.github/CODEOWNERS](../.github/CODEOWNERS) is the public
record.

There is no nomination, application or promotion to maintainer, from contributor or otherwise. A pull request to that
file adds or removes a maintainer.

## Internal-testing features

Graduating or removing an [internal testing](internal-testing.md) feature (in the code, unsupported) needs one approved
issue.

1. Rerail's product owner at Nera Software proposes it in an issue, with the reason.
2. Nera Software's chief product officer approves it there.
3. The follow-up pull request links that issue.

## Reviewing the gate

Nera Software reviews the gate at least every six months in a public issue, first by 30 April 2027.

Each review records what it looked at, the outcome and why.

<details>
<summary>Why the gate exists and what each review checks</summary>

The gate ensures every change comes from someone accountable, costly CI runs only on branches here, and review meets
the [response times](../CONTRIBUTING.md#response-times). Each review looks at:

- **Contributor requests** since the last review: how many, how many approved, time to decision.
- **Fork proposals:** how many ported or declined, time to decision.
- **Response times:** first response within 5 business days, decision within 10.

</details>

## Where to reach the project

Use issues for most things; report security and conduct problems privately.

- **Issues:** bugs, rule misfires, feature and integration requests, questions, contributor requests, fork changes.
- **Security:** privately, per [SECURITY.md](../SECURITY.md), never in a public issue.
- **Conduct:** [opensource@nera.software](mailto:opensource@nera.software), per the
  [Code of Conduct](../CODE_OF_CONDUCT.md#reporting).

## Changing this document

Nera Software decides this policy; a pull request records each change and merges like any other.
