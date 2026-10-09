import { describe, expect, it } from "vitest";
import { folderContains, normalizeFolder } from "../../../src/engine/catalog/repository-model.js";

describe("normalizeFolder", () => {
  it("normalises trailing slashes, ./ segments and backslashes", () => {
    expect(normalizeFolder("web/")).toBe("web");
    expect(normalizeFolder("./web/./src//")).toBe("web/src");
    expect(normalizeFolder(String.raw`web\src`)).toBe("web/src");
    expect(normalizeFolder("./")).toBe("");
  });

  it("rejects absolute paths and folders that leave the repository", () => {
    expect(() => normalizeFolder("/web")).toThrow(/relative/);
    expect(() => normalizeFolder("C:/web")).toThrow(/relative/);
    expect(() => normalizeFolder("web/../../etc")).toThrow(/inside/);
    expect(() => normalizeFolder("..")).toThrow(/inside/);
  });
});

describe("folderContains", () => {
  it("contains the folder itself and paths below it, not a sibling sharing its prefix", () => {
    expect(folderContains("web", "web")).toBe(true);
    expect(folderContains("web", "web/src/a.ts")).toBe(true);
    expect(folderContains("web", "web2/x")).toBe(false);
    expect(folderContains("web/src", "web/a.ts")).toBe(false);
  });

  it("treats the empty folder as the whole repository", () => {
    expect(folderContains("", "anything/at/all.ts")).toBe(true);
    expect(folderContains("", "")).toBe(true);
  });
});
