import { readFile } from "node:fs/promises";
import { describe, expect, it } from "vitest";
import { commandFileText } from "../../src/harness/rerail-command-files.js";
import { bugReportInstructions, RERAIL_COMMANDS } from "../../src/harness/rerail-commands.js";

describe("/rerail-bug-report", () => {
  const context = { cli: "'rerail'", packageRoot: "/package", setupLauncher: "'rerail'" };
  const instructions = bugReportInstructions({ cli: "'rerail'", packageRoot: "/package", request: "\"it hung\"" });
  const skill = readFile("skills/rerail-bug-report/SKILL.md", "utf8");

  it("is a command every coding agent gets", () => {
    expect(RERAIL_COMMANDS.map(command => command.name)).toContain("rerail-bug-report");
  });

  it("points the agent at the skill, with the person's words and the launcher", () => {
    expect(instructions).toContain("The person's words: \"it hung\"");
    expect(instructions).toContain("Read /package/skills/rerail-bug-report/SKILL.md and follow it.");
    expect(instructions).toContain("'rerail' support-bundle --json");
  });

  it("never has the agent send anything on its own: an issue is public", async () => {
    expect(instructions).toMatch(/never upload, post, send or paste/iu);
    expect(instructions).toMatch(/never open the issue yourself, unless the person asks/iu);
    expect(await skill).toMatch(/## Never\n\n- Upload, post, send or paste the bundle/u);
  });

  it("points a vulnerability away from a public issue", async () => {
    expect(instructions).toContain("SECURITY.md");
    expect(await skill).toContain("SECURITY.md");
  });

  it("lets Claude Code run only what reads or tests without asking: --output writes any path and rules add changes a rule", () => {
    const file = commandFileText("claude-code", "rerail-bug-report", context);
    expect(file).toContain("Bash('rerail' support-bundle --json)");
    expect(file).toContain("Bash('rerail' support-bundle --json --session *)");
    expect(file).toContain("Bash('rerail' update check)");
    expect(file).toContain("Bash('rerail' rules test *)");
    expect(file).not.toMatch(/Bash\([^)]*--output/u);
    expect(file).not.toMatch(/Bash\([^)]*rules add/u);
    expect(file).not.toMatch(/Bash\([^)]*gh issue/u);
    expect(file).toContain("\"$ARGUMENTS\"");
  });

  it("has the skill name itself in front matter the way agents load skills", async () => {
    expect(await skill).toMatch(/^---\nname: rerail-bug-report\ndescription: \S.*\n---\n/u);
  });

  it("routes each kind of problem: own rules to Tune, the others to the form that fits", async () => {
    const text = await skill;
    expect(text).toContain("[../rerail-create/SKILL.md](../rerail-create/SKILL.md) and follow its Tune section");
    await expect(readFile("skills/rerail-create/SKILL.md", "utf8")).resolves.toContain("## Tune");
    expect(text).toContain("`pack` is null: an **own rule bug**");
    expect(text).toContain("`pack` is `worktree`: the rule is in this worktree's `.rerail/rules/`, so it is the person's: an **own rule bug**");
    expect(text).toContain("never report it on neralabs/rerail");
    expect(text).toContain("Any other `pack`: a **shipped rule bug**");
    expect(text).toContain("| Bug report | `1-bug.yml` |");
    expect(text).toContain("| Rule misfire | `2-rule-misfire.yml` |");
  });

  it("offers the report as a choice", async () => {
    expect(await skill).toContain("Report this on neralabs/rerail? Issues are public.\n     1. Yes, give me the form, filled in\n     2. Yes, file it for me\n     3. Not now");
  });

  // A query parameter that names no field is dropped without a word, so the skill's list has to follow the forms.
  it.each([["Bug report", "1-bug.yml"], ["Rule misfire", "2-rule-misfire.yml"]])("fills only fields the %s form has", async (form, template) => {
    const row = (await skill).split("\n").find(line => line.includes(`| ${form} | \`${template}\``));
    const fields = [...row!.split("|").at(-2)!.matchAll(/`([\w-]+)`/gu)].map(match => match[1]!);
    expect(fields.length).toBeGreaterThan(0);
    const yaml = await readFile(`.github/ISSUE_TEMPLATE/${template}`, "utf8");
    for (const field of fields) {
      const block = new RegExp(`- type: (input|textarea)\\n    id: ${field}\\n`, "u");
      expect(yaml, field).toMatch(block);
    }
  });

  it("is what the forms and the README tell people to run", async () => {
    for (const path of [".github/ISSUE_TEMPLATE/1-bug.yml", ".github/ISSUE_TEMPLATE/2-rule-misfire.yml", "README.md", "docs/troubleshooting.md"]) {
      await expect(readFile(path, "utf8"), path).resolves.toContain("/rerail-bug-report");
    }
  });
});
