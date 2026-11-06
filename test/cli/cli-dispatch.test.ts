/** Which command answers each word of `rerail`, told apart by everything it writes when it refuses a bad argument. */
import { spawnSync, type SpawnSyncReturns } from "node:child_process";
import { mkdtempSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { PACKAGE_ROOT } from "../../src/engine/index.js";

const REMOTE_USAGE = "Usage: rerail remote add <url> [--name <name>] [--label <machine label>] (--token-stdin | --token-file <file>) [--no-content] | list | remove <name|url> | status | push | exclude [--repo <dir>] | include [--repo <dir>]";
const SESSION_USAGE = "Usage: rerail session [<host> [<session-id>]] [--open]; inside Codex, rerail session uses CODEX_THREAD_ID";

describe("rerail's command dispatch", () => {
  const home = mkdtempSync(join(tmpdir(), "rerail-dispatch-"));
  // `scan` asks for a Git checkout before it reads its other options, so its cases bring their own: these tests must
  // not depend on the package being run from a checkout.
  const checkout = realpathSync(mkdtempSync(join(tmpdir(), "rerail-dispatch-checkout-")));
  const folder = realpathSync(mkdtempSync(join(tmpdir(), "rerail-dispatch-folder-")));
  let usage: string;

  const rerail = (args: readonly string[], input = ""): SpawnSyncReturns<string> =>
    spawnSync(process.execPath, ["--import", "tsx", join(PACKAGE_ROOT, "src/cli/cli.ts"), "--rerail-managed-child", ...args], {
      cwd: PACKAGE_ROOT,
      encoding: "utf8",
      input,
      env: { ...process.env, RERAIL_HOME: home, HOME: home, RERAIL_AUTOSTART: "0" },
    });

  beforeAll(() => {
    expect(spawnSync("git", ["init", "--quiet", checkout]).status).toBe(0);
    const help = rerail(["help"]);
    expect(help.status).toBe(0);
    usage = help.stdout;
    expect(usage).toMatch(/^Usage:\n {2}rerail setup /u);
  }, 60_000);

  afterAll(() => {
    for (const directory of [home, checkout, folder]) rmSync(directory, { recursive: true, force: true });
  });

  /** Refused with `message` and nothing else, or with the whole usage after it. */
  const refuses = (args: readonly string[], message: string, withUsage: boolean): void => {
    const run = rerail(args);
    expect(run.stderr).toBe(withUsage ? `rerail: ${message}\n\n${usage}` : `rerail: ${message}\n`);
    expect(run.stdout).toBe("");
    expect(run.status).toBe(1);
  };

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
    [["nope"], "Unknown command: nope"],
  ])("%j says so, prints the usage and exits 1", (args, message) => {
    refuses(args, message, true);
  }, 60_000);

  it.each([
    [["setup", "--nope"], "Unknown setup option: --nope"],
    [["install", "--nope"], "Unknown install option: --nope"],
    [["install", "--refresh", "--remove"], "install --refresh takes no other options"],
    [["install", "--delete-data"], "install --delete-data goes with --remove"],
    [["scan", "--nope"], "Unknown option: --nope"],
    [["scan", "--pack"], "--pack requires a value"],
    [["scan", "--paths-from"], "--paths-from requires a value"],
    [["settings", "--nope"], "Unknown option: --nope"],
    [["remote", "nope"], REMOTE_USAGE],
    [["session", "nope"], SESSION_USAGE],
    [["update", "nope"], "Unknown update action: nope"],
  ])("%j says so, and only that, and exits 1", (args, message) => {
    refuses(args, message, false);
  }, 60_000);

  it("scan asks for a Git checkout first, then refuses --base outside --scope git-diff", () => {
    refuses(["scan", "--repo", folder, "--scope", "repository", "--base", "x"], `${folder} is not inside a Git checkout`, false);
    refuses(["scan", "--repo", checkout, "--scope", "repository", "--base", "x"], "--base applies only to --scope git-diff", false);
  }, 60_000);

  // The one case that runs the built `dist/` (the event stream is a child process), so it needs `npm run build`.
  it("events answers on the event protocol, whatever its arguments", () => {
    const run = rerail(["events", "--nope"], "nope\n");
    expect(run.stdout).toBe(`${JSON.stringify({ eventId: null, error: { code: "unsupported-version", message: "Expected RERAIL-EVENTS/1" } })}\n`);
    expect(run.stderr).toBe("");
    expect(run.status).toBe(0);
  }, 60_000);
});
