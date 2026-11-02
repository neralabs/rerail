<!--
From a fork, this pull request is a proposal: it gets no CI, and a maintainer who takes it ports it to a branch here
and closes this one with a link. Contributors, which Nera Software employees are by default, open pull requests from branches here:
https://github.com/neralabs/rerail/blob/main/CONTRIBUTING.md#how-contribution-works
-->

Fixes #

## What and why

## How you tested it

## Checklist

<!-- From https://github.com/neralabs/rerail/blob/main/CONTRIBUTING.md#before-you-open-a-pull-request -->

- [ ] It links the issue where the approach was agreed, or it's a change that needs none (typo, doc fix, a bug or rule
      fix with its test)
- [ ] `npm run check` and `npm test` pass (`npm run build` first)
- [ ] `npm run check:licenses` and `reuse lint` pass
- [ ] Tests cover the behavior this changes, and the docs that describe it are updated
- [ ] A change to a pack under `packs/` adds a changeset
- [ ] A change to what the core package ships adds an entry under `## Unreleased` in `CHANGELOG.md`
- [ ] The rule-file format, the API and installed packs keep working, or this says above what changes and how
      people upgrade
