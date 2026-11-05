/** Which command answers each word of `rerail`, told apart by the message it refuses a bad argument with. */
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, describe, expect, it } from "vitest";
import { PACKAGE_ROOT } from "../../src/engine/index.js";

describe("rerail's command dispatch", () => {
  const home = mkdtempSync(join(tmpdir(), "rerail-dispatch-"));
  afterAll(() => rmSync(home, { recursive: true, force: true }));

  it.each([
    [["stop", "--nope"], "Unknown stop option: --nope"],
    // `--json` and `--relay` are `start`'s alone.
    [["stop", "--json"], "Unknown stop option: --json"],
    [["stop", "--relay"], "Unknown stop option: --relay"],
    [["start", "--nope"], "Unknown start option: --nope"],
    [["state", "--nope"], "Unknown state option: --nope"],
    [["rules", "evidence", "--nope"], "Unknown rules evidence option: --nope"],
    [["rules", "approve", "--nope"], "Unknown rules approve option: --nope"],
    [["rules", "test", "--nope"], "rules test needs a draft file; rerail rules test --help shows its format"],
    [["rules", "add", "--nope"], "rules add needs a draft file; rerail rules add --help shows its format"],
    [["rules", "nope"], "Unknown rules action: nope"],
    [["seed", "--nope"], "Unknown seed option: --nope"],
    [["show-me", "--nope"], "Unknown show-me option: --nope"],
    [["support-bundle", "--nope"], "Unknown support-bundle option: --nope"],
    [["packs", "nope"], "Unknown packs command: nope"],
    [["reset", "--nope"], "Unknown reset option: --nope"],
    [["feedback"], "rerail feedback needs a rule id and your reason"],
    [["scrub", "--nope"], "Unknown scrub option: --nope"],
    [["setup", "--nope"], "Unknown setup option: --nope"],
    [["install", "--nope"], "Unknown install option: --nope"],
    [["install", "--refresh", "--remove"], "install --refresh takes no other options"],
    [["install", "--delete-data"], "install --delete-data goes with --remove"],
    [["scan", "--nope"], "Unknown option: --nope"],
    [["scan", "--pack"], "--pack requires a value"],
    [["scan", "--paths-from"], "--paths-from requires a value"],
    [["scan", "--scope", "repository", "--base", "x"], "--base applies only to --scope git-diff"],
    [["settings", "--nope"], "Unknown option: --nope"],
    [["nope"], "Unknown command: nope"],
  ])("%j says so and exits 1", (args, message) => {
    const run = spawnSync(process.execPath, ["--import", "tsx", join(PACKAGE_ROOT, "src/cli/cli.ts"), "--rerail-managed-child", ...args], {
      cwd: PACKAGE_ROOT,
      encoding: "utf8",
      env: { ...process.env, RERAIL_HOME: home, HOME: home, RERAIL_AUTOSTART: "0" },
    });
    expect(run.stderr.split("\n")[0]).toBe(`rerail: ${message}`);
    expect(run.status).toBe(1);
  }, 60_000);
});
