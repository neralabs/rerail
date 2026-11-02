// The changelog generator `changeset version` writes each pack's CHANGELOG.md with (.changeset/config.json,
// `changelog`): a changeset's summary becomes one entry, as the contributor wrote it, with its paragraphs kept under
// the bullet. The default generator (@changesets/cli/changelog) led each entry with its commit hash, which says
// nothing to the person upgrading (docs/release.md, "Release notes"); the hash is in git. The packs depend on no
// other workspace package, so a dependency bump gets no line.
//
// Resolved from .changeset/ (`../scripts/release/changeset-changelog.mjs`), as changesets resolves a relative path.

export default {
  /** The changelog entry of `changeset`: `- <first line>`, its later lines indented under it. */
  getReleaseLine: async (changeset) => {
    const [first, ...rest] = changeset.summary.trim().split("\n");
    return [`- ${first}`, ...rest.map((line) => (line.trim() === "" ? "" : `  ${line}`))].join("\n");
  },
  getDependencyReleaseLine: async () => "",
};
