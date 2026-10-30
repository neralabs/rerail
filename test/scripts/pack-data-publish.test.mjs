// scripts/s3/pack-data-publish.mjs: the publish workflow's sticky comment, and the request workflow's waiting state with
// its replace recommendation.

import { describe, expect, it } from "vitest";
import { publishComment, replaceAdvice, requestedToolPacks } from "../../scripts/s3/pack-data-publish.mjs";

/** A tool-pack as the request workflow counts it: `edits` over p/t with nothing staged and both locks. */
const toolPack = (edits = {}) => ({ pack: "p", tool: "t", files: 0, testData: 0, samples: 0, provenance: false, lost: 0, locked: { testData: 100, samples: 50 }, ...edits });
/** toolPack with `staged` test data files and provenance.json among them. */
const testData = (staged, edits = {}) => toolPack({ files: staged, testData: staged, provenance: true, ...edits });

describe("the replace recommendation", () => {
  it("is on for a whole test data set: provenance.json staged with at least half as many files as the lock counts", () => {
    const whole = replaceAdvice([testData(50)]);
    expect(whole.mode).toBe("on");
    expect(whole.reasons).toEqual(["`p/t`: on. `publish/` holds 50 test data files with `provenance.json` against 100 in `test-data.lock`: a whole set."]);
    expect(replaceAdvice([testData(1823, { locked: { testData: 1900, samples: null } })]).mode).toBe("on");
  });

  it("is off for additions or changes: fewer than half as many files as the lock counts, or no provenance.json", () => {
    const few = replaceAdvice([testData(49)]);
    expect(few.mode).toBe("off");
    expect(few.reasons).toEqual(["`p/t`: off. `publish/` holds 49 test data files against 100 in `test-data.lock`: additions or changes, not a whole set."]);
    const unrecorded = replaceAdvice([testData(90, { provenance: false })]);
    expect(unrecorded.mode).toBe("off");
    expect(unrecorded.reasons).toEqual(["`p/t`: off. `publish/` holds 90 test data files and no `provenance.json`, which a whole set carries and replace needs."]);
  });

  it("is off for removed authored rows alone", () => {
    const removed = replaceAdvice([toolPack({ lost: 2 })]);
    expect(removed.mode).toBe("off");
    expect(removed.reasons).toEqual(["`p/t`: off. 2 authored row(s) removed with nothing in `publish/`: an overlay drops their files, and a replace publishes nothing for it."]);
  });

  it("cannot tell for samples, whatever the counts, and defaults to off", () => {
    // The 669 samples staged beside 227 locked ones (#226) were additions, which a replace would have dropped.
    const samples = replaceAdvice([toolPack({ pack: "php", tool: "phpstan", files: 670, samples: 669, locked: { testData: null, samples: 227 } })]);
    expect(samples.mode).toBe("off");
    expect(samples.text).toBe("**off**, unless a sample set was rebuilt without some of its samples: nothing here marks a whole set.");
    expect(samples.reasons).toEqual([
      "`php/phpstan`: 669 samples files beside 227 in `samples.lock`. Nothing marks a whole sample set, so the counts cannot tell: off keeps the locked samples `publish/` lacks, on drops them; prep's summary counts them and warns when the mode looks wrong.",
    ]);
  });

  it("says both modes make the same archive for a tool-pack without a lock", () => {
    const fresh = replaceAdvice([toolPack({ files: 30, testData: 20, samples: 10, provenance: true, locked: { testData: null, samples: null } })]);
    expect(fresh.mode).toBe("off");
    expect(fresh.reasons).toEqual(["`p/t`: no `test-data.lock` yet, so both modes make the same test data archive.", "`p/t`: no `samples.lock` yet, so both modes make the same samples archive."]);
  });

  it("is on for samples beside a whole test data set, the samples' line saying what on does to them", () => {
    const both = replaceAdvice([testData(100, { files: 110, samples: 10 })]);
    expect(both.mode).toBe("on");
    expect(both.reasons).toHaveLength(2);
    expect(both.reasons[1]).toContain("on drops them");
  });

  it("asks to split a pull request whose tool-packs want different values", () => {
    const split = replaceAdvice([testData(100), testData(3, { pack: "q", tool: "u" })]);
    expect(split.mode).toBe("split");
    expect(split.text).toBe("**the tool-packs disagree**: one run publishes every `publish/` the same way, so publish them from separate pull requests.");
    expect(split.reasons).toHaveLength(2);
  });

  it("is on for an upstream bump's pull request, whatever it stages", () => {
    expect(replaceAdvice([testData(3)], { bump: true })).toEqual({ mode: "on", text: "**on**: an upstream bump's `publish/` holds each tool-pack's whole derived set.", reasons: [] });
  });
});

describe("the request workflow's tool-packs", () => {
  it("takes counts, flags and lock counts, and refuses anything else", () => {
    expect(requestedToolPacks([testData(3)])).toEqual([testData(3)]);
    expect(() => requestedToolPacks({})).toThrow("not a JSON array");
    expect(() => requestedToolPacks([toolPack({ pack: "../x" })])).toThrow("must be ids");
    expect(() => requestedToolPacks([toolPack({ files: -1 })])).toThrow("must be counts");
    expect(() => requestedToolPacks([toolPack({ provenance: "yes" })])).toThrow("must be true or false");
    expect(() => requestedToolPacks([toolPack({ locked: { testData: "100", samples: null } })])).toThrow("locked must give");
  });
});

describe("the waiting comment", () => {
  const request = (edits = {}) => ({
    repository: "neralabs/rerail",
    pr: "191",
    toolPacks: [testData(1823, { pack: "python", tool: "ruff", locked: { testData: 1900, samples: null } })],
    fork: false,
    bump: false,
    environment: "42",
    filesCap: "3000",
    ...edits,
  });

  it("lists each tool-pack, states the replace rule with the recommendation and its reasons, and passes the flag to gh only when on", () => {
    const text = publishComment("waiting", request());
    expect(text.startsWith("<!-- rerail-pack-data-publish state=waiting -->\n### Pack data waiting to publish\n")).toBe(true);
    expect(text).toContain("| `python/ruff` | 1823 | 0 |");
    const step = text.split("\n").find((line) => line.startsWith("3. "));
    expect(step).toContain("**replace**: on when `publish/` holds the tool-pack's whole data set");
    expect(step).toContain("On over additions drops every locked file `publish/` lacks; off over a regeneration keeps the files it deleted.");
    expect(step.endsWith(" Here: **on**.")).toBe(true);
    expect(text).toContain("\n   - `python/ruff`: on. `publish/` holds 1823 test data files with `provenance.json` against 1900 in `test-data.lock`: a whole set.\n4. ");
    expect(text).toContain("gh workflow run pack-data-publish.yml -R neralabs/rerail -f pr=191 -f replace=true\n");
    expect(text).toContain('echo \'{"environment_ids":[42],"state":"approved"');

    const off = publishComment("waiting", request({ toolPacks: [toolPack({ pack: "php", tool: "phpstan", files: 670, samples: 669, locked: { testData: null, samples: 227 } })] }));
    expect(off).toContain("Here: **off**, unless a sample set was rebuilt");
    expect(off).toContain("gh workflow run pack-data-publish.yml -R neralabs/rerail -f pr=191\n");
    expect(publishComment("waiting", request({ bump: true }))).toContain(" -f pr=191 -f replace=true\n");
    expect(publishComment("waiting", request({ toolPacks: [testData(100), testData(3, { pack: "q", tool: "u" })] }))).toContain("Here: **the tool-packs disagree**");
  });

  it("lists a fork's data as a proposal, without the publish steps", () => {
    const text = publishComment("waiting", request({ fork: true, environment: "" }));
    expect(text.startsWith("<!-- rerail-pack-data-publish state=waiting -->\n### Pack data in a proposal from a fork\n")).toBe(true);
    expect(text).toContain("| `python/ruff` | 1823 | 0 |");
    expect(text).not.toContain("Run workflow");
    expect(text).not.toContain("replace");
  });

  it("says when the pull request is too large to list, and then states the rule alone", () => {
    const text = publishComment("waiting", request({ toolPacks: [] }));
    expect(text).toContain("| (this pull request changes more than 3000 files, so its pack data cannot be listed) | | |");
    expect(text).not.toContain("Here:");
    expect(text).toContain("-f pr=191\n");
  });

  it("refuses a repository, pull request number or environment id that is not one", () => {
    expect(() => publishComment("waiting", request({ repository: "neralabs/rerail/x" }))).toThrow("is not owner/name");
    expect(() => publishComment("waiting", request({ pr: "191; rm" }))).toThrow("is not a pull request number");
    expect(() => publishComment("waiting", request({ environment: "" }))).toThrow("is not the test-data-upload environment's id");
  });
});

describe("the failed comment", () => {
  const failedLine = (outcomes) => publishComment("failed", { runUrl: "https://run", objects: [], outcomes }).split("\n").find((line) => line.startsWith("- "));

  it("blames approval only when the upload job never ran, and the step when it ran or its credentials failed", () => {
    expect(failedLine({ credentials: "", upload: "" })).toBe("- upload did not run: approval rejected or job cancelled");
    expect(failedLine({ credentials: "failure", upload: "skipped" })).toBe("- upload step failed: see the run");
    expect(failedLine({ credentials: "success", upload: "failure" })).toBe("- upload step failed: see the run");
    expect(failedLine({ credentials: "skipped", upload: "skipped" })).toBe("- upload job failed before its upload step: see the run");
  });
});
