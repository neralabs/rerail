import { randomUUID } from "node:crypto";
import type {
  ContentSnapshot,
  ContentSource,
  MarkReceipt,
  WatermarkAssignment,
  WatermarkChange,
  WatermarkSnapshot,
  WatermarkStore,
} from "./semantic-types.js";

export const OMP_WATERMARK_ENTRY_TYPE = "rerail.watermark";

interface OmpMessageLike {
  role: string;
  content?: unknown;
}

interface OmpEntryBaseLike {
  type: string;
  id: string;
  timestamp: string;
}

export type OmpSessionEntryLike =
  | (OmpEntryBaseLike & { type: "message"; message: OmpMessageLike })
  | (OmpEntryBaseLike & { type: "custom"; customType: string; data?: unknown })
  | OmpEntryBaseLike;

function messageText(message: OmpMessageLike): string {
  if (typeof message.content === "string") return message.content;
  if (!Array.isArray(message.content)) return "";
  return message.content
    .flatMap(block => {
      if (typeof block === "string") return [block];
      if (
        block !== null &&
        typeof block === "object" &&
        "type" in block &&
        block.type === "text" &&
        "text" in block &&
        typeof block.text === "string"
      ) {
        return [block.text];
      }
      return [];
    })
    .join("\n");
}

export class OmpMessageContentSource implements ContentSource {
  readonly id: string;
  readonly #sessionId: string;
  readonly #entries: () => readonly OmpSessionEntryLike[];

  constructor(options: {
    sessionId: string;
    entries: () => readonly OmpSessionEntryLike[];
    sourceId?: string;
  }) {
    if (options.sessionId.trim() === "") throw new Error("OMP session id must be non-empty");
    this.id = options.sourceId ?? "omp:message";
    this.#sessionId = options.sessionId;
    this.#entries = options.entries;
  }

  async read(): Promise<ContentSnapshot> {
    const entries = this.#entries();
    const items = entries.flatMap(entry => {
      if (entry.type !== "message" || !("message" in entry)) return [];
      const text = messageText(entry.message);
      if (text.trim() === "") return [];
      const parsedTimestamp = Date.parse(entry.timestamp);
      return [
        {
          ref: { source: this.id, id: entry.id },
          text,
          metadata: {
            owner: entry.message.role,
            session: this.#sessionId,
            ...(Number.isFinite(parsedTimestamp) ? { timestamp: parsedTimestamp } : {}),
            fields: { role: entry.message.role },
          },
        },
      ];
    });
    return {
      id: `${this.#sessionId}@${entries.at(-1)?.id ?? "empty"}`,
      items,
    };
  }
}

interface PersistedAttach {
  version: 1;
  kind: "attach";
  assignments: WatermarkAssignment[];
}

interface PersistedDetach {
  version: 1;
  kind: "detach";
  assignmentIds: string[];
}

type PersistedWatermarkChange = PersistedAttach | PersistedDetach;
function persistedChange(data: unknown): PersistedWatermarkChange | undefined {
  if (data === null || typeof data !== "object") return undefined;
  const candidate = data as Partial<PersistedWatermarkChange>;
  if (candidate.version !== 1) return undefined;
  if (candidate.kind === "attach" && Array.isArray(candidate.assignments)) {
    return candidate as PersistedAttach;
  }
  if (
    candidate.kind === "detach" &&
    Array.isArray(candidate.assignmentIds) &&
    candidate.assignmentIds.every(id => typeof id === "string" && id.trim() !== "")
  ) {
    return candidate as PersistedDetach;
  }
  return undefined;
}

export class OmpWatermarkStore implements WatermarkStore {
  readonly #entries: () => readonly OmpSessionEntryLike[];
  readonly #appendEntry: (customType: string, data: PersistedWatermarkChange) => void;

  constructor(options: {
    entries: () => readonly OmpSessionEntryLike[];
    appendEntry: (customType: string, data: PersistedWatermarkChange) => void;
  }) {
    this.#entries = options.entries;
    this.#appendEntry = options.appendEntry;
  }

  async apply(change: WatermarkChange): Promise<MarkReceipt> {
    let assignmentIds: string[];
    if (change.kind === "attach") {
      const assignments = change.targets.flatMap(target =>
        change.watermarks.map(watermark => ({
          id: randomUUID(),
          target: structuredClone(target),
          watermark: structuredClone(watermark),
        })),
      );
      if (assignments.length === 0) return { revision: (await this.read()).revision, assignmentIds: [] };
      assignmentIds = assignments.map(assignment => assignment.id);
      this.#appendEntry(OMP_WATERMARK_ENTRY_TYPE, { version: 1, kind: "attach", assignments });
    } else {
      const active = new Set((await this.read()).assignments.map(assignment => assignment.id));
      assignmentIds = change.assignmentIds.filter(id => active.has(id));
      if (assignmentIds.length === 0) return { revision: (await this.read()).revision, assignmentIds: [] };
      this.#appendEntry(OMP_WATERMARK_ENTRY_TYPE, { version: 1, kind: "detach", assignmentIds });
    }
    return { revision: (await this.read()).revision, assignmentIds };
  }

  async read(): Promise<WatermarkSnapshot> {
    const assignments = new Map<string, WatermarkAssignment>();
    let revision = "0";
    for (const entry of this.#entries()) {
      if (entry.type !== "custom" || !("customType" in entry) || entry.customType !== OMP_WATERMARK_ENTRY_TYPE) continue;
      const change = persistedChange(entry.data);
      if (change === undefined) throw new Error(`Invalid OMP watermark entry: ${entry.id}`);
      revision = entry.id;
      if (change.kind === "attach") {
        for (const assignment of change.assignments) assignments.set(assignment.id, assignment);
      } else {
        for (const id of change.assignmentIds) assignments.delete(id);
      }
    }
    return { revision, assignments: [...assignments.values()] };
  }
}
