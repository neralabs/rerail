# Governance

How Rerail is run: who decides, how decisions are made and recorded, and how someone becomes a maintainer. How to
contribute day to day is in [CONTRIBUTING.md](../CONTRIBUTING.md).

Rerail is an open-source project started by Nera Software Inc. and licensed under the Apache License 2.0.

## Roles

| Role | Who | What they do |
| --- | --- | --- |
| Contributor | Anyone | Opens and comments on issues |
| Trusted contributor | The list in [CONTRIBUTING.md](../CONTRIBUTING.md#the-list) | Opens pull requests from a fork |
| Maintainer | The people in [.github/CODEOWNERS](../.github/CODEOWNERS) | Reviews and merges pull requests, triages issues, agrees the approach for work that starts with an issue, keeps the trusted-contributor list, and enforces the [Code of Conduct](../CODE_OF_CONDUCT.md) |

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

## Becoming a maintainer

Maintainers come from trusted contributors. The maintainers look for:

- merged pull requests and reviews over several months, in at least one part of Rerail;
- judgement that holds up in issues and reviews: the approach agreed before the code, tests that prove the change;
- how the person has worked with others, under the [Code of Conduct](../CODE_OF_CONDUCT.md).

The steps:

1. A maintainer nominates the person in an issue, linking the work that shows it. You can ask a maintainer to
   nominate you.
2. The nominee accepts in the issue and turns on two-factor authentication for their GitHub account.
3. A pull request adds them to [.github/CODEOWNERS](../.github/CODEOWNERS), linking the issue. It merges once a
   majority of the maintainers approve it and none objects, and the new maintainer is then given write access.

## Leaving

- A maintainer can step down at any time, by a pull request that removes them from `.github/CODEOWNERS`.
- Twelve months without a review, pull request, issue or comment here takes a maintainer off the list. They can come
  back through the steps above.
- A Code of Conduct breach is handled as its enforcement steps describe. Otherwise, a maintainer is removed only by a
  majority of the other maintainers.

## Changing this document

A pull request changes it, under the same rule as adding a maintainer: a majority of the maintainers approve and none
objects.
