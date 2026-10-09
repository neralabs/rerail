import { defaultRerailDatabasePath, openRerailDatabase, resolveRepositoryContext } from "../../src/engine/index.js";
import { startRepository, stopRepository } from "../../src/runtime/index.js";

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
