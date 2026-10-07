import { describe, expect, it } from "vitest";
import {
  InMemoryWatermarkStore,
  SemanticWorkspace,
} from "../src/semantic-primitives.js";
import type {
  ContentSource,
  SemanticEmbedder,
  Watermark,
} from "../src/semantic-types.js";

const source: ContentSource = {
  id: "messages",
  async read() {
    return {
      id: "messages@3",
      items: [
        {
          ref: { source: "messages", id: "m1" },
          text: "Alpha beta gamma",
          metadata: { owner: "tim", session: "s1", timestamp: 10 },
        },
        {
          ref: { source: "messages", id: "m2" },
          text: "Beta plan",
          metadata: { owner: "agent", session: "s1", timestamp: 20 },
        },
        {
          ref: { source: "messages", id: "m3" },
          text: "Alpha details",
          metadata: { owner: "agent", session: "s2", timestamp: 30 },
        },
      ],
    };
  },
};

const embedder: SemanticEmbedder = {
  fingerprint() {
    return {
      modelId: "test",
      revision: "1",
      dimensions: 2,
      weightsSha256: "weights",
      tokenizerSha256: "tokens",
      kernelFormatVersion: 1,
    };
  },
  async embed(texts) {
    return texts.map(text => {
      const alpha = (text.match(/alpha/gi) ?? []).length;
      const beta = (text.match(/beta/gi) ?? []).length;
      const length = Math.hypot(alpha, beta) || 1;
      return Float32Array.from([alpha / length, beta / length]);
    });
  },
  async fitToTokenLimit(texts, maximumTokens, overflow) {
    return texts.map(text => {
      const tokens = text.split(/\s+/);
      if (tokens.length <= maximumTokens) return text;
      if (overflow === "reject") throw new Error("token limit exceeded");
      return (overflow === "truncate-start" ? tokens.slice(-maximumTokens) : tokens.slice(0, maximumTokens)).join(" ");
    });
  },
};

function watermark(key: string, value?: string): Watermark {
  return {
    namespace: "experiment",
    key,
    ...(value === undefined ? {} : { value }),
    owner: "tim",
    visibility: "private",
    createdAt: 100,
  };
}

describe("semantic primitives", () => {
  it("marks addressable spans and resolves boolean watermark selections", async () => {
    const workspace = new SemanticWorkspace({
      sources: [source],
      watermarks: new InMemoryWatermarkStore(),
      embedder,
      now: () => 150,
    });
    const receipt = await workspace.mark({
      kind: "attach",
      targets: [
        { source: "messages", id: "m1", span: { start: 0, end: 5 } },
        { source: "messages", id: "m2" },
      ],
      watermarks: [watermark("candidate", "yes")],
    });
    await workspace.mark({
      kind: "attach",
      targets: [{ source: "messages", id: "m2" }],
      watermarks: [watermark("discard")],
    });

    const selected = await workspace.select({
      where: { mark: { namespace: "experiment", key: "candidate", value: "yes" } },
      exclude: { mark: { key: "discard" } },
      order: "time-ascending",
    });

    expect(selected.items.map(item => [item.ref, item.text])).toEqual([
      [
        { source: "messages", id: "m1", span: { start: 0, end: 5 } },
        "Alpha",
      ],
    ]);
    expect(selected.snapshot).toContain("messages@3");

    await workspace.mark({ kind: "detach", assignmentIds: receipt.assignmentIds });
    expect(
      (
        await workspace.select({
          where: { mark: { key: "candidate" } },
        })
      ).items,
    ).toEqual([]);
  });

  it("projects selected content with lineage and relates top matches", async () => {
    const workspace = new SemanticWorkspace({
      sources: [source],
      watermarks: new InMemoryWatermarkStore(),
      embedder,
    });
    const selection = await workspace.select({ order: "source" });
    const itemProjection = await workspace.project(selection, {
      unit: { kind: "item" },
      render: { kind: "text-only" },
    });
    const wholeProjection = await workspace.project(selection, {
      unit: { kind: "whole-selection" },
      render: { kind: "template", template: "{{owner}}: {{text}}" },
    });

    expect(itemProjection.vectors).toHaveLength(3);
    expect(itemProjection.vectors[0]).toMatchObject({
      members: [{ source: "messages", id: "m1" }],
      renderedText: "Alpha beta gamma",
    });
    expect(wholeProjection.vectors).toHaveLength(1);
    expect(wholeProjection.vectors[0]?.renderedText).toBe(
      "tim: Alpha beta gamma\nagent: Beta plan\nagent: Alpha details",
    );
    expect(wholeProjection.lineage.selectionSnapshot).toBe(selection.snapshot);

    const relationSet = await workspace.relate(itemProjection, itemProjection, {
      pairing: { kind: "top-k", k: 1 },
      excludeSelf: true,
    });
    const relations = relationSet.pairs;
    const windows = await workspace.project(selection, {
      unit: { kind: "window", size: 2, stride: 1 },
      render: { kind: "text-only" },
      limit: { maximumTokens: 3, overflow: "truncate-end" },
    });
    expect(windows.vectors.map(vector => ({
      members: vector.members.map(member => member.id),
      renderedText: vector.renderedText,
      groupKey: vector.groupKey,
    }))).toEqual([
      {
        members: ["m1", "m2"],
        renderedText: "Alpha beta gamma",
        groupKey: "window:0:2",
      },
      {
        members: ["m2", "m3"],
        renderedText: "Beta plan Alpha",
        groupKey: "window:1:3",
      },
    ]);
    const grouped = await workspace.project(selection, {
      unit: { kind: "group", by: "owner" },
      render: { kind: "text-only" },
    });
    const corresponding = await workspace.relate(grouped, grouped, {
      pairing: { kind: "corresponding-groups" },
      reduction: { kind: "mean", groupBy: "all" },
    });
    expect(corresponding.pairs).toHaveLength(2);
    expect(corresponding.reductions).toEqual([
      { key: "all", similarity: expect.closeTo(1, 5), count: 2 },
    ]);
    const weighted = await workspace.relate(grouped, grouped, {
      pairing: { kind: "corresponding-groups" },
      reduction: {
        kind: "weighted-mean",
        groupBy: "all",
        weight: relation => relation.leftMembers.length,
      },
    });
    expect(weighted.reductions[0]).toEqual({
      key: "all",
      similarity: expect.closeTo(1, 5),
      count: 2,
    });


    expect(relations).toHaveLength(3);
    expect(relations[0]).toMatchObject({
      left: itemProjection.vectors[0]?.id,
      right: itemProjection.vectors[1]?.id,
      similarity: expect.closeTo(Math.SQRT1_2, 5),
    });
    expect(relations[2]).toMatchObject({
      left: itemProjection.vectors[2]?.id,
      right: itemProjection.vectors[0]?.id,
      similarity: expect.closeTo(Math.SQRT1_2, 5),
    });
  });
});
