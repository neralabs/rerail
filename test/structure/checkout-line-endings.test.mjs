// A checkout's line ends. Git for Windows sets core.autocrlf=true in its system config, which checks every text file
// out with CRLF unless .gitattributes pins its line ends. The model's checksums, the pack README checks and the
// structure gate read the checkout's bytes, so every text file must check out as it is stored, whatever that setting.

import { execFileSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const git = (args) => execFileSync("git", args, { cwd: root, maxBuffer: 256 * 1024 * 1024 });
const ENTRY = /^i\/(\S*)\s+w\/\S*\s+attr\/(.*?)\s*\t(.*)$/su;

describe("a checkout's line ends", () => {
  it("are pinned by .gitattributes for every tracked text file, so core.autocrlf changes none", () => {
    const loose = git(["ls-files", "--eol", "-z"]).toString("utf8").split("\0").filter(Boolean).flatMap((line) => {
      const [, stored, attributes, path] = ENTRY.exec(line) ?? [];
      const text = ["lf", "crlf", "mixed"].includes(stored);
      return text && !/(?:^|\s)(?:-text|eol=lf)(?:\s|$)/u.test(attributes) ? [path] : [];
    });
    expect(loose.length, `unpinned, for example: ${loose.slice(0, 5).join(", ")}`).toBe(0);
  });

  it("stay LF in each README checked out with Git for Windows' default", () => {
    const readmes = git(["ls-files", "-z", "--", "README.md", "packs/*/README.md"]).toString("utf8").split("\0").filter(Boolean);
    expect(readmes.length).toBeGreaterThan(1);
    const converted = readmes.filter((path) => git(["-c", "core.autocrlf=true", "cat-file", "--filters", `:${path}`]).includes("\r\n"));
    expect(converted).toEqual([]);
  });
});
