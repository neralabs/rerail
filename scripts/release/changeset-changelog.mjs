// The changelog generator `changeset version` writes each pack's CHANGELOG.md with (.changeset/config.json,
// `changelog`): a changeset's summary becomes one entry, as the contributor wrote it, with its paragraphs kept under
// the bullet. The default generator (@changesets/cli/changelog) led each entry with its commit hash, which says
// nothing to the person upgrading (docs/release.md, "Release notes"); the hash is in git. A changeset with an empty
// summary bumps its packs and adds no entry: the Release pull request writes which rules a pack ships and how they
// start from its pack.json (pack-changelog.mjs), so a change with nothing else to tell the people who upgrade (a rule
// shipped, a README reworded) needs no words. The packs depend on no other workspace package, so a dependency bump
// gets no line.
//
// Resolved from .changeset/ (`../scripts/release/changeset-changelog.mjs`), as changesets resolves a relative path.

import { releaseNoteEntry } from "./release-notes.mjs";

export default {
  /** The changelog entry of `changeset`: `- <first line>`, its later lines indented under it (releaseNoteEntry); none for an empty summary. */
  getReleaseLine: async (changeset) => (changeset.summary.trim() === "" ? "" : releaseNoteEntry(changeset.summary)),
  getDependencyReleaseLine: async () => "",
};
