import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, realpathSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { defaultRerailDatabasePath, openRerailDatabase, recordCheckout, repositoryIdentityLocation } from "../../src/engine/index.js";
import { SqliteSessionStore } from "../../src/runtime/index.js";
import { createRerailApi, DatabaseApiStores, operations } from "../../src/api/index.js";
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
   * `target` reached through a free drive letter. Vitest runs test files at once and every file shares the user's drive
   * letters, so this file takes only X to Z (the start state's earlier-spelling tests take U to W, and the registry's
   * tests count on Q being free), and the next one where `subst` refuses a letter.
   */
  function substLetter(target: string): string {
    for (const name of "XYZ") {
      const letter = `${name}:`;
      if (existsSync(`${letter}\\`)) continue;
      try {
        execFileSync("subst", [letter, target], { stdio: "ignore" });
        return letter;
      } catch {
        // Taken since the check above: the next one.
      }
    }
    throw new Error("no free drive letter from X to Z");
  }

  /**
   * A set-up, started repository of one folder outside Git, stored as `…\share\<name>` and reached through a free drive
   * letter mapped to `…\share`, holding the checkout row Rerail 0.1.4's store writes when `rerail stop` runs there: keyed
   * by the JS realpath of the folder as spelt, which keeps the letter, and stopped. This build recorded no row of its own.
   * Each test names its own folder: the cases share one database, and a letter freed by one is taken by the next.
   */
  async function stoppedBy014(name = "Stopped"): Promise<{ root: string; spelled: string }> {
    const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-status-earlier-")));
    made.roots.push(root);
    mkdirSync(join(root, "share", name), { recursive: true });
    const letter = substLetter(join(root, "share"));
    made.letters.push(letter);
    const spelled = `${letter}\\${name}`;
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

  /** The daemon's `/api/presentation` over the home's database, as `repository`'s start state and the answer's status. */
  async function askedApart(test: (state: (query: Record<string, string>) => Promise<[number, string | undefined]>) => Promise<void>): Promise<void> {
    const path = defaultRerailDatabasePath();
    const stores = new DatabaseApiStores(path, {
      health: () => ({ pid: process.pid, database: { path, exists: true }, native: { ok: true } }), origin: () => "http://127.0.0.1:4737",
    });
    try {
      const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
      await test(async query => {
        const response = await app.request(`/api/presentation?${new URLSearchParams({ host: "claude-code", ...query })}`);
        return [response.status, ((await response.json()) as { repositoryState?: string }).repositoryState];
      });
    } finally {
      stores.close();
    }
  }

  it("reads the stop at the daemon only from the spelling apart from `repository`, and asked without one or with one it cannot use, as before", async () => {
    const { root, spelled } = await stoppedBy014("Asked Apart");
    const repository = realpathSync.native(spelled);
    await askedApart(async state => {
      expect(await state({ repository, spelling: spelled })).toEqual([200, "stopped"]);
      expect(await state({ repository })).toEqual([200, "started"]);
      // Relative to the daemon's own folder on that drive, so no spelling of the session's folder.
      expect(await state({ repository, spelling: `${spelled.slice(0, 2)}Asked Apart` })).toEqual([200, "started"]);
      expect(await state({ repository, spelling: "Q:\\a drive no letter names\\Asked Apart" })).toEqual([200, "started"]);
      expect(await state({ repository: join(root, "share"), spelling: `${spelled.slice(0, 2)}\\` })).toEqual([200, "stopped"]);
    });
  });

  it("reads a session's folder below the one asked about without the spelling, which names the folder asked about", async () => {
    const { spelled } = await stoppedBy014("Asked Above");
    const repository = realpathSync.native(spelled);
    const below = join(repository, "Below");
    mkdirSync(below);
    const sessions = new SqliteSessionStore(defaultRerailDatabasePath());
    try {
      sessions.openSession({ host: "claude-code", sessionId: "a session below", cwd: below });
    } finally {
      sessions.close();
    }
    const database = openRerailDatabase(defaultRerailDatabasePath());
    try {
      // Recorded as of the same repository as the folder above, so only which key is read tells the two apart.
      const { checkoutPath } = recordCheckout(database, below);
      const above = database.$client.prepare("SELECT repository_key FROM checkouts WHERE checkout_path = ?").get(realpathSync(spelled)) as { repository_key: string };
      database.$client.prepare("UPDATE checkouts SET repository_key = ? WHERE checkout_path = ?").run(above.repository_key, checkoutPath);
    } finally {
      database.$client.close();
    }
    await askedApart(async state => {
      expect(await state({ repository, spelling: spelled, sessionId: "a session below" })).toEqual([200, "started"]);
      expect(await state({ repository, spelling: spelled })).toEqual([200, "stopped"]);
    });
  });
});
