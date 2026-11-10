// scripts/check/check-internal-refs.mjs: no internal ticket ID, and no `nera-` name outside the public list, in a tracked
// path or text.

import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { internalRefProblems, privateNameProblems } from "../../../scripts/check/check-internal-refs.mjs";
import { removeFolderSync } from "../../support/platform/remove-folder.ts";

/** A ticket ID, assembled so this file does not hold one itself. */
const ticket = (prefix, number) => `${prefix}-${number}`;
/** A `nera-` name, assembled so this file does not hold a flagged one itself. */
const nera = (rest) => `${"nera"}-${rest}`;

let root;
afterEach(() => {
  if (root !== undefined) removeFolderSync(root);
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

  it("flags a nera- name off the public list in any case, in text and in a path, with the names it flags", () => {
    const problems = privateNameProblems(
      repository({
        "src/a.ts": `const a = 1;\n// ported from ${nera("example-service")}'s ${nera("Example-Service")} and ${nera("example-service")}\n`,
        "docs/b.md": `Source: ${nera("Example-App")} web/src/index.css, its ${nera("card-shadow")} included.\n`,
        [`docs/${nera("example-app")}/notes.md`]: "Nothing here.\n",
      }),
    );
    expect(problems).toEqual([
      { path: `docs/${nera("example-app")}/notes.md`, line: 0, text: `docs/${nera("example-app")}/notes.md`, names: [nera("example-app")] },
      { path: "docs/b.md", line: 1, text: `Source: ${nera("Example-App")} web/src/index.css, its ${nera("card-shadow")} included.`, names: [nera("Example-App")] },
      {
        path: "src/a.ts",
        line: 2,
        text: `// ported from ${nera("example-service")}'s ${nera("Example-Service")} and ${nera("example-service")}`,
        names: [nera("example-service"), nera("Example-Service")],
      },
    ]);
  });

  it("passes the public nera- names, Nera alone, and binary files", () => {
    const problems = privateNameProblems(
      repository({
        "a.txt": `s3://${nera("test-fixtures")}/ var(--${nera("card-shadow")}) ${nera("token-set")} ${nera("Token-Redefined")}\n`,
        "b.txt": `${nera("only")} is not a rule; no ${nera("held")} key. Nera and nera- alone, @neralabs/rerail, corpus-nera.\n`,
        "c.bin": Buffer.concat([Buffer.from([0, 1, 2, 0]), Buffer.from(nera("example-app"))]),
      }),
    );
    expect(problems).toEqual([]);
  });

  it("flags a name that only starts with a public one", () => {
    const problems = privateNameProblems(repository({ "a.txt": `${nera("token-example")} ${nera("test-fixtures-archive")}\n` }));
    expect(problems).toEqual([
      { path: "a.txt", line: 1, text: `${nera("token-example")} ${nera("test-fixtures-archive")}`, names: [nera("token-example"), nera("test-fixtures-archive")] },
    ]);
  });

  it("keeps the ticket check apart from the name check", () => {
    const root = repository({ "a.txt": `${nera("example-app")} ${ticket("NER", 7)}\n` });
    expect(internalRefProblems(root)).toEqual([{ path: "a.txt", line: 1, text: `${nera("example-app")} ${ticket("NER", 7)}` }]);
    expect(privateNameProblems(root)).toEqual([{ path: "a.txt", line: 1, text: `${nera("example-app")} ${ticket("NER", 7)}`, names: [nera("example-app")] }]);
  });
});
