/** Which command answers each word of `rerail`, told apart by everything it writes when it refuses a bad argument. */
import { spawnSync, type SpawnSyncReturns } from "node:child_process";
import { mkdtempSync, realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { experimentOffMessage, PACKAGE_ROOT, type ExperimentalFeature } from "../../src/engine/index.js";
import { removeFolderSync } from "../support/platform/remove-folder.js";
import { withExperiments } from "../support/environment/experiments.js";

withExperiments("HEADLESS_SCAN", "REMOTE_SINK", "EVENT_STREAM");

const REMOTE_USAGE = "Usage: rerail remote add <url> [--name <name>] [--label <machine label>] (--token-stdin | --token-file <file>) [--no-content] | list | remove <name|url> | status | push | exclude [--repo <dir>] | include [--repo <dir>]";
const SESSION_USAGE = "Usage: rerail session [<host> [<session-id>]] [--open]; inside Codex, rerail session uses CODEX_THREAD_ID";

describe("rerail's command dispatch", () => {
  const home = mkdtempSync(join(tmpdir(), "rerail-dispatch-"));
  // `scan` asks for a Git checkout before it reads its other options, so its cases bring their own: these tests must
  // not depend on the package being run from a checkout.
  const checkout = realpathSync(mkdtempSync(join(tmpdir(), "rerail-dispatch-checkout-")));
  const folder = realpathSync(mkdtempSync(join(tmpdir(), "rerail-dispatch-folder-")));
  // A checkout with a Git remote, which names its repository for `remote exclude` and `include`.
  const named = realpathSync(mkdtempSync(join(tmpdir(), "rerail-dispatch-named-")));
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
    expect(spawnSync("git", ["init", "--quiet", named]).status).toBe(0);
    expect(spawnSync("git", ["-C", named, "remote", "add", "origin", "https://github.com/acme/app.git"]).status).toBe(0);
    const help = rerail(["help"]);
    expect(help.status).toBe(0);
    usage = help.stdout;
    expect(usage).toMatch(/^Usage:\n {2}rerail setup /u);
  }, 60_000);

  afterAll(() => {
    for (const directory of [home, checkout, folder, named]) removeFolderSync(directory);
  });

  /** Refused with `message` and nothing else, or with the whole usage after it. */
  const refuses = (args: readonly string[], message: string, withUsage: boolean): void => {
    const run = rerail(args);
    expect(run.stderr).toBe(withUsage ? `rerail: ${message}\n\n${usage}` : `rerail: ${message}\n`);
    expect(run.stdout).toBe("");
    expect(run.status).toBe(1);
  };

  it.each([
    [["scan"], "HEADLESS_SCAN"],
    [["remote", "add", "https://remote.example"], "REMOTE_SINK"],
    [["remote", "push"], "REMOTE_SINK"],
    [["events"], "EVENT_STREAM"],
    [["engine", "dump", "--rule", "rule.json"], "ENGINE_TOOLS"],
    [["engine", "asm", "program.rir"], "ENGINE_TOOLS"],
    [["engine", "sql", "store.sqlite", "request.json"], "ENGINE_TOOLS"],
    [["activate", "--file", "profile.jws", "--trust-root", "roots.json"], "ACTIVATION_PROFILES"],
    [["profile", "select"], "ACTIVATION_PROFILES"],
    [["--profile", "acme", "status"], "ACTIVATION_PROFILES"],
  ] as Array<[string[], ExperimentalFeature]>)("%j refuses while its experiment is off, naming the variable", (args, feature) => {
    vi.stubEnv(`RERAIL_EXPERIMENTAL_${feature}`, "0");
    refuses(args, experimentOffMessage(feature), false);
  }, 60_000);

  it("merge-assist refuses while its experiment is off, naming the variable", () => {
    vi.stubEnv("RERAIL_EXPERIMENTAL_MERGE_ASSIST", "0");
    const run = rerail(["merge-assist"]);
    expect(run).toMatchObject({ stdout: "", stderr: `● ${experimentOffMessage("MERGE_ASSIST")}\n`, status: 1 });
  }, 60_000);

  it.each([
    [["profile"]],
    [["profile", "status"]],
  ])("%j works while activation profiles are off, so a profile stored earlier, which still applies, can be seen", args => {
    vi.stubEnv("RERAIL_EXPERIMENTAL_ACTIVATION_PROFILES", "0");
    const run = rerail(args);
    expect(run).toMatchObject({ stdout: "No active Rerail customer profile.\n", stderr: "", status: 0 });
  }, 60_000);

  it.each([
    [["remote", "list"]],
    [["remote", "exclude", "--repo", named]],
    [["remote", "include", "--repo", named]],
  ])("%j works while the remote sink is off, so a remote added earlier can be seen and removed, and a repository opted out", args => {
    vi.stubEnv("RERAIL_EXPERIMENTAL_REMOTE_SINK", "0");
    const run = rerail(args);
    expect(run.stderr).toBe("");
    expect(run.status).toBe(0);
  }, 60_000);

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
  // Two real runs of the CLI, past argument parsing: allow for a slow Windows runner.
  it("start asks set-up's sessions question about sessions like the one it is read in only with --relay", { timeout: 60_000 }, () => {
    const relayed = rerail(["start", "--repo", checkout, "--relay"]);
    expect(relayed.stdout).toContain("OK for Rerail to read coding agent sessions like this one?");
    // Piped without --relay, no coding agent need be relaying it.
    const piped = rerail(["start", "--repo", checkout]);
    expect(piped.stdout).toContain("OK for Rerail to read your coding agent sessions?");
  });

  it("events answers on the event protocol, whatever its arguments", () => {
    const run = rerail(["events", "--nope"], "nope\n");
    expect(run.stdout).toBe(`${JSON.stringify({ eventId: null, error: { code: "unsupported-version", message: "Expected RERAIL-EVENTS/1" } })}\n`);
    expect(run.stderr).toBe("");
    expect(run.status).toBe(0);
  }, 60_000);
});
