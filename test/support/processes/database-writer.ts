import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { resolve } from "node:path";
import { createInterface } from "node:readline";
import { pathToFileURL } from "node:url";

/** Another process holding the Rerail database through Rerail's own opener, as the CLI's commands do. */
export interface DatabaseWriter {
  /** Records the repository `key` there in one commit; resolves once the process says the commit returned. */
  commit(key: string): Promise<void>;
  /** Closes its database and waits for it to end. */
  close(): Promise<void>;
}

const engine = pathToFileURL(resolve(import.meta.dirname, "..", "..", "..", "src", "engine", "index.ts")).href;

/** Opens the database, says "ready", then commits one repository per line it reads, saying "done <key>" after each. */
const program = `import { openRerailDatabase, repositories } from ${JSON.stringify(engine)};
import { createInterface } from "node:readline";
const database = openRerailDatabase(process.argv[1]);
process.stdout.write("ready\\n");
for await (const key of createInterface({ input: process.stdin })) {
  database.insert(repositories).values({ repositoryKey: key, displayName: key, configuredAt: null, started: null, createdAt: 1, updatedAt: 1 }).run();
  process.stdout.write("done " + key + "\\n");
}
database.$client.close();
`;

/**
 * Starts a {@link DatabaseWriter} on the database at `databasePath`. Its commits run `synchronous = NORMAL`, as every
 * commit through `openRerailDatabase` does: each writes the write-ahead log and publishes it to readers without waiting
 * for the disk, and on Windows no watch of the folder hears it until the writer closes the log (`databaseChangeFeed`).
 */
export async function startDatabaseWriter(databasePath: string): Promise<DatabaseWriter> {
  const child: ChildProcess = spawn(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program, databasePath], {
    cwd: resolve(import.meta.dirname, "..", "..", ".."),
    stdio: ["pipe", "pipe", "inherit"],
    windowsHide: true,
  });
  const waiting = new Map<string, () => void>();
  let ready!: () => void;
  const started = new Promise<void>(resolveStarted => { ready = resolveStarted; });
  createInterface({ input: child.stdout! }).on("line", line => {
    if (line === "ready") ready();
    else waiting.get(line.slice("done ".length))?.();
  });
  const exited = once(child, "exit").then(([code]) => `ended (${String(code)})`);
  const first = await Promise.race([started.then(() => "ready"), exited]);
  if (first !== "ready") throw new Error(`the writer ${first} before it opened the database`);
  return {
    commit: key => new Promise<void>(resolveCommit => {
      waiting.set(key, resolveCommit);
      child.stdin!.write(`${key}\n`);
    }),
    close: async () => {
      if (child.exitCode !== null) return;
      const ended = once(child, "exit");
      child.stdin!.end();
      await ended;
    },
  };
}
