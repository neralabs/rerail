import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, realpathSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { defaultRerailDatabasePath, openRerailDatabase, recordCheckout, repositoryIdentityLocation } from "../../src/engine/index.js";
import { readPresentation } from "../../src/presentation/session-presentation.js";
import { closeNativeTestDaemons, startNativeTestDaemon } from "../support/native-daemon.js";
import { removeFolder } from "../support/platform/remove-folder.js";

describe.runIf(process.platform === "win32")("On Windows, the status line in a folder outside Git that Rerail 0.1.4 stopped through a mapped drive letter", () => {
  const made: { letters: string[]; roots: string[] } = { letters: [], roots: [] };
  afterEach(async () => {
    await closeNativeTestDaemons();
    for (const letter of made.letters.splice(0)) execFileSync("subst", [letter, "/d"], { stdio: "ignore" });
    await Promise.all(made.roots.splice(0).map(root => removeFolder(root)));
  });

  /**
   * A set-up, started repository of one folder outside Git, stored as `…\share\Stopped` and reached through a free drive
   * letter mapped to `…\share`, holding the checkout row Rerail 0.1.4's store writes when `rerail stop` runs there: keyed
   * by the JS realpath of the folder as spelt, which keeps the letter, and stopped. This build recorded no row of its own.
   */
  async function stoppedBy014(): Promise<{ root: string; spelled: string }> {
    const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-status-earlier-")));
    made.roots.push(root);
    mkdirSync(join(root, "share", "Stopped"), { recursive: true });
    const letter = [..."QRSTUVWXYZ"].map(name => `${name}:`).find(name => !existsSync(`${name}\\`));
    if (letter === undefined) throw new Error("no free drive letter");
    execFileSync("subst", [letter, join(root, "share")]);
    made.letters.push(letter);
    const spelled = `${letter}\\Stopped`;
    expect(repositoryIdentityLocation(spelled).git).toBe(false);
    const database = openRerailDatabase(defaultRerailDatabasePath());
    try {
      const { repositoryKey, checkoutPath } = recordCheckout(database, spelled);
      expect(realpathSync(spelled)).not.toBe(checkoutPath);
      database.$client.prepare("UPDATE repositories SET started = 1, configured_at = 1 WHERE repository_key = ?").run(repositoryKey);
      database.$client.prepare("UPDATE checkouts SET checkout_path = ?, started = 0 WHERE checkout_path = ?").run(realpathSync(spelled), checkoutPath);
    } finally {
      database.$client.close();
    }
    return { root, spelled };
  }

  it("draws nothing for a session the daemon has not recorded, as the hook there records none", async () => {
    const { root, spelled } = await stoppedBy014();
    await startNativeTestDaemon(root);
    const footer = await readPresentation({ sessionId: "a session the daemon has not recorded", host: "claude-code", cwd: spelled });
    expect(footer).toMatchObject({ status: "not-on-here", text: "", repositoryState: "stopped" });
  });
});
