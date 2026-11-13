import { isAbsolute } from "node:path";
import { and, asc, desc, eq, inArray, or } from "drizzle-orm";
import type { RerailHost } from "../engine/adapter-contract.js";
import { LazyReadonlyRerailDatabase, openRerailDatabase, type RerailDatabase } from "../engine/storage/database.js";
import {
  notAnEarlierSpelling, pathKeySpellings, presentationPreferences, type PresentationPreference, type PresentationPresence,
} from "../engine/storage/schema.js";
import { physicalPath } from "../engine/catalog/checkout-control.js";

/** Match project path identity even through symlinks or not-yet-created descendants. */
function repositoryKey(repository: string): string {
  if (repository === "") return "";
  if (!isAbsolute(repository)) throw new Error("Presentation repositories must be absolute paths");
  return physicalPath(repository);
}

/** Database-local user settings. The daemon alone owns the writable store. */
export class SqlitePresentationPreferenceStore {
  readonly #reader: LazyReadonlyRerailDatabase;
  #writer: RerailDatabase | undefined;

  constructor(path: string) {
    this.#reader = new LazyReadonlyRerailDatabase(path);
  }

  get #database(): RerailDatabase {
    this.#writer ??= openRerailDatabase(this.#reader.path);
    return this.#writer;
  }

  /** Every preference, each folder under the key this build gives it: a copy kept for an earlier build is left out. */
  list(): PresentationPreference[] {
    return this.#reader.open()?.select().from(presentationPreferences).where(notAnEarlierSpelling(presentationPreferences.repository))
      .orderBy(asc(presentationPreferences.repository), asc(presentationPreferences.host)).all() ?? [];
  }

  upsert(preference: PresentationPreference): PresentationPreference {
    return this.#database.insert(presentationPreferences)
      .values({ ...preference, repository: repositoryKey(preference.repository) })
      .onConflictDoUpdate({
        target: [presentationPreferences.repository, presentationPreferences.host],
        set: { presence: preference.presence },
      })
      .returning().get()!;
  }

  /**
   * Removes the folder's preference for `host`, and the copies kept under the folder's earlier spellings for an earlier
   * build (`pathKeySpellings`), so no build shows it after; the count is of this build's own.
   */
  remove(repository: string, host: PresentationPreference["host"]): number {
    const key = repositoryKey(repository);
    const earlier = this.#database.select({ stored: pathKeySpellings.stored }).from(pathKeySpellings).where(eq(pathKeySpellings.current, key));
    return this.#database.delete(presentationPreferences)
      .where(and(or(eq(presentationPreferences.repository, key), inArray(presentationPreferences.repository, earlier)), eq(presentationPreferences.host, host)))
      .returning({ repository: presentationPreferences.repository }).all().filter(row => row.repository === key).length;
  }

  effective(repository: string, host: RerailHost): PresentationPresence {
    const key = repositoryKey(repository);
    return this.#reader.open()?.select({ presence: presentationPreferences.presence }).from(presentationPreferences)
      .where(and(inArray(presentationPreferences.repository, [key, ""]), inArray(presentationPreferences.host, [host, ""])))
      .orderBy(desc(eq(presentationPreferences.repository, key)), desc(eq(presentationPreferences.host, host)))
      .limit(1).get()?.presence ?? "compact";
  }

  close(): void {
    this.#reader.close();
    this.#writer?.$client.close();
  }
}
