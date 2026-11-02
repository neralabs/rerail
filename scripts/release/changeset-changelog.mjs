// The changelog generator `changeset version` writes each pack's CHANGELOG.md with (.changeset/config.json,
// `changelog`): a changeset's summary becomes one entry, as the contributor wrote it, with its paragraphs kept under
// the bullet. The default generator (@changesets/cli/changelog) led each entry with its commit hash, which says
// nothing to the person upgrading (docs/release.md, "Release notes"); the hash is in git. The packs depend on no
// other workspace package, so a dependency bump gets no line.
//
// Resolved from .changeset/ (`../scripts/release/changeset-changelog.mjs`), as changesets resolves a relative path.

import { releaseNoteEntry } from "./release-notes.mjs";

export default {
  /** The changelog entry of `changeset`: `- <first line>`, its later lines indented under it (releaseNoteEntry). */
  getReleaseLine: async (changeset) => releaseNoteEntry(changeset.summary),
  getDependencyReleaseLine: async () => "",
};
