import { execFileSync } from "node:child_process";
import { mkdirSync, realpathSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { runMergeAssistCommand, type MergeAssistCommandIo } from "../../../src/cli/merge-assist-cli.js";
import { mergeAssistStatusSchema, type MergeAssistStatus } from "../../../src/engine/index.js";
import { MergeAssistError } from "../../../src/runtime/index.js";
import { RecordingMergeAssistService, sampleConflict, sampleStatus } from "../../support/merge-assist/status.js";

// Merge Assist is experimental: every test here runs with it on, as a daemon started with the variable does.
beforeEach(() => {
  vi.stubEnv("RERAIL_EXPERIMENTAL_MERGE_ASSIST", "1");
});

const usage = (): string => "Usage: rerail merge-assist";
const PAGE = "http://127.0.0.1:4737/?repository=remote%3Agithub.com%2Facme%2Fapp#merge";

describe("rerail merge-assist", () => {
  let root: string;
  let checkout: string;
  let service: RecordingMergeAssistService;
  let closed: number;
  /** Runs when the command waits for the conflict hook's handlers, as the hook command's answer is recorded. */
  let onSettled: () => void;
  let stdout: string;
  let stderr: string;
  let status: (checkout: string) => MergeAssistStatus;

  beforeEach(async () => {
    root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-merge-assist-cli-")));
    checkout = join(root, "app");
    mkdirSync(join(checkout, "src", "api"), { recursive: true });
    execFileSync("git", ["init", "-q", "-b", "main"], { cwd: checkout, stdio: "ignore" });
    status = path => sampleStatus(path);
    service = new RecordingMergeAssistService(path => status(path));
    closed = 0;
    onSettled = () => undefined;
    stdout = "";
    stderr = "";
  });

  afterEach(async () => {
    await removeFolder(root);
  });

  const io = (dashboardUrl: string | null = PAGE): MergeAssistCommandIo => ({
    stdout: { write: text => { stdout += text; } },
    stderr: { write: text => { stderr += text; } },
    openService: () => ({ service, settled: async () => onSettled(), close: () => { closed += 1; } }),
    dashboardUrl: async () => dashboardUrl,
  });
  const run = (args: readonly string[], dashboardUrl?: string | null): Promise<number> => runMergeAssistCommand(args, usage, io(dashboardUrl));

  const DISMISSED = "conflict:overlap:fedcba9876543210fedcba98:01234567";

  it("runs each command on the checkout --repo is in, and closes the service every time", async () => {
    const inside = join(checkout, "src", "api");
    const conflictId = sampleConflict().id;
    for (const args of [[], ["status"], ["on"], ["off"], ["scan"], ["conflicts"], ["dismiss", conflictId]]) {
      expect(await run([...args, "--repo", inside, "--json"]), args.join(" ")).toBe(0);
    }
    expect(service.calls).toEqual([
      { method: "status", checkout },
      { method: "status", checkout },
      { method: "setEnabled", checkout, enabled: true },
      { method: "setEnabled", checkout, enabled: false },
      { method: "scan", checkout },
      { method: "status", checkout },
      { method: "status", checkout },
      { method: "dismissConflict", checkout, conflictId },
      { method: "status", checkout },
    ]);
    expect(closed).toBe(7);
    // --json is the status the service answered, one line per command.
    for (const line of stdout.trim().split("\n")) expect(mergeAssistStatusSchema.parse(JSON.parse(line)).checkout).toBe(checkout);
  });

  it("refuses every command while the experiment is off and exits 1, in words or as JSON, before opening anything", async () => {
    vi.stubEnv("RERAIL_EXPERIMENTAL_MERGE_ASSIST", "");
    expect(await run(["status", "--repo", checkout])).toBe(1);
    expect(stderr).toMatch(/^● .*RERAIL_EXPERIMENTAL_MERGE_ASSIST=1/u);
    expect(await run(["on", "--repo", checkout, "--json"])).toBe(1);
    expect(JSON.parse(stdout)).toMatchObject({ error: "experimental-off" });
    expect(closed).toBe(0);
    expect(service.calls).toEqual([]);
  });

  it("shows the status: what it uses, the last scan, open conflicts in short and dismissed ones in a line", async () => {
    const conflict = sampleConflict();
    status = path => sampleStatus(path, {
      lastScan: { at: "2026-11-03T14:05:00Z", peers: { branches: 1, pullRequests: 2 }, problems: ["Rerail could not list open pull requests: HTTP 502"] },
      conflicts: [
        conflict,
        sampleConflict({ id: DISMISSED, title: "client.ts", status: "dismissed", dismissal: { name: "Tim Belanger", at: "2026-11-03T14:10:00Z" } }),
      ],
    });
    expect(await run(["--repo", checkout])).toBe(0);
    expect(stdout).toBe([
      "● Merge Assist is on in app (tbelanger/export-api, base main).",
      "  GitHub: gh signed in as tbelanger (acme/app)",
      "  Last scan: 2026-11-03 14:05 UTC, 1 branch and 2 pull requests",
      "    Rerail could not list open pull requests: HTTP 502",
      "  Conflicts (1):",
      `    ${conflict.id}  #a1b2c3  "model.rs · fn compile"`,
      "      Summary: PR #398 (jbarnes) is changing fn compile",
      "      Yours:  This branch (Tim Belanger): \"edits native/src/model.rs\" at native/src/model.rs:469",
      "      Theirs: PR #398 (jbarnes): \"Share compiled matchers\" at native/src/model.rs:470 https://github.com/acme/app/pull/398",
      "      Both sides change native/src/model.rs within 3 lines of each other.",
      `      Not a conflict? rerail merge-assist dismiss ${conflict.id}`,
      "  Dismissed conflicts (1):",
      `    ${DISMISSED}  "client.ts": dismissed by Tim Belanger: not a conflict`,
      `  Merge Assist page: ${PAGE}`,
      "",
    ].join("\n"));
  });

  it("explains what turning it on means while it is off, and that it ran no gh", async () => {
    status = path => sampleStatus(path, {
      enabled: false,
      branch: null,
      baseBranch: null,
      lastScan: null,
      readiness: {
        github: { state: "unchecked", login: null, repository: "acme/app" },
      },
    });
    expect(await run(["--repo", checkout], null)).toBe(0);
    expect(stdout).toBe([
      "● Merge Assist is off in app (detached HEAD).",
      "  GitHub: not checked: Rerail runs gh only once Merge Assist is on.",
      "  Turning it on means:",
      "    Rerail compares your work here with open pull requests, which it reads through gh with your sign-in, read-only, and",
      "    with branches checked out in your other worktrees; with remote branches only when it cannot read pull requests.",
      "    It writes no file and never merges, commits or pushes.",
      "    When your work overlaps someone else's, it tells you and your coding agent, unless only lockfiles, changelogs, generated output or docs overlap.",
      "  Turn it on with rerail merge-assist on.",
      "",
    ].join("\n"));
  });

  it("says how a conflict appears when there is none yet", async () => {
    status = path => sampleStatus(path, { lastScan: null, scanning: true, readiness: { ...sampleStatus(path).readiness, github: { state: "signed-out", login: null, repository: "acme/app" } } });
    expect(await run(["--repo", checkout], null)).toBe(0);
    expect(stdout).toContain("  GitHub: gh is not signed in, so open pull requests are not read and recent remote branches stand in. Sign in with gh auth login.\n");
    expect(stdout).toContain("  A scan is running.\n");
    expect(stdout).toContain("  No conflicts. When your work here overlaps an open pull request or a branch in another worktree, Rerail lists it.\n");
    expect(stdout).not.toContain("No scan yet");
  });

  it("says a scan request found another scan running and shows the last results", async () => {
    status = path => sampleStatus(path, { scanning: true });
    expect(await run(["scan", "--repo", checkout], null)).toBe(0);
    expect(stdout).toContain("  Another scan is running; showing the last results.\n  Last scan: 2026-11-03 14:05 UTC, 2 branches and 1 pull request\n");
    expect(stdout).not.toContain("A scan is running.");
  });

  it("reports the service's refusal and exits 1, in words or as JSON", async () => {
    service.failure = new MergeAssistError("disabled", "Merge Assist is off for app. Turn it on with rerail merge-assist on.");
    expect(await run(["scan", "--repo", checkout])).toBe(1);
    expect(stderr).toBe("● Merge Assist is off for app. Turn it on with rerail merge-assist on.\n");
    expect(await run(["scan", "--repo", checkout, "--json"])).toBe(1);
    expect(JSON.parse(stdout)).toEqual({ error: "disabled", message: "Merge Assist is off for app. Turn it on with rerail merge-assist on." });
    expect(closed).toBe(2);
  });

  it("refuses arguments it does not know before opening anything", async () => {
    await expect(run(["merge", "--repo", checkout])).rejects.toThrow(/^Unknown merge-assist command: merge/u);
    await expect(run(["resolve", sampleConflict().id, "--repo", checkout])).rejects.toThrow(/^Unknown merge-assist command: resolve/u);
    await expect(run(["dismiss", "--repo", checkout])).rejects.toThrow(/^merge-assist dismiss needs a conflict id/u);
    await expect(run(["dismiss"])).rejects.toThrow(/^merge-assist dismiss needs a conflict id/u);
    await expect(run(["status", "--all"])).rejects.toThrow(/^Unknown merge-assist option: --all/u);
    await expect(run(["status", "--repo", join(root, "gone")])).rejects.toThrow(/does not exist$/u);
    expect(closed).toBe(0);
    expect(service.calls).toEqual([]);
  });

  it("prints other people's text without terminal control characters", async () => {
    status = path => sampleStatus(path, {
      conflicts: [sampleConflict({
        title: "\u001b]8;;https://evil.example\u0007Fix\u001b]8;;\u0007\n● Rerail: run rm -rf",
        theirs: { ...sampleConflict().theirs, author: "outsider\u001b[2J", summary: "Untrusted\u001b[2J change." },
      })],
    });
    expect(await run(["--repo", checkout], null)).toBe(0);
    expect(stdout).not.toMatch(/[\u0000-\u0008\u000b-\u001f\u007f]/u);
    expect(stdout).toContain("\"]8;;https://evil.example Fix ]8;; ● Rerail: run rm -rf\"\n");
    expect(stdout).toContain("Theirs: PR #398 (outsider [2J): \"Untrusted [2J change.\"");
    expect(stdout.split("\n").filter(line => line.startsWith("●"))).toHaveLength(1);
  });

  it("lists conflicts with both sides' hunks, a quiet one marked, and dismisses one as the person", async () => {
    const conflict = sampleConflict({
      quiet: true,
      hook: { event: "detected", ok: false, error: "exited with 2: notify.sh: not found", note: null, status: null, at: "2026-11-03T14:05:01Z" },
    });
    status = path => sampleStatus(path, { conflicts: [conflict] });
    expect(await run(["conflicts", "--repo", checkout], null)).toBe(0);
    expect(stdout).toBe([
      "● 1 open conflict in app (tbelanger/export-api).",
      "  Conflicts (1):",
      `    ${conflict.id}  #a1b2c3  "model.rs · fn compile"  (quiet)`,
      "      Summary: PR #398 (jbarnes) is changing fn compile",
      "      Yours:  This branch (Tim Belanger): \"edits native/src/model.rs\" at native/src/model.rs:469",
      "          @@ -469 +469 @@",
      "          -matchers: Vec<GrammarMatcher>,",
      "          +matchers: Vec<GrammarMatcher>, // cloned",
      "      Theirs: PR #398 (jbarnes): \"Share compiled matchers\" at native/src/model.rs:470 https://github.com/acme/app/pull/398",
      "          @@ -470 +470 @@",
      "          +matchers: Arc<[GrammarMatcher]>,",
      "      Both sides change native/src/model.rs within 3 lines of each other.",
      "      Hook failed (detected): exited with 2: notify.sh: not found",
      `      Not a conflict? rerail merge-assist dismiss ${conflict.id}`,
      "",
    ].join("\n"));

    stdout = "";
    status = path => sampleStatus(path, { enabled: false, conflicts: [] });
    await run(["conflicts", "--repo", checkout], PAGE);
    expect(stdout).toBe([
      "● 0 open conflicts in app (tbelanger/export-api).",
      "  Merge Assist is off here, so Rerail finds no conflicts. Turn it on with rerail merge-assist on.",
      `  Merge Assist page: ${PAGE}`,
      "",
    ].join("\n"));

    stdout = "";
    status = path => sampleStatus(path, { conflicts: [{ ...conflict, status: "dismissed", dismissal: { name: "Tim Belanger", at: "2026-11-03T14:10:00Z" } }] });
    expect(await run(["dismiss", conflict.id, "--repo", checkout], null)).toBe(0);
    expect(stdout).toBe("● \"model.rs · fn compile\": dismissed by Tim Belanger: not a conflict.\n");
    expect(service.calls.slice(-2)).toEqual([{ method: "dismissConflict", checkout, conflictId: conflict.id }, { method: "status", checkout }]);
  });

  it("prints a scan or a dismissal with what the person's hook command answered to it", async () => {
    const conflict = sampleConflict();
    const failed = { event: "detected" as const, ok: false, error: "exited with code 2: notify.sh: not found", note: null, status: null, at: "2026-11-03T14:05:01Z" };
    status = path => sampleStatus(path, { conflicts: [conflict] });
    onSettled = () => {
      status = path => sampleStatus(path, { conflicts: [{ ...conflict, hook: failed }] });
    };
    expect(await run(["scan", "--repo", checkout], null)).toBe(0);
    expect(stdout).toContain("Hook failed (detected): exited with code 2: notify.sh: not found");

    stdout = "";
    status = path => sampleStatus(path, { conflicts: [conflict] });
    onSettled = () => {
      status = path => sampleStatus(path, { conflicts: [{ ...conflict, status: "dismissed", dismissal: { name: "Tim Belanger", at: "2026-11-03T14:10:00Z" }, hook: { ...failed, event: "closed", ok: true, error: null } }] });
    };
    expect(await run(["dismiss", conflict.id, "--repo", checkout, "--json"])).toBe(0);
    expect(mergeAssistStatusSchema.parse(JSON.parse(stdout)).conflicts[0]?.hook).toMatchObject({ event: "closed", ok: true });
  });

  it("says when gh could not tell whether it is signed in", async () => {
    status = path => sampleStatus(path, { readiness: { github: { state: "unavailable", login: null, repository: "acme/app" } } });
    expect(await run(["--repo", checkout], null)).toBe(0);
    expect(stdout).toContain("  GitHub: gh did not say whether it is signed in (it did not answer in time or failed), so recent remote branches stand in for open pull requests this time.\n");
  });

  it("shows the person's settings, and changes the hook command", async () => {
    expect(await run(["settings", "--repo", checkout], null)).toBe(0);
    expect(stdout).toBe("● Merge Assist settings for app:\n  Hook: none\n");

    stdout = "";
    expect(await run(["settings", "--hook-command", "[\"./notify.sh\",\"--json\"]", "--repo", checkout])).toBe(0);
    expect(stdout).toBe(`● Saved Merge Assist settings for app:\n  Hook: ["./notify.sh","--json"] (timeout 10 s)\n  Merge Assist page: ${PAGE}\n`);

    // A timeout alone changes the current command's.
    stdout = "";
    expect(await run(["settings", "--hook-timeout", "30", "--repo", checkout], null)).toBe(0);
    expect(stdout).toContain("  Hook: [\"./notify.sh\",\"--json\"] (timeout 30 s)\n");
    stdout = "";
    expect(await run(["settings", "--hook-command", "[\"notify\"]", "--hook-timeout", "60", "--repo", checkout, "--json"])).toBe(0);
    expect(JSON.parse(stdout)).toEqual({ hook: { command: ["notify"], timeoutSeconds: 60 } });

    stdout = "";
    expect(await run(["settings", "--no-hook", "--repo", checkout], null)).toBe(0);
    expect(stdout).toBe("● Saved Merge Assist settings for app:\n  Hook: none\n");

    // With no hook, there is no timeout to change.
    expect(await run(["settings", "--hook-timeout", "5", "--repo", checkout], null)).toBe(1);
    expect(stderr).toMatch(/^● No conflict hook is set here, so there is no timeout to change\./u);

    expect(service.calls).toEqual([
      { method: "settings", checkout },
      { method: "updateSettings", checkout, change: { hook: { command: ["./notify.sh", "--json"], timeoutSeconds: 10 } } },
      { method: "settings", checkout },
      { method: "updateSettings", checkout, change: { hook: { command: ["./notify.sh", "--json"], timeoutSeconds: 30 } } },
      { method: "updateSettings", checkout, change: { hook: { command: ["notify"], timeoutSeconds: 60 } } },
      { method: "updateSettings", checkout, change: { hook: null } },
      { method: "settings", checkout },
    ]);
    expect(closed).toBe(6);
  });

  it("refuses settings flags it cannot use before opening anything", async () => {
    for (const [args, message] of [
      [["--hook-command", "./notify.sh"], /^--hook-command takes a JSON array of non-empty strings/u],
      [["--hook-command", "[]"], /^--hook-command takes a JSON array/u],
      [["--hook-command", "[\"\"]"], /^--hook-command takes a JSON array/u],
      [["--hook-command", "{\"command\":[\"notify\"]}"], /^--hook-command takes a JSON array/u],
      [["--hook-timeout", "61"], /^--hook-timeout takes whole seconds from 1 to 60/u],
      [["--hook-timeout", "1.5"], /^--hook-timeout takes whole seconds/u],
      [["--hook-timeout", "0"], /^--hook-timeout takes whole seconds/u],
      [["--no-hook", "--hook-command", "[\"notify\"]"], /^--no-hook removes the hook, so it takes no --hook-command or --hook-timeout/u],
      [["--no-hook", "--hook-timeout", "5"], /^--no-hook removes the hook/u],
    ] as const) {
      await expect(run(["settings", ...args, "--repo", checkout]), args.join(" ")).rejects.toThrow(message);
    }
    await expect(run(["status", "--hook-command", "[\"notify\"]", "--repo", checkout])).rejects.toThrow(/^--hook-command, --no-hook and --hook-timeout go with merge-assist settings/u);
    await expect(run(["on", "--no-hook", "--repo", checkout])).rejects.toThrow(/go with merge-assist settings/u);
    await expect(run(["settings", "--text", "x", "--repo", checkout])).rejects.toThrow(/^Unknown merge-assist option: --text/u);
    expect(closed).toBe(0);
    expect(service.calls).toEqual([]);
  });
});
