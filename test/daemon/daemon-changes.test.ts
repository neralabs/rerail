import { mkdtemp, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { RerailDaemon } from "../../src/daemon/daemon.js";
import { startPolling } from "../../ui/poll.js";
import { data, setScope } from "../../ui/state.js";
import { removeFolder } from "../support/platform/remove-folder.js";
import { startDatabaseWriter } from "../support/processes/database-writer.js";
import { drivenWatch } from "../support/platform/driven-watch.js";

const ACCESS = "test-access-capability";
const healthyNative = () => ({ ok: true, kernelVersion: "1" });
const originalRerailHome = process.env.RERAIL_HOME;
/** The page's visibility listeners, which the test calls when it hides the tab. */
let visibility: (() => void)[] = [];

/**
 * The dashboard's own fetch orchestration (`ui/poll.js`) against a real daemon, as a browser tab runs it: the page's
 * origin is the daemon's, its storage holds the capability the link gave it, and the tab shows.
 */
function openPage(daemon: RerailDaemon): { rendered: (until: () => boolean) => Promise<void>; nextRender: () => Promise<void> } {
  vi.stubGlobal("location", { origin: new URL(daemon.url).origin });
  vi.stubGlobal("localStorage", { getItem: (key: string) => (key === "rerail-access" ? ACCESS : null) });
  vi.stubGlobal("document", { hidden: false, addEventListener: (_: string, listener: () => void) => visibility.push(listener) });
  let waiting: (() => void) | undefined;
  setScope(null);
  startPolling(() => waiting?.());
  return {
    rendered: until => new Promise<void>(resolve => {
      waiting = () => { if (until()) resolve(); };
      waiting();
    }),
    nextRender: () => new Promise<void>(resolve => { waiting = resolve; }),
  };
}

describe("the dashboard against a running daemon", () => {
  let root: string;
  let daemon: RerailDaemon | undefined;

  afterEach(async () => {
    // Hide the tab: its stream ends, and its reads and timers with it.
    vi.stubGlobal("document", { hidden: true, addEventListener: () => {} });
    for (const listener of visibility.splice(0)) listener();
    await daemon?.close();
    vi.unstubAllGlobals();
    if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
    else process.env.RERAIL_HOME = originalRerailHome;
    await removeFolder(root);
  });

  it("shows a repository another process records before the daemon's next beat, where the 5 s poll showed it at its next read", async () => {
    root = await realpath(await mkdtemp(join(tmpdir(), "rerail-changes-daemon-")));
    process.env.RERAIL_HOME = join(root, "home");
    const databasePath = join(root, "rerail.sqlite");
    // The folder's notices come as the test says: the writer's -wal write heard before readers can see its commit, as
    // Linux tells it, on every system. The beat never comes within the test.
    const folder = drivenWatch();
    daemon = await RerailDaemon.start({
      version: "0.0.0-test", databasePath, workspaceRoot: root, port: 0, nativeHealth: healthyNative, accessToken: ACCESS,
      remoteSink: false, checkoutPruning: false, changeFeed: { watch: folder.watch, beatMs: 60 * 60_000 },
    });
    // Another process committing through Rerail's own opener, as the CLI's writes do. It opens first: opening creates
    // the database, a commit of its own that the page's stream must already hold.
    const writer = await startDatabaseWriter(databasePath);
    try {
      // The page's streams: a stall reopens one after two missed beats (12 s), and its first message reads.
      const realFetch = globalThis.fetch;
      let streams = 0;
      vi.stubGlobal("fetch", (input: string | URL, init?: RequestInit) => {
        if (new URL(input).pathname === "/api/changes") streams += 1;
        return realFetch(input, init);
      });
      const page = openPage(daemon);
      await page.rendered(() => data.loaded);

      const committed = writer.commit("remote:github.com/acme/web");
      folder.report(root, "rerail.sqlite-wal");
      await committed;

      await page.rendered(() => data.repositories.some(repository => repository.repositoryKey === "remote:github.com/acme/web"));

      // Shown on the stream the page had: no beat, no read of its own and no new stream brought it.
      expect(streams).toBe(1);
    } finally {
      await writer.close();
    }
  }, 30_000);
});
