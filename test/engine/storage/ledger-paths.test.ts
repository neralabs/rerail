import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { makeLedgerFolder, writeLedgerFile } from "../../../src/engine/storage/ledger-paths.js";

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function checkout(): string {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-ledger-")));
  roots.push(root);
  execFileSync("git", ["init", "--quiet"], { cwd: root });
  return root;
}

describe("session record folders", () => {
  it("keep Git from offering their records for a commit, and leave .rerail/rules committable", async () => {
    const root = checkout();
    const sessions = join(root, ".rerail", "sessions");
    await writeLedgerFile(join(sessions, "s-1.json"), sessions, "{\"prompt\":\"secret\"}");
    mkdirSync(join(root, ".rerail", "rules"));
    writeFileSync(join(root, ".rerail", "rules", "r.json"), "{}");
    const status = execFileSync("git", ["status", "--porcelain", "--untracked-files=all"], { cwd: root, encoding: "utf8" });
    expect(status.split("\n").filter(line => line.length > 0)).toEqual(["?? .rerail/rules/r.json"]);
  });

  it("never replace a .gitignore that is already there", async () => {
    const root = checkout();
    const sessions = join(root, ".rerail", "sessions");
    mkdirSync(sessions, { recursive: true });
    writeFileSync(join(sessions, ".gitignore"), "mine\n");
    await makeLedgerFolder(join(sessions, "nested"));
    expect(readFileSync(join(sessions, ".gitignore"), "utf8")).toBe("mine\n");
  });

  it("write nothing into a storage root outside any .rerail folder", async () => {
    const root = checkout();
    await makeLedgerFolder(join(root, "elsewhere"));
    expect(existsSync(join(root, "elsewhere", ".gitignore"))).toBe(false);
  });
});
