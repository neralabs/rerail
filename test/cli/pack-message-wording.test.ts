import { execFile } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { parseRuleDraft, testRuleDraft } from "../../src/cli/rule-draft.js";

/**
 * `packs/pack-message-wording.draft.json`: the rule this repository adds for itself (`rerail rules add`), which steers
 * whoever edits a pack rule's message. Every case must be right, as `rerail rules test` would report it. What it matches
 * in the packs is left to the rule while someone edits them: CI does not gate messages.
 */

const DRAFT = join("packs", "pack-message-wording.draft.json");

let repo = "";

beforeAll(async () => {
  repo = await mkdtemp(join(tmpdir(), "rerail-pack-message-wording-"));
  await promisify(execFile)("git", ["init", "-q", "-b", "main"], { cwd: repo });
});

afterAll(async () => {
  await rm(repo, { recursive: true, force: true });
});

describe("pack message wording rule", () => {
  it("is valid and every case is right", async () => {
    const report = await testRuleDraft(parseRuleDraft(JSON.parse(await readFile(DRAFT, "utf8"))), repo);
    expect(report.valid).toBe(true);
    expect(report.cases.filter(result => !result.ok)).toEqual([]);
  });
});
