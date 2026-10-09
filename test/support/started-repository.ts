import { defaultRerailDatabasePath, openRerailDatabase } from "../../src/database.js";
import type { RepositoryStartState } from "../../src/repository-model.js";
import { recordCheckout } from "../../src/repository-registry.js";
import { setRepositoryStarted, stopRerail } from "../../src/repository-start.js";

/** Rerail is off until started; record `workspace`'s checkout and start its repository as a whole, not set up. */
export function startTestRepository(workspace: string, databasePath = defaultRerailDatabasePath()): string {
  const database = openRerailDatabase(databasePath);
  try {
    const { repositoryKey } = recordCheckout(database, workspace);
    setRepositoryStarted(database, repositoryKey, true);
    return repositoryKey;
  } finally {
    database.$client.close();
  }
}

/** Stop `workspace`'s checkout, as `rerail stop` leaves it; returns the state before. */
export function stopTestRepository(workspace: string, databasePath = defaultRerailDatabasePath()): RepositoryStartState {
  return stopRerail(workspace, { lane: "terminal", say: () => {} }, { databasePath });
}
