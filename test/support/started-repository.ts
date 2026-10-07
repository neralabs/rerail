import { defaultRerailDatabasePath, openRerailDatabase } from "../../src/database.js";
import { startRepository, stopRepository } from "../../src/repository-start.js";
import { resolveRepositoryContext } from "../../src/scoped-rule-catalog.js";

function withRepository(workspace: string, databasePath: string, change: typeof startRepository): string {
  const { repositoryId } = resolveRepositoryContext(workspace);
  const database = openRerailDatabase(databasePath);
  try {
    change(database, repositoryId);
  } finally {
    database.$client.close();
  }
  return repositoryId;
}

/** Rerail is off until started; mark `workspace`'s repository started, as `rerail start` leaves it. */
export function startTestRepository(workspace: string, databasePath = defaultRerailDatabasePath()): string {
  return withRepository(workspace, databasePath, startRepository);
}

/** Mark `workspace`'s started repository stopped, as `rerail stop` leaves it. */
export function stopTestRepository(workspace: string, databasePath = defaultRerailDatabasePath()): string {
  return withRepository(workspace, databasePath, stopRepository);
}

/** Start a repository known only by id, for fixtures that inject their own repository context. */
export function startTestRepositoryById(repositoryId: string, databasePath: string): void {
  const database = openRerailDatabase(databasePath);
  try {
    startRepository(database, repositoryId);
  } finally {
    database.$client.close();
  }
}
