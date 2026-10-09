import { describe, expect, it } from "vitest";
import {
  OmpMessageContentSource,
  OmpWatermarkStore,
  OMP_WATERMARK_ENTRY_TYPE,
  type OmpSessionEntryLike,
} from "../../src/runtime/omp-semantic.js";
import type { Watermark } from "../../src/engine/index.js";

function mark(): Watermark {
  return {
    namespace: "review",
    key: "embed",
    value: true,
    owner: "tim",
    visibility: "project",
    createdAt: 100,
  };
}

describe("OMP semantic adapters", () => {
  it("exposes textual branch messages as addressable content", async () => {
    const entries: OmpSessionEntryLike[] = [
      {
        type: "message",
        id: "u1",
        timestamp: "2026-01-01T00:00:00.000Z",
        message: { role: "user", content: "Design the selector." },
      },
      {
        type: "message",
        id: "a1",
        timestamp: "2026-01-01T00:00:01.000Z",
        message: {
          role: "assistant",
          content: [
            { type: "thinking", thinking: "not projected" },
            { type: "text", text: "Use an append-only watermark ledger." },
          ],
        },
      },
      {
        type: "custom",
        id: "state1",
        timestamp: "2026-01-01T00:00:02.000Z",
        customType: "unrelated",
        data: {},
      },
    ];
    const source = new OmpMessageContentSource({
      sessionId: "session-1",
      entries: () => entries,
    });

    await expect(source.read()).resolves.toEqual({
      id: "session-1@state1",
      items: [
        {
          ref: { source: "omp:message", id: "u1" },
          text: "Design the selector.",
          metadata: {
            owner: "user",
            session: "session-1",
            timestamp: Date.parse("2026-01-01T00:00:00.000Z"),
            fields: { role: "user" },
          },
        },
        {
          ref: { source: "omp:message", id: "a1" },
          text: "Use an append-only watermark ledger.",
          metadata: {
            owner: "assistant",
            session: "session-1",
            timestamp: Date.parse("2026-01-01T00:00:01.000Z"),
            fields: { role: "assistant" },
          },
        },
      ],
    });
  });

  it("persists watermark mutations as replayable OMP custom entries", async () => {
    const entries: OmpSessionEntryLike[] = [];
    let nextId = 1;
    const store = new OmpWatermarkStore({
      entries: () => entries,
      appendEntry(customType, data) {
        entries.push({
          type: "custom",
          id: `custom-${nextId++}`,
          timestamp: new Date().toISOString(),
          customType,
          data,
        });
      },
    });

    const attached = await store.apply({
      kind: "attach",
      targets: [{ source: "omp:message", id: "u1" }],
      watermarks: [mark()],
    });
    expect(entries[0]).toMatchObject({
      type: "custom",
      customType: OMP_WATERMARK_ENTRY_TYPE,
    });
    expect((await store.read()).assignments).toEqual([
      expect.objectContaining({
        id: attached.assignmentIds[0],
        target: { source: "omp:message", id: "u1" },
        watermark: mark(),
      }),
    ]);

    await store.apply({ kind: "detach", assignmentIds: attached.assignmentIds });
    await expect(store.read()).resolves.toEqual({
      revision: "custom-2",
      assignments: [],
    });
  });
});
