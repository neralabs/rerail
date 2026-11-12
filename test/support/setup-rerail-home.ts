import { afterAll } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { stopProcess, strandedDaemons } from "./processes/stranded-daemons.js";

const priorRerailHome = process.env.RERAIL_HOME;
const priorContentStorePath = process.env.RERAIL_CONTENT_STORE_PATH;
const rerailHome = mkdtempSync(join(tmpdir(), "rerail-vitest-home-"));
const contentStoreDirectory = mkdtempSync(join(tmpdir(), "rerail-vitest-store-"));

process.env.RERAIL_HOME = rerailHome;
// A test run is a dev build, whose store would otherwise be this checkout's `.rerail/content-v1.sqlite`, shared by
// every test file; each file gets its own (`contentStorePath` honours the override).
process.env.RERAIL_CONTENT_STORE_PATH = join(contentStoreDirectory, "content.sqlite");

afterAll(async () => {
  try {
    const stranded = strandedDaemons();
    if (stranded.length > 0) {
      for (const daemon of stranded) await stopProcess(daemon.pid);
      throw new Error(`Test file left ${stranded.length} Rerail daemon(s) running (stopped now): ${stranded.map(daemon => `pid ${daemon.pid} in ${daemon.home}`).join(", ")}`);
    }
  } finally {
    if (priorRerailHome === undefined) delete process.env.RERAIL_HOME;
    else process.env.RERAIL_HOME = priorRerailHome;
    if (priorContentStorePath === undefined) delete process.env.RERAIL_CONTENT_STORE_PATH;
    else process.env.RERAIL_CONTENT_STORE_PATH = priorContentStorePath;
    rmSync(rerailHome, { recursive: true, force: true });
    rmSync(contentStoreDirectory, { recursive: true, force: true });
  }
});
