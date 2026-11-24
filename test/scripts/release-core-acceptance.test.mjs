// scripts/release/core.mjs: the core releases a pack's rules are checked against, and the oldest release
// from which every release accepts them all, which the Release pull request raises the pack's core range to.

import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { acceptedFloor, coreInstallSpecs, coreRefusals, newerFloor, packRuleDocuments, releasedCoreVersions } from "../../scripts/release/core.mjs";
import { ENGINE_PACKAGES } from "../../scripts/release/engine/packages.mjs";
import { removeFolderSync } from "../support/platform/remove-folder.ts";

const folders = [];
afterEach(() => {
  for (const folder of folders.splice(0)) removeFolderSync(folder);
});
const temporary = () => {
  const folder = mkdtempSync(join(tmpdir(), "rerail-core-acceptance-"));
  folders.push(folder);
  return folder;
};
const write = (path, text) => {
  mkdirSync(join(path, ".."), { recursive: true });
  writeFileSync(path, text);
};

describe("releasedCoreVersions", () => {
  it("lists each plain core version npm has that can check a rule (0.1.5 and later), oldest first", () => {
    const published = ["0.1.0-next.4", "0.1.0", "0.1.4", "0.2.0", "0.1.5", "0.2.1-next.1"];
    expect(releasedCoreVersions(published)).toEqual(["0.1.5", "0.2.0"]);
  });

  it("lists none before 0.1.5, which exported no validateCodeRule", () => {
    expect(releasedCoreVersions(["0.1.0", "0.1.4"])).toEqual([]);
  });
});

describe("acceptedFloor", () => {
  const refusing = (byVersion) => (version) => byVersion[version] ?? [];

  it("sets no floor when the oldest release checked accepts every rule: the older cores are the maps' to judge", () => {
    expect(acceptedFloor(["0.1.5", "0.2.0"], refusing({}), "0.2.0")).toEqual({ floor: undefined, refusals: [] });
  });

  it("is the oldest release from which every release accepts every rule, naming what each older one refuses", () => {
    expect(acceptedFloor(["0.1.5", "0.2.0"], refusing({ "0.1.5": ["eslint-max-params", "eslint-no-sequences"] }), "0.2.0")).toEqual({
      floor: "0.2.0",
      refusals: [{ version: "0.1.5", rules: ["eslint-max-params", "eslint-no-sequences"] }],
    });
  });

  it("is the core this release brings when no released core accepts the rules yet", () => {
    expect(acceptedFloor(["0.1.5"], refusing({ "0.1.5": ["eslint-max-params"] }), "0.2.0").floor).toBe("0.2.0");
  });

  it("is past every release when even the newest refuses and this release brings no newer core, so the pack is named not to merge", () => {
    expect(acceptedFloor(["0.1.5", "0.2.0"], refusing({ "0.2.0": ["regressed"] }), "0.2.0").floor).toBe("0.2.1");
  });
});

describe("newerFloor", () => {
  it("is the newer of two floors, either of which may be none", () => {
    expect(newerFloor("0.1.5", "0.2.0")).toBe("0.2.0");
    expect(newerFloor("0.2.0", undefined)).toBe("0.2.0");
    expect(newerFloor(undefined, "0.1.5")).toBe("0.1.5");
    expect(newerFloor(undefined, undefined)).toBeUndefined();
  });
});

describe("packRuleDocuments", () => {
  it("reads each code rule under rules/, nested ones by their path, and leaves out a conversation rule and a file that is not JSON", () => {
    const pack = temporary();
    write(join(pack, "rules", "a.json"), JSON.stringify({ format: "rerail.policy/v1", id: "a" }));
    write(join(pack, "rules", "group", "b.json"), JSON.stringify({ format: "rerail.policy/v1", id: "b" }));
    write(join(pack, "rules", "conversation.json"), JSON.stringify({ id: "talk", text: "Ask before deleting a branch." }));
    write(join(pack, "rules", "broken.json"), "{");
    write(join(pack, "rules", "notes.md"), "not a rule");
    expect(packRuleDocuments(pack)).toEqual({ a: { format: "rerail.policy/v1", id: "a" }, "group/b": { format: "rerail.policy/v1", id: "b" } });
  });
});

describe("coreInstallSpecs", () => {
  it("installs a core from before the engine packages by itself: it carries its engines", () => {
    expect(coreInstallSpecs("0.1.5", "linux", "x64")).toEqual(["@neralabs/rerail@0.1.5"]);
    expect(coreInstallSpecs("0.2.0", "darwin", "arm64")).toEqual(["@neralabs/rerail@0.2.0"]);
  });

  // npm alone installs all six through core's shrinkwrap; the core-refusals job installs every released core.
  it("installs a core with engine packages without the six it pins, beside the one this machine loads, at core's version", () => {
    for (const { os, cpu, name } of ENGINE_PACKAGES) {
      expect(coreInstallSpecs("0.2.1", os, cpu), name).toEqual(["--omit=optional", "@neralabs/rerail@0.2.1", `${name}@0.2.1`]);
    }
  });

  it("refuses a machine Rerail has no engine for, where the core could not check a rule", () => {
    expect(() => coreInstallSpecs("0.2.1", "freebsd", "x64")).toThrow("no engine package for freebsd x64");
  });
});

describe("coreRefusals", () => {
  it("hands a released core every document as written, multi-byte text across stdin's chunks included, and returns the ids it refuses", () => {
    const cores = temporary();
    const core = join(cores, "9.9.9", "node_modules", "@neralabs", "rerail");
    write(join(core, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: "9.9.9", type: "module", exports: { "./rules": "./rules.js" } }));
    // A stand-in core that accepts a document only when its text arrived intact.
    write(join(core, "rules.js"), 'export const validateCodeRule = (document) => ({ ok: document.text === "é".repeat(200000) && document.refuse !== true });\n');
    const text = "é".repeat(200000);
    const previous = process.env.RERAIL_RELEASE_CORES;
    process.env.RERAIL_RELEASE_CORES = cores;
    try {
      expect(coreRefusals("9.9.9", { python: { kept: { text }, refused: { text, refuse: true } }, react: { other: { text } } })).toEqual({ python: ["refused"], react: [] });
    } finally {
      if (previous === undefined) delete process.env.RERAIL_RELEASE_CORES;
      else process.env.RERAIL_RELEASE_CORES = previous;
    }
  });
});

