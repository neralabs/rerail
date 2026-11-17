// context7.json tells Context7 which of Rerail's docs to serve to coding agents, and the guidance to serve with them.
// Context7 skips a list entry its schema (https://context7.com/schema/context7.json) refuses and says so only in its
// own logs, so a rule that outgrows the schema's limit goes missing without a failure here. These are that schema's
// limits for the fields the file uses, and the two things the release checklist asks of it (docs/release.md, "Before
// merging"): every exclusion still names a file, and no design note is served.
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { basename, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const REPOSITORY = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const read = (path: string): string => readFileSync(resolve(REPOSITORY, path), "utf8");

interface Context7 {
  description: string;
  folders: string[];
  excludeFolders: string[];
  excludeFiles: string[];
  rules: string[];
}

const context7 = JSON.parse(read("context7.json")) as Context7;

/** The documents docs/README.md lists under `heading`, by file name. */
function documentsUnder(heading: string): string[] {
  const section = read("docs/README.md").split(/^## /m).find(part => part.startsWith(`${heading}\n`));
  if (section === undefined) throw new Error(`docs/README.md has no "${heading}" section`);
  return [...section.matchAll(/^- \[[^\]]+\]\(([^)#]+)\)/gm)].map(match => basename(match[1] ?? ""));
}

describe("context7.json", () => {
  it("keeps every rule within the schema's 255 characters, and the list within its 50", () => {
    expect(context7.rules.filter(rule => rule.length < 1 || rule.length > 255)).toEqual([]);
    expect(context7.rules.length).toBeLessThanOrEqual(50);
  });

  it("keeps the description within the schema's 10 to 500 characters", () => {
    expect(context7.description.length).toBeGreaterThanOrEqual(10);
    expect(context7.description.length).toBeLessThanOrEqual(500);
  });

  it("excludes files by name alone, each the name of a tracked file", () => {
    const tracked = new Set(
      execFileSync("git", ["ls-files", "-z"], { cwd: REPOSITORY, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 })
        .split("\0")
        .map(path => basename(path)),
    );
    expect(context7.excludeFiles.filter(name => /[/\\]/.test(name))).toEqual([]);
    expect(context7.excludeFiles.filter(name => !tracked.has(name))).toEqual([]);
  });

  it("serves no design note", () => {
    const designNotes = documentsUnder("Design notes");
    expect(designNotes.length).toBeGreaterThan(0);
    expect(designNotes.filter(name => !context7.excludeFiles.includes(name))).toEqual([]);
  });
});
