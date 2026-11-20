// scripts/release/core.mjs: the core releases a pack's rules are checked against, and the oldest release
// from which every release accepts them all, which the Release pull request raises the pack's core range to.

import { describe, expect, it } from "vitest";
import { acceptedFloor, newerFloor, releasedCoreVersions } from "../../scripts/release/core.mjs";

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
