# Changelog entries

Core's release notes wait here for the next release, one file per entry, so pull requests that add one never edit the
same lines of `CHANGELOG.md`. A pull request that changes what the core package ships adds a `.md` file here named for
what it changes (`dashboard-toast-titles.md`); its text is the entry as it should read, without the leading `- `. A
pull request that changes an entry not released yet edits its file. Rule packs take a changeset instead
([.changeset/README.md](../.changeset/README.md)).

An entry says what changed for the person upgrading, in plain language, in a sentence or two (at most 60 words), and
leaves out commit hashes, ticket ids, pull request numbers and repository-internal maintenance; CI fails a pull request
whose entry says otherwise or starts with a `- ` of its own, or that writes an entry into `CHANGELOG.md`'s first
section, `## Unreleased` or the last release's (`scripts/release/core-changelog-check.mjs`).

The **Release** pull request (`scripts/release/release-version.mjs`) writes the release's section of `CHANGELOG.md`:
the lead paragraph written under `## Unreleased`, then every entry here in file-name order, so entries named for the
same area sit together; it deletes the files. The lead is the one part of the notes still written in `CHANGELOG.md`
([release notes](../docs/release.md#release)).
