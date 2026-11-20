import { describe, expect, it } from "vitest";
import { enforcementOf } from "../../ui/render/enforcement.js";
import { enforcementCounts } from "../../ui/pack-model.js";

const stored = (mode: string, extra: object = {}) => ({ rule: { id: "r", mode }, enabled: true, configError: null, ...extra });

describe("a rule's enforcement, shown read-only", () => {
  it("reads Steer or Log only from the rule's mode", () => {
    expect(enforcementOf(stored("steer"))).toMatchObject({ kind: "steer", label: "Steer" });
    expect(enforcementOf(stored("log"))).toMatchObject({ kind: "log", label: "Log only" });
  });

  it("shows Block as Steer, which it does until blocking ships, and says so", () => {
    expect(enforcementOf(stored("block"))).toMatchObject({ kind: "steer", label: "Steer", tip: "Set to Block, which steers for now." });
  });

  it("shows a rule that waits for approval as Log only, whatever its mode", () => {
    expect(enforcementOf(stored("steer", { approval: { approved: false } }))).toMatchObject({ kind: "log" });
    expect(enforcementOf(stored("steer", { approval: { approved: true } }))).toMatchObject({ kind: "steer" });
  });
});

describe("the counts line", () => {
  it("counts the rules that steer, log only and are off, leaving out zeros", () => {
    const rules = [stored("steer"), stored("steer"), stored("log"), stored("steer", { enabled: false })];
    expect(enforcementCounts(rules)).toBe("2 steer · 1 log only · 1 off");
    expect(enforcementCounts([stored("log")])).toBe("1 log only");
  });
});
