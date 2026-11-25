/**
 * A session's record (`events.jsonl`) is appended to for as long as the session lives, and read back whole each time a
 * daemon opens the session: after an update, a `rerail daemon stop` or a restart. These cases open sessions from records
 * a session could have left, of any size, and check what the open gives back and where the session's next line goes.
 */
import { constants as bufferConstants } from "node:buffer";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, open, readFile, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { readSessionLedger, RerailRuntime } from "../../../src/runtime/runtime.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { symlinkOrSkip } from "../../support/platform/links.js";

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

const SESSION = { host: "claude-code" as const, sessionId: "weeks-long", cwd: "" };
/** The line an open writes to a record that has none, which these cases read back as the session's next line. */
const TRACKING_LINE = `${JSON.stringify({ type: "hud-tracking-start" })}\n`;

interface World {
  readonly root: string;
  readonly storageRoot: string;
  readonly record: string;
}

async function world(): Promise<World> {
  const root = await mkdtemp(join(tmpdir(), "rerail-record-size-"));
  roots.push(root);
  const storageRoot = join(root, "sessions");
  const record = join(storageRoot, createHash("sha256").update(SESSION.sessionId).digest("hex"), "events.jsonl");
  await mkdir(dirname(record), { recursive: true });
  return { root, storageRoot, record };
}

function runtimeIn({ root, storageRoot }: World): RerailRuntime {
  return new RerailRuntime({
    autostart: false,
    workspaceRoot: root,
    storageRoot,
    modelPath: resolve("model"),
    databasePath: join(root, "rerail.sqlite"),
    createAuditor: () => ({
      openWorkspace: async () => undefined,
      acceptsPath: () => false,
      auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue", violations: [], failures: [] }),
      close: async () => undefined,
    }),
  });
}

/** Opens the session from its record, then the ids of the conversation rules the record restored. */
async function openedRules(at: World): Promise<string[]> {
  const runtime = runtimeIn(at);
  try {
    await runtime.ingest({ kind: "session-start", eventId: "start", session: { ...SESSION, cwd: at.root } });
    const listed = await runtime.manageRules(SESSION.sessionId, { action: "list" }) as { rules: Array<{ rule: { id: string } }> };
    return listed.rules.map(inspection => inspection.rule.id);
  } finally {
    await runtime.dispose();
  }
}

/** A record line that adds the conversation rule `id`, which an open replays. */
function ruleLine(id: string): string {
  const rule = {
    id, mode: "block", reference: "Guest checkout remains available.", match: { operator: "gte", threshold: 0.99 },
    sample: { source: "assistant-text", boundary: "message" }, action: { content: "Check checkout." },
  };
  return `${JSON.stringify({ type: "rule-mutation", mutation: { operation: "upsert", rule, replace: false } })}\n`;
}

/** What JSON.parse says of `text`, the error an open gives for a line of it. */
function jsonError(text: string): string {
  try {
    JSON.parse(text);
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
  throw new Error(`${text} is JSON`);
}

/** A padded line of `PADDED_LINE_BYTES`: an event entry and spaces after it, as `rerail scrub` pads a line it shortens. */
const PADDED_LINE_BYTES = 1024 * 1024;
function paddedLine(index: number): string {
  const entry = JSON.stringify({ type: "event", eventId: `bulk-${index}`, kind: "call", observedAt: index });
  return `${entry}${" ".repeat(PADDED_LINE_BYTES - entry.length - 1)}\n`;
}

describe("a session record of any size", () => {
  // More text than Node's longest string, in lines `rerail scrub` writes (JSON and spaces after it): its size is all the
  // case needs, and those lines keep the memory of the open small. A rule added before them and one after them come back.
  it("opens with every entry of a record that holds more text than one string can, and takes its next line after the last", async () => {
    const at = await world();
    const lines = Math.ceil(bufferConstants.MAX_STRING_LENGTH / PADDED_LINE_BYTES) + 1;
    const handle = await open(at.record, "w");
    try {
      await handle.write(ruleLine("before-the-bulk"));
      for (let index = 0; index < lines; index += 1) await handle.write(paddedLine(index));
      await handle.write(ruleLine("after-the-bulk"));
    } finally {
      await handle.close();
    }
    const written = (await stat(at.record)).size;
    expect(written).toBeGreaterThan(bufferConstants.MAX_STRING_LENGTH);

    expect(await openedRules(at)).toEqual(expect.arrayContaining(["before-the-bulk", "after-the-bulk"]));
    const end = await open(at.record, "r");
    try {
      const { size } = await end.stat();
      const tail = Buffer.alloc(size - written);
      await end.read(tail, 0, tail.length, written);
      expect(tail.toString("utf8")).toBe(TRACKING_LINE);
    } finally {
      await end.close();
    }
  }, 180_000);

  it("cuts a last line a write left unfinished off at its first byte, after more than a read's worth of multi-byte text, before the next line", async () => {
    const at = await world();
    // Two bytes a character, so the cut is at a byte offset no character count gives, and past the first read (1 MiB).
    const whole = Buffer.from(`${JSON.stringify({ type: "event", eventId: "é".repeat(700_000) })}\n${ruleLine("kept")}`);
    await writeFile(at.record, Buffer.concat([whole, Buffer.from('{"type":"event","eventId":"cut sh')]));
    expect(await openedRules(at)).toContain("kept");
    expect((await readFile(at.record)).equals(Buffer.concat([whole, Buffer.from(TRACKING_LINE)]))).toBe(true);
  });

  it("gives a whole last line with no newline its newline before the next line", async () => {
    const at = await world();
    const last = ruleLine("last").trimEnd();
    await writeFile(at.record, last);
    expect(await openedRules(at)).toContain("last");
    expect(await readFile(at.record, "utf8")).toBe(`${last}\n${TRACKING_LINE}`);
  });

  it("opens a record with empty lines among its lines", async () => {
    const at = await world();
    await writeFile(at.record, `${ruleLine("first")}\n\n${ruleLine("second")}`);
    expect(await openedRules(at)).toEqual(expect.arrayContaining(["first", "second"]));
  });

  it("fails to open a record with a line that is no JSON before its last, with JSON's own error, and writes nothing to it", async () => {
    const at = await world();
    const text = `${ruleLine("first")}not json\n${ruleLine("second")}`;
    await writeFile(at.record, text);
    await expect(openedRules(at)).rejects.toThrow(jsonError("not json"));
    expect(await readFile(at.record, "utf8")).toBe(text);
  });

  it("keeps the preimages of the calls still open and lets go of those of a call whose result came or whose id a later call took", async () => {
    const at = await world();
    const call = (toolCallId: string, text: string) => ({ type: "policy-call", toolCallId, toolName: "Edit", paths: ["a.py"], preimages: { "a.py": { text } } });
    const [closedCall, open, firstTaken, secondTaken] = [
      call("closed", "before the closed call"), call("open", "before the open call"),
      call("taken", "before the first call of the id"), call("taken", "before the second call of the id"),
    ] as const;
    const result = { type: "policy-result", toolCallId: "closed" };
    const lines = [closedCall, result, open, firstTaken, secondTaken];
    await writeFile(at.record, lines.map(line => `${JSON.stringify(line)}\n`).join(""));
    const { preimages: _closed, ...closed } = closedCall;
    const { preimages: _first, ...first } = firstTaken;
    expect(await readSessionLedger(at.record)).toEqual({
      entries: [closed, result, open, first, secondTaken], lines: 5, appendState: { prefix: "", truncateTo: null },
    });
  });

  it("refuses a record that is a symbolic link and writes nothing where it leads", async context => {
    const at = await world();
    const elsewhere = join(at.root, "elsewhere.jsonl");
    await writeFile(elsewhere, ruleLine("planted"));
    symlinkOrSkip(context, elsewhere, at.record, "file");
    await expect(openedRules(at)).rejects.toThrow(`through ${at.record}: it is a symbolic link`);
    expect(await readFile(elsewhere, "utf8")).toBe(ruleLine("planted"));
  });
});
