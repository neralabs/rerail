import { chmod, mkdir, mkdtemp, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ presentation: vi.fn(), rulesPage: vi.fn(), openPage: vi.fn() }));
vi.mock("../src/session-presentation.js", () => ({ readPresentation: mocks.presentation, readRulesPage: mocks.rulesPage }));
vi.mock("../src/daemon-control.js", async () => ({
  ...await vi.importActual<Record<string, unknown>>("../src/daemon-control.js"),
  openPage: mocks.openPage,
}));
import { registerRerailCommands, type RerailCommandHost, type RerailCommandContext } from "../src/rerail-command.js";

const temporaryDirectories: string[] = [];

beforeEach(() => {
  mocks.openPage.mockResolvedValue("system-browser");
});

afterEach(async () => {
  vi.unstubAllEnvs();
  vi.clearAllMocks();
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

function commandHarness() {
  const commands = new Map<string, Parameters<RerailCommandHost["registerCommand"]>[1]>();
  registerRerailCommands({ registerCommand(name, command) { commands.set(name, command); } }, {
    packageRoot: resolve("."),
    cwd: () => "/workspace",
    sessionId: () => "session-1",
  });
  const notices: Array<{ message: string; level: string }> = [];
  const ctx: RerailCommandContext = {
    ui: {
      notify(message, level) { notices.push({ message, level }); },
      select: async () => undefined,
      input: async () => undefined,
      confirm: async () => false,
    },
  };
  return { run: (name: string) => commands.get(name)!.handler("", ctx), notices };
}

describe("/rerail-ui", () => {
  it("opens the rules page in the browser", async () => {
    mocks.rulesPage.mockResolvedValue({ url: "http://127.0.0.1:4737/#rules", reason: null });
    const { run, notices } = commandHarness();
    await run("rerail-ui");
    expect(mocks.rulesPage).toHaveBeenCalledWith({ cwd: "/workspace" });
    expect(mocks.openPage).toHaveBeenCalledWith("http://127.0.0.1:4737/#rules");
    expect(notices).toEqual([{ message: "Opening http://127.0.0.1:4737/#rules", level: "info" }]);
  });

  it("says so when the page opened as a tab in the Orca window", async () => {
    mocks.rulesPage.mockResolvedValue({ url: "http://127.0.0.1:4737/#rules", reason: null });
    mocks.openPage.mockResolvedValue("orca-tab");
    const { run, notices } = commandHarness();
    await run("rerail-ui");
    expect(notices).toEqual([{ message: "Opening http://127.0.0.1:4737/#rules in the Orca window", level: "info" }]);
  });

  it("reports why there is no page instead of opening nothing", async () => {
    mocks.rulesPage.mockResolvedValue({ url: null, reason: "daemon unavailable" });
    const { run, notices } = commandHarness();
    await run("rerail-ui");
    expect(mocks.openPage).not.toHaveBeenCalled();
    expect(notices).toEqual([{ message: "/rerail-ui: no Rerail page to open: daemon unavailable", level: "error" }]);
  });
});

describe("/rerail-session", () => {
  it("opens the session's page in the browser", async () => {
    mocks.presentation.mockResolvedValue({ text: "Rerail | 3 rules", url: "http://127.0.0.1:4737/?sessionId=session-1&host=omp#sessions", parts: [], presence: "resident" });
    const { run, notices } = commandHarness();
    await run("rerail-session");
    expect(mocks.openPage).toHaveBeenCalledWith("http://127.0.0.1:4737/?sessionId=session-1&host=omp#sessions");
    expect(notices).toEqual([{ message: "Opening http://127.0.0.1:4737/?sessionId=session-1&host=omp#sessions", level: "info" }]);
  });

  it("reports why there is no page instead of opening nothing", async () => {
    mocks.presentation.mockResolvedValue({ text: "Rerail | daemon unavailable", url: null, parts: [], presence: "absent" });
    const { run, notices } = commandHarness();
    await run("rerail-session");
    expect(mocks.openPage).not.toHaveBeenCalled();
    expect(notices).toEqual([{ message: "/rerail-session: no Rerail page to open: Rerail | daemon unavailable", level: "error" }]);
  });
});

describe("Rerail development command launcher", () => {
  it("runs the checkout operation when PATH supplies a malicious just", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-command-launcher-"));
    temporaryDirectories.push(root);
    const bin = join(root, "bin");
    const hijackMarker = join(root, "just-was-run");
    await mkdir(join(root, ".nera"), { recursive: true });
    await mkdir(join(root, "scripts"));
    await mkdir(bin);
    await writeFile(join(root, ".nera", "instance.env"), "NERA_RERAIL_ENABLED=true\nNERA_RERAIL_PUBLIC_URL=http://127.0.0.1:4737\n");
    await writeFile(join(root, "scripts", "ops.sh"), [
      "set -eu",
      "[ \"$1\" = rerail-command ]",
      "[ \"$2\" = status ]",
      "printf 'checkout status is available\\n'",
      "",
    ].join("\n"));
    const fakeJust = join(bin, "just");
    await writeFile(fakeJust, "#!/bin/sh\nprintf hijacked > \"$HIJACK_MARKER\"\n");
    await chmod(fakeJust, 0o755);
    vi.stubEnv("PATH", bin);
    vi.stubEnv("HIJACK_MARKER", hijackMarker);

    const commands = new Map<string, Parameters<RerailCommandHost["registerCommand"]>[1]>();
    registerRerailCommands({ registerCommand(name, command) { commands.set(name, command); } }, {
      packageRoot: resolve("."),
      cwd: () => root,
    });
    const notices: Array<{ message: string; level: string }> = [];
    await commands.get("rerail-status")!.handler("", {
      ui: {
        notify(message, level) { notices.push({ message, level }); },
        select: async () => undefined,
        input: async () => undefined,
        confirm: async () => false,
      },
    });

    expect(notices).toEqual([{ message: "checkout status is available", level: "info" }]);
    await expect(stat(hijackMarker)).rejects.toMatchObject({ code: "ENOENT" });
  });
});
