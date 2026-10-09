import { isAbsolute } from "node:path";
import { and, asc, desc, eq, inArray } from "drizzle-orm";
import type { RerailHost } from "./adapter-contract.js";
import { LazyReadonlyRerailDatabase, openRerailDatabase, type RerailDatabase } from "./database.js";
import { presentationPreferences, type PresentationPreference, type PresentationPresence } from "./schema.js";
import { physicalPath } from "./scoped-rule-catalog.js";

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

  list(): PresentationPreference[] {
    return this.#reader.open()?.select().from(presentationPreferences)
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

  remove(repository: string, host: PresentationPreference["host"]): number {
    return this.#database.delete(presentationPreferences)
      .where(and(eq(presentationPreferences.repository, repositoryKey(repository)), eq(presentationPreferences.host, host)))
      .returning({ repository: presentationPreferences.repository }).all().length;
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
