/** What `rerail status` says of the folder it runs in. */
import { spawnSync } from "node:child_process";
import { mkdtempSync, realpathSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { afterAll, describe, expect, it } from "vitest";
import { PACKAGE_ROOT, RERAIL_DATABASE_FILE } from "../../../src/engine/index.js";
import { removeFolderSync } from "../../support/platform/remove-folder.js";

// By its file URL: the run starts outside the package, where a bare `tsx` does not resolve.
const TSX = pathToFileURL(createRequire(import.meta.url).resolve("tsx")).href;

describe("rerail status", () => {
  const home = mkdtempSync(join(tmpdir(), "rerail-status-home-"));
  // A folder outside every Git repository, as its real path: the message names `process.cwd()`.
  const folder = realpathSync(mkdtempSync(join(tmpdir(), "rerail-status-folder-")));

  afterAll(() => {
    for (const directory of [home, folder]) removeFolderSync(directory);
  });

  it("outside a Git repository, says Rerail works in one and how to start it in one from here", () => {
    // A database makes the install read as set up, so status goes on to the folder instead of saying it is not installed.
    writeFileSync(join(home, RERAIL_DATABASE_FILE), "");
    expect(spawnSync("git", ["-C", folder, "rev-parse", "--git-dir"]).status).not.toBe(0);

    const run = spawnSync(process.execPath, ["--import", TSX, join(PACKAGE_ROOT, "src/cli/cli.ts"), "--rerail-managed-child", "status"], {
      cwd: folder,
      encoding: "utf8",
      env: { ...process.env, RERAIL_HOME: home, HOME: home, RERAIL_AUTOSTART: "0" },
    });

    expect(run.stdout.split("\n")[0]).toBe(
      `No Git repository here (${folder}). Rerail works in a Git repository: run rerail status inside one, or start Rerail in one with rerail start --repo <folder>.`,
    );
  }, 60_000);
});
