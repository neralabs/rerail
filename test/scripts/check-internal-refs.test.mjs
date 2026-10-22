// scripts/check/check-internal-refs.mjs: no internal ticket ID in a tracked path or text.

import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { internalRefProblems } from "../../scripts/check/check-internal-refs.mjs";

/** A ticket ID, assembled so this file does not hold one itself. */
const ticket = (prefix, number) => `${prefix}-${number}`;

let root;
afterEach(() => {
  if (root !== undefined) rmSync(root, { recursive: true, force: true });
  root = undefined;
});

/** A Git repository holding `files` (path to content), all of them tracked. */
function repository(files) {
  root = mkdtempSync(join(tmpdir(), "rerail-internal-refs-"));
  execFileSync("git", ["init", "-q"], { cwd: root });
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), content);
  }
  execFileSync("git", ["add", "."], { cwd: root });
  return root;
}

describe("check-internal-refs", () => {
  it("flags a ticket in any case, in text and in a path, with its line", () => {
    const problems = internalRefProblems(
      repository({
        "src/a.ts": `const a = 1;\n// fixed in ${ticket("NER", 1764)}, see also (${ticket("NER", 12)})\n`,
        "docs/b.md": `On branch \`claude/${ticket("ner", 1762)}-gate-harness\`.\n`,
        [`docs/${ticket("ner", 2036)}-notes.md`]: "Nothing here.\n",
      }),
    );
    expect(problems).toEqual([
      { path: `docs/${ticket("ner", 2036)}-notes.md`, line: 0, text: `docs/${ticket("ner", 2036)}-notes.md` },
      { path: "docs/b.md", line: 1, text: `On branch \`claude/${ticket("ner", 1762)}-gate-harness\`.` },
      { path: "src/a.ts", line: 2, text: `// fixed in ${ticket("NER", 1764)}, see also (${ticket("NER", 12)})` },
    ]);
  });

  it("passes words that only contain the prefix, a prefix without digits, and binary files", () => {
    const problems = internalRefProblems(
      repository({
        "a.txt": `${ticket("INNER", 12)} corpus-nera ${ticket("NER", "x")}\n`,
        "b.bin": Buffer.concat([Buffer.from([0, 1, 2, 0]), Buffer.from(ticket("NER", 99))]),
      }),
    );
    expect(problems).toEqual([]);
  });
});
