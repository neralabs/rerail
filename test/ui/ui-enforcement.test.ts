import { describe, expect, it } from "vitest";
import { enforcementOf, modeEnforcementOf } from "../../ui/render/enforcement.js";
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

  it("shows a rule switched on that runs nowhere as Not running, with why, and keeps its mode for when it runs", () => {
    const reason = "This Rerail no longer runs the rule as JavaScript 0.1.9 writes it: it tests a token that parsed files now leave out. The pack's next version runs it again.";
    const notRun = stored("steer", { inactive: { code: "reads-left-out-token", reason } });
    expect(enforcementOf(notRun)).toEqual({ kind: "none", glyph: null, label: "Not running", tip: reason });
    expect(modeEnforcementOf(notRun)).toMatchObject({ kind: "steer", label: "Steer" });
    expect(enforcementOf(stored("log", { configError: "`opton` is not an option" }))).toMatchObject({ kind: "none", tip: "Its options don't apply: `opton` is not an option" });
    // Switched off, it shows the mode it would run in, as any rule switched off does.
    expect(enforcementOf(stored("steer", { enabled: false, inactive: { code: "reads-left-out-token", reason } }))).toMatchObject({ kind: "steer" });
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
