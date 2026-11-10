import { createHash } from "node:crypto";
import { cp, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { runInNewContext } from "node:vm";
import { setFlagsFromString } from "node:v8";
import { describe, expect, it } from "vitest";
import { ConversationRuleRegistry } from "../../../src/engine/catalog/registry.js";
import type { ConversationRule } from "../../../src/engine/catalog/rule-schema.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

setFlagsFromString("--expose-gc");
const gc = runInNewContext("gc") as () => void;

function rule(overrides: Partial<ConversationRule> = {}): ConversationRule {
  return {
    id: "checkout",
    mode: "steer",
    reference: "Guest checkout remains available.",
    match: { operator: "gte", threshold: 0.99 },
    sample: {
      source: "assistant-text",
      boundary: "message",
      minimumTokens: 1,
    },
    action: { content: "Check checkout requirements." },
    repeat: { mode: "always" },
    ...overrides,
  };
}

describe("ConversationRuleRegistry", () => {
  it("returns configured actions with native conversation rule match facts", () => {
    const registry = new ConversationRuleRegistry({ modelPath: resolve("model") });
    registry.replace([rule()]);

    const matches = registry.match({
      source: "assistant-text",
      boundary: "message",
      text: "Guest checkout remains available.",
    });

    expect(matches).toEqual([
      expect.objectContaining({
        ruleId: "checkout",
        similarity: 1,
        mode: "steer",
        action: { content: "Check checkout requirements." },
      }),
    ]);
  });

  it("keeps the active registry when replacement validation fails", () => {
    const registry = new ConversationRuleRegistry({ modelPath: resolve("model") });
    registry.replace([rule()]);

    expect(() =>
      registry.replace([
        rule({
          id: "invalid",
          match: { operator: "gte", threshold: 2 },
        }),
      ]),
    ).toThrow(/threshold/i);

    expect(registry.inspect().map(item => item.rule.id)).toEqual(["checkout"]);
  });

  it("applies once and after-gap repeat policies only after delivery", () => {
    const registry = new ConversationRuleRegistry({ modelPath: resolve("model") });
    registry.replace([
      rule({ id: "once", repeat: { mode: "once" } }),
      rule({ id: "gap", repeat: { mode: "after-gap", gapTurns: 3 } }),
    ]);
    const sample = {
      source: "assistant-text",
      boundary: "message",
      text: "Guest checkout remains available.",
    } as const;

    expect(registry.match(sample, { turn: 1 }).map(match => match.ruleId)).toEqual(["gap", "once"]);
    registry.markDelivered(["gap", "once"], 1);
    expect(registry.match(sample, { turn: 2 })).toEqual([]);
    expect(registry.match(sample, { turn: 4 }).map(match => match.ruleId)).toEqual(["gap"]);
    expect(registry.match(sample, { turn: 2, respectRepeat: false }).map(match => match.ruleId)).toEqual([
      "gap",
      "once",
    ]);
  });

  it("holds its model once among all registries of that model, each with its own rules, since every session opens one", () => {
    // The first load also pays for the native module and its one-time state.
    const first = new ConversationRuleRegistry({ modelPath: resolve("model") });
    first.replace([rule()]);
    gc();
    const before = process.memoryUsage().rss;
    const count = 10;
    const registries = Array.from({ length: count }, () => new ConversationRuleRegistry({ modelPath: resolve("model") }));
    gc();
    const perRegistry = (process.memoryUsage().rss - before) / count;
    // A loaded copy of the model's weights and tokenizers is about 14 MB; a registry that shares it adds its rule set.
    expect(perRegistry / 1e6).toBeLessThan(2);
    const sample = { source: "assistant-text", boundary: "message", text: "Guest checkout remains available." } as const;
    expect(first.match(sample).map(match => match.ruleId)).toEqual(["checkout"]);
    expect(registries.map(registry => registry.match(sample))).toEqual(registries.map(() => []));
  });

  it("loads its model folder anew once the manifest names other files, while a registry of the old files keeps them", async () => {
    const folder = await mkdtemp(join(tmpdir(), "rerail-model-"));
    try {
      await cp(resolve("model"), folder, { recursive: true });
      const held = new ConversationRuleRegistry({ modelPath: folder });
      const before = held.fingerprint().tokenizerSha256;
      // The same tokenizer written out again: other bytes, so another checksum in the manifest, and the same tokens.
      const tokenizer = JSON.stringify(JSON.parse(await readFile(join(folder, "tokenizer.json"), "utf8")));
      await writeFile(join(folder, "tokenizer.json"), tokenizer);
      const after = createHash("sha256").update(tokenizer).digest("hex");
      const manifest = JSON.parse(await readFile(join(folder, "manifest.json"), "utf8")) as { files: Record<string, { sha256: string }> };
      manifest.files["tokenizer.json"]!.sha256 = after;
      await writeFile(join(folder, "manifest.json"), JSON.stringify(manifest));
      expect(new ConversationRuleRegistry({ modelPath: folder }).fingerprint().tokenizerSha256).toBe(after);
      expect(held.fingerprint().tokenizerSha256).toBe(before);
      expect(after).not.toBe(before);
    } finally {
      await removeFolder(folder);
    }
  });
});
