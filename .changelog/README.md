# Changelog entries

Core's release notes wait here for the next release, one file per entry, so pull requests that add one never edit the
same lines of `CHANGELOG.md`. A pull request that changes what the core package ships adds a `.md` file here named for
its change type and what it changes (`fixed-dashboard-toast-titles.md`); its text is the entry as it should read,
without the leading `- `. A pull request that changes an entry not released yet edits its file. Rule packs take a
changeset instead ([.changeset/README.md](../.changeset/README.md)).

The name starts with one of the change types of [Keep a Changelog](https://keepachangelog.com/en/1.1.0/):

| Name starts with | For |
|---|---|
| `added-` | something new |
| `changed-` | something that works differently now |
| `deprecated-` | something that still works and will be removed |
| `removed-` | something that is gone |
| `fixed-` | a bug fixed |
| `security-` | a vulnerability closed |

An entry that does more than one of these takes the type that matters most to the person upgrading.

An entry says what changed for the person upgrading, in plain language, in a sentence or two (at most 60 words), and
leaves out commit hashes, ticket ids, pull request numbers and repository-internal maintenance; CI fails a pull request
whose entry says otherwise or starts with a `- ` of its own, while any entry here has no type, or when it writes an
entry into `CHANGELOG.md`'s first section, `## Unreleased` or the last release's
(`scripts/release/core-changelog-check.mjs`).

The **Release** pull request (`scripts/release/release-version.mjs`) writes the release's section of `CHANGELOG.md`:
the lead paragraph written under `## Unreleased`, then every entry here under a `### <type>` heading, in the order of
the table and leaving out types with no entry. Within a type entries go in file-name order, so entries named for the
same area sit together. It deletes the files. The lead is the one part of the notes still written in `CHANGELOG.md`
([release notes](../docs/release.md#release)).
