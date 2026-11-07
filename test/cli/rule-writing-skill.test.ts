import { execFile } from "node:child_process";
import { access, mkdtemp, readdir, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { promisify } from "node:util";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { parseRuleDraft, testRuleDraft } from "../../src/cli/rule-draft.js";

/**
 * The `/rerail-create` skill (`skills/rerail-create/`): every rule draft it shows the agent is a fenced `json draft` block, and
 * each one must be valid with every case right, as `rerail rules test` would report it. Its Markdown links must resolve.
 */

const SKILL = resolve("skills", "rerail-create");

async function markdownFiles(directory: string): Promise<string[]> {
  const entries = await readdir(directory, { withFileTypes: true, recursive: true });
  return entries.filter(entry => entry.isFile() && entry.name.endsWith(".md")).map(entry => join(entry.parentPath, entry.name)).sort();
}

const files = await markdownFiles(SKILL);
const texts = new Map(await Promise.all(files.map(async file => [file, await readFile(file, "utf8")] as const)));

const drafts = [...texts].flatMap(([file, text]) =>
  [...text.matchAll(/^```json draft\n([\s\S]*?)^```$/gmu)].map((match, index) => ({
    name: `${file.slice(SKILL.length + 1)} #${index + 1}`,
    json: match[1]!,
  })));

let repo = "";

beforeAll(async () => {
  repo = await mkdtemp(join(tmpdir(), "rerail-skill-drafts-"));
  await promisify(execFile)("git", ["init", "-q", "-b", "main"], { cwd: repo });
});

afterAll(async () => {
  await rm(repo, { recursive: true, force: true });
});

describe("rerail-create skill drafts", () => {
  it("holds drafts of both kinds", () => {
    const kinds = drafts.map(draft => (JSON.parse(draft.json) as { kind: string }).kind);
    expect(kinds).toContain("code");
    expect(kinds).toContain("conversation");
  });

  it.each(drafts)("$name is valid and every case is right", async ({ json }) => {
    const report = await testRuleDraft(parseRuleDraft(JSON.parse(json)), repo);
    expect(report.valid).toBe(true);
    expect(report.cases.filter(result => !result.ok)).toEqual([]);
    expect(report.cases.filter(result => result.expected === "caught").length).toBeGreaterThanOrEqual(1);
    expect(report.cases.filter(result => result.expected === "passed").length).toBeGreaterThanOrEqual(2);
  });
});

describe("rerail-create skill", () => {
  it("names itself in front matter the way agents load skills", () => {
    const skill = texts.get(join(SKILL, "SKILL.md"))!;
    expect(skill).toMatch(/^---\nname: rerail-create\ndescription: \S.*\n---\n/u);
  });

  it("asks before adding, with the read-back the command promises", () => {
    const skill = texts.get(join(SKILL, "SKILL.md"))!;
    expect(skill).toContain("rerail rules evidence --json");
    expect(skill).toContain("rerail rules test <tmp>/draft.json --json");
    expect(skill).toContain("rerail rules add <tmp>/draft.json");
    expect(skill).toContain("Add this rule?\n     1. Yes, add it\n     2. Change something\n     3. Show me the rule\n     4. Cancel");
  });

  it.each(files.map(file => [file.slice(SKILL.length + 1), file]))("%s links only to files that exist", async (_, file) => {
    const prose = texts.get(file)!.replace(/^```[\s\S]*?^```$/gmu, "");
    const links = [...prose.matchAll(/\]\(([^)#\s]+)(?:#[^)]*)?\)/gu)].map(match => match[1]!).filter(link => !/^[a-z]+:/u.test(link));
    for (const link of links) await expect(access(resolve(dirname(file), link)), link).resolves.toBeUndefined();
  });
});
