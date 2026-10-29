# Governance

How Rerail is run: who takes part and how, who the maintainers are, and how decisions are made and recorded. How to
contribute day to day is in [CONTRIBUTING.md](../CONTRIBUTING.md).

Rerail is an open-source project developed by Nera Software Inc. and licensed under the Apache License 2.0. At this
stage it is open source with closed contribution: anyone can use, fork and redistribute the code under the licence,
and anyone can send bug reports, feature requests, questions and links to changes in their fork through the issues,
but only Nera Software employees and the project's own automation open pull requests here. If that changes, this document changes first.

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

- **In the open.** Decisions about Rerail's code, rules, packs and docs are made in its issues and pull requests. A
  decision reached somewhere else, such as on a call, is written into the issue or pull request it concerns, with its
  reason, before anyone acts on it.
- **Everyday changes.** A pull request merges once a maintainer other than its author approves it and **CI result**
  passes. It is squash-merged.
- **Changes that start with an issue.** For anything [Start with an issue](../CONTRIBUTING.md#start-with-an-issue)
  lists, a maintainer agrees the approach in the issue before the code is written. A large change writes a short design
  there first ([Large changes](../CONTRIBUTING.md#large-changes)).
- **Disagreement.** Maintainers look for consensus. Any maintainer can object to a change with a reason, and the
  change waits while it is talked through in the issue or pull request. If the maintainers still disagree, a majority of
  them decides, and the issue records the outcome and why.
- **Saying no.** A maintainer closes an issue or pull request as `wontfix` or `duplicate` with a one-line reason, and
  links the original for a duplicate.
- **Releases.** Core and the packs publish only after one of the `release` environment's required reviewers, who are
  maintainers, approves the run ([docs/release.md](release.md)).

## Who the maintainers are

Nera Software appoints and removes maintainers internally. There is no nomination, application or promotion to
maintainer from contributing here. A pull request to [.github/CODEOWNERS](../.github/CODEOWNERS) adds or removes a
maintainer, and that file is the public record of who they are.

## Changing this document

Nera Software decides the policy this document states. A pull request records a change to it and merges like any other.
