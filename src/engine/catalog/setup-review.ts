/**
 * Set-up's offer of the dashboard's review, kept per repository in the database, which the CLI and the daemon both
 * write. Set-up records it when it offers the review link in a repository that is not set up. The set-up the person
 * then makes in the dashboard keeps it, so set-up carries on at its confirm question. Any other set-up (`rerail rules
 * add`, a coding agent, `rules import`), a start and a stop delete it, so a repository set up some other way is never
 * taken for one the review set up.
 */
import { eq, sql } from "drizzle-orm";
import type { RerailDatabase } from "../storage/database.js";
import type { RepositoryKey } from "./repository-model.js";
import { repositories, setupReviews } from "../storage/schema.js";

type Reader = Pick<RerailDatabase, "select" | "get">;
type Deleter = Pick<RerailDatabase, "delete">;

/** Set-up offered the review in `repositoryKey`; nothing is recorded once the repository is set up. */
export function offerSetupReview(database: RerailDatabase, repositoryKey: RepositoryKey, now = Date.now()): void {
  database.transaction(tx => {
    const repository = tx.select({ configuredAt: repositories.configuredAt }).from(repositories)
      .where(eq(repositories.repositoryKey, repositoryKey)).get();
    if (repository === undefined || repository.configuredAt !== null) return;
    tx.insert(setupReviews).values({ repositoryKey, offeredAt: now }).onConflictDoNothing().run();
  }, { behavior: "immediate" });
}

/**
 * Whether set-up's offer of the review still stands in `repositoryKey`. A database from before the offer was kept has
 * no table for it until a writer opens it (read-only opens apply no schema), and no offer either.
 */
export function setupReviewOffered(db: Reader, repositoryKey: RepositoryKey): boolean {
  const kept = db.get<{ name: string } | undefined>(sql`SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'setup_reviews'`);
  if (kept === undefined) return false;
  return db.select({ key: setupReviews.repositoryKey }).from(setupReviews)
    .where(eq(setupReviews.repositoryKey, repositoryKey)).get() !== undefined;
}

/** Withdraw set-up's offer of the review in `repositoryKey`, if it made one. */
export function clearSetupReview(db: Deleter, repositoryKey: RepositoryKey): void {
  db.delete(setupReviews).where(eq(setupReviews.repositoryKey, repositoryKey)).run();
}
