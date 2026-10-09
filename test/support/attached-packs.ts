import { openRerailDatabase } from "../../src/database.js";
import { lookupCheckout } from "../../src/repository-registry.js";

/** The rule packs added to `workspace`'s repository: the pack id, or `id@folder` for a pack added in one folder. */
export function attachedPacks(workspace: string, databasePath: string): string[] {
  const database = openRerailDatabase(databasePath, { readonly: true });
  try {
    const repositoryKey = lookupCheckout(database, workspace)?.repositoryKey;
    if (repositoryKey === undefined) return [];
    return database.$client
      .prepare("SELECT source_id, folder FROM rule_attachments WHERE repository_key = ? AND source_kind = 'pack' ORDER BY source_id, folder")
      .all(repositoryKey)
      .map(row => {
        const { source_id: id, folder } = row as { source_id: string; folder: string };
        return folder === "" ? id : `${id}@${folder}`;
      });
  } finally {
    database.$client.close();
  }
}
