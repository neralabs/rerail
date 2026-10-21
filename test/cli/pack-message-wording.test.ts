import { execFile } from "node:child_process";
import { mkdtemp, readdir, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { parseRuleDraft, testRuleDraft } from "../../src/cli/rule-draft.js";

/**
 * `packs/*.draft.json`: the rules this repository adds for itself (`rerail rules add`), which steer whoever edits a pack
 * rule's message. Every case of each must be right, as `rerail rules test` would report it. What they match in the packs
 * is left to the rules while someone edits them: CI does not gate messages.
 */

const drafts = (await readdir("packs")).filter(name => name.endsWith(".draft.json")).sort().map(name => join("packs", name));

let repo = "";

beforeAll(async () => {
  repo = await mkdtemp(join(tmpdir(), "rerail-pack-message-wording-"));
  await promisify(execFile)("git", ["init", "-q", "-b", "main"], { cwd: repo });
});

afterAll(async () => {
  await rm(repo, { recursive: true, force: true });
});

describe("pack message wording rules", () => {
  it("holds the tool name, Rerail and shouting rules", () => {
    expect(drafts).toEqual(expect.arrayContaining([
      join("packs", "pack-message-rerail.draft.json"),
      join("packs", "pack-message-shouting.draft.json"),
      join("packs", "pack-message-tool-names.draft.json"),
    ]));
  });

  it.each(drafts)("%s is valid and every case is right", async draft => {
    const report = await testRuleDraft(parseRuleDraft(JSON.parse(await readFile(draft, "utf8"))), repo);
    expect(report.valid).toBe(true);
    expect(report.cases.filter(result => !result.ok)).toEqual([]);
    expect(report.cases.filter(result => result.expected === "caught").length).toBeGreaterThanOrEqual(2);
    expect(report.cases.filter(result => result.expected === "passed").length).toBeGreaterThanOrEqual(2);
  });
});
