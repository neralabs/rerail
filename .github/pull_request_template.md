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
- [ ] Tests cover the behavior this changes
- [ ] Docs updated? A change to commands, coding agents or setup updates `README.md` and `docs/` in this pull request,
      and `context7.json` where it names what changed
- [ ] A feature that enters, graduates from or leaves internal testing updates `docs/internal-testing.md`
- [ ] It carries only approved Rerail context: the code, the docs, fixtures, comments, the commits, this description
      and any agent session trace hold no private project names, customer names, old product names (such as "Tempo"),
      internal tickets or URLs, private links (Notion, Slack, Orca shares, signed attachment URLs), personal paths or
      research participant details (`npm run check:internal-refs` covers tracked files)
- [ ] A change to what a package does keeps its npm page current: its `README.md`, and `keywords` and `repository` in
      its `package.json`
- [ ] A change to a pack under `packs/` adds a changeset
- [ ] A change to what the core package ships adds an entry file in `.changelog/`, named for its change type
- [ ] The rule-file format, the API and installed packs keep working, or this says above what changes and how
      people upgrade
