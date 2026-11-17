// context7.json tells Context7 which of Rerail's docs to serve to coding agents, and the guidance to serve with them.
// Context7 skips a list entry its schema (https://context7.com/schema/context7.json) refuses and says so only in its
// own logs, so a rule that outgrows the schema's limit goes missing without a failure here. These are that schema's
// limits for the fields the file uses, and two checks that read the release checklist's "its folders, exclusions and
// rules match the docs" (docs/release.md, "Before merging"): every exclusion names a file Context7 would serve, and no
// design note is served.
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

/** Each `excludeFolders` glob as a test for a path inside that folder: `**` spans any folders, `*` stays in one. */
const excludedFolders = context7.excludeFolders.map(glob => {
  const pattern = glob
    .split("/")
    .map(part => (part === "**" ? "(?:[^/]+/)*" : `${part.replace(/[.+?^${}()|[\]\\]/g, "\\$&").replace(/\*/g, "[^/]*")}/`))
    .join("");
  return new RegExp(`^${pattern}`);
});

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

  it("excludes files by name alone, each the name of a file Context7 would otherwise serve", () => {
    const served = new Set(
      execFileSync("git", ["ls-files", "-z", "--", ...context7.folders], {
        cwd: REPOSITORY,
        encoding: "utf8",
        maxBuffer: 64 * 1024 * 1024,
      })
        .split("\0")
        .filter(path => path !== "" && !excludedFolders.some(folder => folder.test(path)))
        .map(path => basename(path)),
    );
    expect(context7.excludeFiles.filter(name => /[/\\]/.test(name))).toEqual([]);
    expect(context7.excludeFiles.filter(name => !served.has(name))).toEqual([]);
  });

  it("serves no design note", () => {
    const designNotes = documentsUnder("Design notes");
    expect(designNotes.length).toBeGreaterThan(0);
    expect(designNotes.filter(name => !context7.excludeFiles.includes(name))).toEqual([]);
  });
});
