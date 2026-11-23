import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { RerailDaemon } from "../../src/daemon/index.js";
import { removeFolderSync } from "../support/platform/remove-folder.js";

const cleanups: Array<() => Promise<void> | void> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function daemonIn(prefix: string, options: Pick<Parameters<typeof RerailDaemon.start>[0], "updateChecks"> = {}): Promise<RerailDaemon> {
  const home = mkdtempSync(join(tmpdir(), prefix));
  cleanups.push(() => removeFolderSync(home));
  const daemon = await RerailDaemon.start({ version: "0.2.1", databasePath: join(home, "rerail.sqlite"), workspaceRoot: home, port: 0, ...options });
  cleanups.push(() => daemon.close());
  return daemon;
}

describe("/healthz.update", () => {
  it("is null for a daemon that does not check, as a checkout's and every test's daemon", async () => {
    const daemon = await daemonIn("rerail-update-none-");
    expect(daemon.health().update).toBeNull();
  });

  it("reports the update check's latest answer, and stops the check when the daemon closes", async () => {
    let answer: { latest: string; checkedAt: number } | null = null;
    let stops = 0;
    const daemon = await daemonIn("rerail-update-some-", { updateChecks: () => ({ newest: () => answer, stop: () => { stops += 1; } }) });
    expect(daemon.health().update).toBeNull();
    answer = { latest: "0.3.0", checkedAt: 1_795_238_148_885 };
    expect(daemon.health().update).toEqual({ latest: "0.3.0", checkedAt: 1_795_238_148_885 });

    await daemon.close();
    expect(stops).toBe(1);
  });
});
