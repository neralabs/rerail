# Governance

How Rerail is run: who takes part and how, who the maintainers are, and how decisions are made and recorded. How to
contribute day to day is in [CONTRIBUTING.md](../CONTRIBUTING.md).

Rerail is an open-source project developed by Nera Software Inc. and licensed under the Apache License 2.0. At this
stage it is open source with gated contribution: anyone can use, fork and redistribute the code under the licence,
and anyone can send bug reports, feature requests, questions and changes made in their fork through the issues, but
code reaches this repository only through a pull request that a Nera Software employee or the project's own
automation opens. [Reviewing the gate](#reviewing-the-gate) says when and how that is looked at again.

Nera Software Inc. holds the copyright ([NOTICE](../NOTICE)). The licence does not grant use of the Rerail name
(section 6 of the Apache License), so a fork can use the code but not present itself as Rerail.

## Roles

| Role | Who | What they do |
| --- | --- | --- |
| Anyone | Anyone | Opens and comments on issues: bug reports, rule misfires, feature and integration requests, questions, and a change made in their fork, linked from the issue ([Proposing a change from your fork](../CONTRIBUTING.md#proposing-a-change-from-your-fork)) |
| Nera Software employee | Nera Software employees with write access | Opens pull requests, from a branch in this repository |
| Automation | Dependabot, and the GitHub Actions release and upstream bump workflows ([Who opens pull requests](../CONTRIBUTING.md#who-opens-pull-requests)) | Opens dependency update, release and upstream bump pull requests, reviewed and merged like any other |
| Maintainer | The Nera Software employees in [.github/CODEOWNERS](../.github/CODEOWNERS) | Reviews and merges pull requests, triages issues, agrees the approach for work that starts with an issue, and enforces the [Code of Conduct](../CODE_OF_CONDUCT.md) |

A pull request from anyone else is closed with a link to
[Who opens pull requests](../CONTRIBUTING.md#who-opens-pull-requests).

## How decisions are made

- **In the open.** A decision about Rerail's code, rules, packs or docs, wherever it is reached, is recorded in the
  issue or pull request it concerns, with its reason, before anyone acts on it.
- **Everyday changes.** A pull request merges once someone with write access other than its author approves it and
  **CI result** passes. That approval can come from a maintainer, another Nera Software employee, or, for a low-risk
  change, the `neralabs-ci` reviewer (`reviewer.yml`). It is squash-merged.
- **Changes that start with an issue.** For anything [Start with an issue](../CONTRIBUTING.md#start-with-an-issue)
  lists, a maintainer agrees the approach in the issue before the code is written. A large change writes a short design
  there first ([Large changes](../CONTRIBUTING.md#large-changes)).
- **Disagreement.** Maintainers look for consensus. Any maintainer can object to a change with a reason, and the
  change waits while it is talked through in the issue or pull request. If the maintainers still disagree, a majority of
  them decides, and the issue records the outcome and why.
- **Saying no.** A maintainer closes an issue or pull request as `wontfix` or `duplicate` with a one-line reason, and
  links the original for a duplicate.
- **Asking again.** Anyone who disagrees with a decision on their issue or fork change can say why in the issue. A
  maintainer other than the one who decided answers there.
- **Releases.** Core and the packs publish only after one of the `release` environment's required reviewers, who are
  maintainers, approves the run ([docs/release.md](release.md)).

## Who the maintainers are

Nera Software appoints and removes maintainers internally. There is no nomination, application or promotion to
maintainer from contributing here. A pull request to [.github/CODEOWNERS](../.github/CODEOWNERS) adds or removes a
maintainer, and that file is the public record of who they are.

## Reviewing the gate

Contribution is gated so that every change reaching this repository comes from someone at Nera Software who answers for
it, and so that review stays within the [response times](../CONTRIBUTING.md#response-times).

Nera Software reviews the gate at least every six months, in a public issue that records what was looked at, the
outcome and why. The first review is due by 30 April 2027. Each review looks at:

- the changes linked from forks since the last review: how many, how many went in, and how long each took to a first
  response and to a decision;
- whether the maintainers met the response times: a first response within 5 business days, and a fix or a decision
  within 10;
- whether there is a way to let named people outside Nera Software open pull requests without opening them to
  everyone.

When the gate opens, it opens first to people whose changes from a fork have gone in, and this document says how
before it does.

## Where to reach the project

- **Issues:** bug reports, rule misfires, feature and integration requests, questions, and changes from a fork.
- **Security:** privately, as [SECURITY.md](../SECURITY.md) describes, never in a public issue.
- **Conduct:** [opensource@nera.software](mailto:opensource@nera.software), as the
  [Code of Conduct](../CODE_OF_CONDUCT.md#reporting) describes.

## Changing this document

Nera Software decides the policy this document states. A pull request records a change to it and merges like any other.
