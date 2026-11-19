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
// `git ls-files --eol` reads every tracked file's attributes and bytes: seconds on a large tree, more under a loaded
// test run. Both tests read the one listing; each has room for it.
let listing;
const eolEntries = () => (listing ??= git(["ls-files", "--eol", "-z"]).toString("utf8").split("\0").filter(Boolean));
const LISTING_TIMEOUT = 30_000;

describe("a checkout's line ends", () => {
  it("are pinned by .gitattributes for every tracked text file, so core.autocrlf changes none", { timeout: LISTING_TIMEOUT }, () => {
    const loose = eolEntries().flatMap((line) => {
      const [, stored, attributes, path] = ENTRY.exec(line) ?? [];
      const text = ["lf", "crlf", "mixed"].includes(stored);
      return text && !/(?:^|\s)(?:-text|eol=lf)(?:\s|$)/u.test(attributes) ? [path] : [];
    });
    expect(loose.length, `unpinned, for example: ${loose.slice(0, 5).join(", ")}`).toBe(0);
  });

  // text=auto keeps a CR the index holds only until `git add --renormalize`, which rewrites the file with LF and
  // changes the bytes its test reads; -text keeps it as stored.
  it("keep a file stored with CRLF or mixed line ends as stored, by -text", { timeout: LISTING_TIMEOUT }, () => {
    const renormalizable = eolEntries().flatMap((line) => {
      const [, stored, attributes, path] = ENTRY.exec(line) ?? [];
      return ["crlf", "mixed"].includes(stored) && !/(?:^|\s)-text(?:\s|$)/u.test(attributes) ? [path] : [];
    });
    expect(renormalizable).toEqual([]);
  });

  it("stay LF in each README checked out with Git for Windows' default", () => {
    const readmes = git(["ls-files", "-z", "--", "README.md", "packs/*/README.md"]).toString("utf8").split("\0").filter(Boolean);
    expect(readmes.length).toBeGreaterThan(1);
    const converted = readmes.filter((path) => git(["-c", "core.autocrlf=true", "cat-file", "--filters", `:${path}`]).includes("\r\n"));
    expect(converted).toEqual([]);
  });
});
