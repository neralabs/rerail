import { describe, expect, it } from "vitest";
import { checkNodeVersion, nodeUpgradeCommand, REQUIRED_NODE_MAJOR } from "../../src/setup/node-version.js";

const NVM = { execPath: "/home/me/.nvm/versions/node/v20.11.1/bin/node", env: { NVM_DIR: "/home/me/.nvm" } };

const ANSI = /\u001b\[/;

describe("checkNodeVersion", () => {
  it("requires Node 24, matching engines.node", () => {
    expect(REQUIRED_NODE_MAJOR).toBe(24);
  });

  it.each(["24.0.0", "v24.0.0", "24", "v24.17.0", "25.2.1", "100.0.0", "24.0.0-nightly20250101abcdef", "v24.0.0-rc.1", " 24.1.0\n"])(
    "passes %j",
    found => {
      expect(checkNodeVersion(found, "terminal").ok).toBe(true);
    },
  );

  it.each(["23.99.99", "v23.11.0", "20.11.1", "v8.17.0", "9.0.0", "0.24.0"])("stops on %j", found => {
    expect(checkNodeVersion(found, "terminal").ok).toBe(false);
  });

  it("stops on a version it cannot read rather than guessing", () => {
    for (const found of ["", "node", "24.x", "v", "twenty-four", "24..0"]) expect(checkNodeVersion(found, "agent").ok).toBe(false);
  });

  it("names both versions, says nothing changed, gives an upgrade hint, and says to run the same command again", () => {
    const check = checkNodeVersion("v20.11.1", "agent", REQUIRED_NODE_MAJOR, NVM);
    if (check.ok) throw new Error("expected Node 20 to fail");
    expect(check.found).toBe("20.11.1");
    expect(check.requiredMajor).toBe(24);
    expect(check.message).toContain("Rerail needs Node 24 or newer. You have Node 20.11.1.");
    expect(check.message).toContain("Nothing was changed on this machine.");
    expect(check.message).toContain("Upgrade Node:  nvm install 24");
    expect(check.message).toContain("Then run the same command again.");
  });

  it("uses the required major it is given", () => {
    const check = checkNodeVersion("25.9.0", "agent", 26, NVM);
    if (check.ok) throw new Error("expected Node 25 to fail a Node 26 requirement");
    expect(check.message).toContain("Rerail needs Node 26 or newer. You have Node 25.9.0.");
    expect(check.message).toContain("nvm install 26");
    expect(checkNodeVersion("26.0.0", "agent", 26).ok).toBe(true);
  });

  it("upgrades Node the way it was installed, and points to nodejs.org when nothing says how", () => {
    const upgrade = (execPath: string, env: NodeJS.ProcessEnv = {}) => nodeUpgradeCommand({ execPath, env }, 24);
    expect(upgrade("/opt/homebrew/Cellar/node/22.1.0/bin/node")).toBe("brew upgrade node");
    expect(upgrade("/usr/local/Cellar/node@22/22.9.0/bin/node")).toBe("brew install node");
    expect(upgrade("/home/me/.volta/tools/image/node/20.0.0/bin/node")).toBe("volta install node@24");
    expect(upgrade("/home/me/.local/share/fnm/node-versions/v20.0.0/installation/bin/node")).toBe("fnm install 24 && fnm use 24");
    expect(upgrade(String.raw`C:\Users\me\AppData\Local\nvm\v20.0.0\node.exe`, { NVM_HOME: String.raw`C:\Users\me\AppData\Local\nvm` })).toBe("nvm install 24");
    // A system Node first on PATH while nvm is installed: nvm still upgrades it for this shell.
    expect(upgrade("/usr/bin/node", { NVM_DIR: "/home/me/.nvm" })).toBe("nvm install 24");
    expect(upgrade("/usr/bin/node")).toBeUndefined();
    const check = checkNodeVersion("18.19.1", "agent", 24, { execPath: "/usr/bin/node", env: {} });
    if (check.ok) throw new Error("expected Node 18 to fail");
    expect(check.message).toContain("Install Node 24 or newer from https://nodejs.org or your package manager.");
  });

  it("says unknown when no version was found", () => {
    const check = checkNodeVersion("  ", "agent");
    if (check.ok) throw new Error("expected an empty version to fail");
    expect(check.message).toContain("You have Node unknown.");
  });

  it("colours only the terminal lane", () => {
    const terminal = checkNodeVersion("20.0.0", "terminal");
    const agent = checkNodeVersion("20.0.0", "agent");
    if (terminal.ok || agent.ok) throw new Error("expected Node 20 to fail");
    expect(terminal.message).toMatch(ANSI);
    expect(agent.message).not.toMatch(ANSI);
  });
});
