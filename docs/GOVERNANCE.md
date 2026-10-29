# Governance

How Rerail is run: who takes part and how, who the maintainers are, and how decisions are made and recorded. How to
contribute day to day is in [CONTRIBUTING.md](../CONTRIBUTING.md).

Rerail is an open-source project developed by Nera Software Inc. and licensed under the Apache License 2.0, and it
stays under that licence. It is open source with gated contribution: anyone can use, fork and redistribute the code
under the licence, and propose a change from their fork, but code reaches this repository only through a pull request
from a branch here, which Nera Software employees, approved contributors and the project's own automation open.
[Reviewing the gate](#reviewing-the-gate) says when and how that is looked at again.

Nera Software Inc. holds the copyright ([NOTICE](../NOTICE)). The licence does not grant use of the Rerail name
(section 6 of the Apache License), so a fork can use the code but not present itself as Rerail.

## Roles

| Role | Who | What they do |
| --- | --- | --- |
| Anyone | Anyone | Opens and comments on issues: bug reports, rule misfires, feature and integration requests, questions. Proposes a change from their fork, as a pull request or a branch linked from an issue, with no CI ([Proposing a change from your fork](../CONTRIBUTING.md#proposing-a-change-from-your-fork)) |
| Contributor | The list in [CONTRIBUTING.md](../CONTRIBUTING.md#the-list), approved by the maintainers | Pushes branches here and opens pull requests from them, with CI; claims `good first issue` and `help wanted` issues |
| Nera Software employee | Nera Software employees with write access | Opens pull requests from branches here |
| Automation | Dependabot, and the GitHub Actions release and upstream bump workflows ([How contribution works](../CONTRIBUTING.md#how-contribution-works)) | Opens dependency update, release and upstream bump pull requests, reviewed and merged like any other |
| Maintainer | The Nera Software employees in [.github/CODEOWNERS](../.github/CODEOWNERS), appointed by Nera Software | Reviews and merges pull requests, ports changes proposed from forks, triages issues, agrees the approach for work that starts with an issue, keeps the contributor list, and enforces the [Code of Conduct](../CODE_OF_CONDUCT.md) |

A pull request from a fork is a proposal. A maintainer who takes it ports it onto a branch here in their own pull
request, credits its author with `Co-authored-by:`, and closes it with a link to the new one.

## How decisions are made

- **Who has the final say.** Nera Software first: it sets the policy this document states and appoints the
  maintainers. Within that policy, the maintainers decide, by consensus or, when they cannot agree, by a majority
  ([Disagreement](#how-decisions-are-made)).
- **In the open.** A decision about Rerail's code, rules, packs or docs, wherever it is reached, is recorded in the
  issue or pull request it concerns, with its reason, before anyone acts on it.
- **Everyday changes.** A pull request merges once a code owner other than its author approves it, or the
  `neralabs-ci` reviewer approves a low-risk change (`reviewer.yml`), and **CI result** passes. It is squash-merged.
- **Changes that start with an issue.** For anything [Start with an issue](../CONTRIBUTING.md#start-with-an-issue)
  lists, a maintainer agrees the approach in the issue before the code is written. A large change writes a short design
  there first ([Large changes](../CONTRIBUTING.md#large-changes)).
- **Disagreement.** Maintainers look for consensus. Any maintainer can object to a change with a reason, and the
  change waits while it is talked through in the issue or pull request. If the maintainers still disagree, a majority of
  them decides, and the issue records the outcome and why.
- **Saying no.** A maintainer closes an issue or pull request as `wontfix` or `duplicate` with a one-line reason, and
  links the original for a duplicate.
- **Asking again.** Anyone who disagrees with a decision on their issue, pull request or contributor request can say why
  there. A maintainer other than the one who decided answers.
- **Releases.** Core and the packs publish only after one of the `release` environment's required reviewers, who are
  maintainers, approves the run ([docs/release.md](release.md)).

## Who the maintainers are

Nera Software appoints and removes maintainers internally. There is no nomination, application or promotion to
maintainer, from being a contributor or otherwise. A pull request to [.github/CODEOWNERS](../.github/CODEOWNERS) adds
or removes a maintainer, and that file is the public record of who they are.

## Reviewing the gate

Contribution is gated so that every change reaching this repository comes from someone who answers for it, so that CI,
which is costly to run, runs only on branches here, and so that review stays within the
[response times](../CONTRIBUTING.md#response-times).

Nera Software reviews the gate at least every six months, in a public issue that records what was looked at, the
outcome and why. The first review is due by 30 April 2027. Each review looks at:

- requests to become a contributor since the last review: how many, how many were approved, and how long each took to
  a decision;
- changes proposed from forks: how many were ported and how many declined, and how long each took to a decision;
- whether the maintainers met the response times: a first response within 5 business days, and a decision within 10.

## Where to reach the project

- **Issues:** bug reports, rule misfires, feature and integration requests, questions, requests to become a
  contributor, and changes from a fork.
- **Security:** privately, as [SECURITY.md](../SECURITY.md) describes, never in a public issue.
- **Conduct:** [opensource@nera.software](mailto:opensource@nera.software), as the
  [Code of Conduct](../CODE_OF_CONDUCT.md#reporting) describes.

## Changing this document

Nera Software decides the policy this document states. A pull request records a change to it and merges like any other.
