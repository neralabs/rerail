/**
 * Set-up's offer of the dashboard's review, kept per repository in the database, which the CLI and the daemon both
 * write. Earlier versions' set-up recorded it when it offered the review link in a repository that is not set up; set-up
 * no longer offers the review. While such a repository is still not set up, set-up goes straight to its confirm question
 * there. Any set-up (the dashboard's included), a start and a stop delete it: a repository set up is just set up.
 */
import { eq, sql } from "drizzle-orm";
import type { RerailDatabase } from "../storage/database.js";
import type { RepositoryKey } from "./repository-model.js";
import { repositories, setupReviews } from "../storage/schema.js";

type Reader = Pick<RerailDatabase, "select" | "get">;
type Deleter = Pick<RerailDatabase, "delete">;

/** Set-up offered the review in `repositoryKey` (earlier versions); nothing is recorded once the repository is set up. */
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
