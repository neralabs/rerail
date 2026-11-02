import { describe, expect, it } from "vitest";
import { commandFileText } from "../../src/harness/rerail-command-files.js";
import { RERAIL_COMMANDS, supportBundleInstructions } from "../../src/harness/rerail-commands.js";

describe("/rerail-support-bundle", () => {
  const context = { cli: "'rerail'", packageRoot: "/package", setupLauncher: "'rerail'" };

  it("is a command every coding agent gets", () => {
    expect(RERAIL_COMMANDS.map(command => command.name)).toContain("rerail-support-bundle");
  });

  it("has the agent run the command, read the file and tell the person what to check", () => {
    const text = supportBundleInstructions("'rerail'");
    expect(text).toContain("'rerail' support-bundle --json");
    expect(text).toMatch(/steer/iu);
    expect(text).toMatch(/names?, emails?/iu);
  });

  it("never has the agent send the file: the person attaches it", () => {
    expect(supportBundleInstructions("'rerail'")).toMatch(/never upload, post, send or paste/iu);
  });

  it("points a vulnerability away from a public issue", () => {
    expect(supportBundleInstructions("'rerail'")).toContain("SECURITY.md");
  });

  it("lets Claude Code run only the fixed invocations without asking: --output writes any path", () => {
    const file = commandFileText("claude-code", "rerail-support-bundle", context);
    expect(file).toContain("Bash('rerail' support-bundle --json)");
    expect(file).toContain("Bash('rerail' support-bundle --json --session *)");
    expect(file).not.toMatch(/Bash\([^)]*--output/u);
  });
});
