<!--
Pull requests come from maintainers and trusted contributors:
https://github.com/neralabs/rerail/blob/main/CONTRIBUTING.md#trusted-contributors
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
