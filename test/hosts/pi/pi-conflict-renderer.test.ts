import type { MessageRenderer, MessageRenderOptions, Theme } from "@earendil-works/pi-coding-agent";
import { visibleWidth } from "@earendil-works/pi-tui";
import { describe, expect, it } from "vitest";
import { conflictNote, RULE_MESSAGE_TYPE } from "../../../src/engine/index.js";
import { formatPiConflict, registerPiConflictRenderer, type ConflictTheme } from "../../../src/hosts/pi/pi-conflict-renderer.js";

/** What a Merge Assist note's details carry for the person's line (`ConflictNotice`). */
type ConflictNotice = Parameters<typeof formatPiConflict>[1];

const CARD = "http://127.0.0.1:4737/?repository=acme-web#merge-assist/conflict%3Aa";
const CONFLICT: ConflictNotice = { id: "conflict:a", text: "your edit overlaps PR #398 (Sam): model.rs · GrammarMatcher", url: CARD };
const NOTE = conflictNote({ file: "src/model.rs", peer: "PR #398", author: "Sam", summary: "Share matchers", pullRequest: true });

/** Tags each colour so the expected rows read the structure rather than escape codes. */
const theme: ConflictTheme = { fg: (colour, text) => `<${colour}>${text}</${colour}>` };
const MARK = "<mark>R E R A I L</mark>";
const link = (url: string, text: string): string => `\u001b]8;;${url}\u0007${text}\u001b]8;;\u0007`;

/** The renderer the extension registers for `rerail.rule`, with links on or off and a plain mark. */
function registered(hyperlinks: boolean): MessageRenderer {
  const renderers = new Map<string, MessageRenderer>();
  registerPiConflictRenderer({ registerMessageRenderer: (customType, renderer) => { renderers.set(customType, renderer as MessageRenderer); } }, () => hyperlinks, () => "R E R A I L");
  expect([...renderers.keys()]).toEqual([RULE_MESSAGE_TYPE]);
  return renderers.get(RULE_MESSAGE_TYPE)!;
}

const message = (details: unknown, content: string = NOTE): Parameters<MessageRenderer>[0] =>
  ({ role: "custom", customType: RULE_MESSAGE_TYPE, content, display: true, details, timestamp: 0 }) as Parameters<MessageRenderer>[0];
const options = (expanded: boolean): MessageRenderOptions => ({ expanded, outputPad: 1 }) as MessageRenderOptions;
const plainTheme = { fg: (_colour: string, text: string) => text } as Theme;
/** A rendered line as the person reads it: colours dropped, a link's text kept. */
const readable = (line: string): string => line.replace(/\u001b\[[0-9;]*m|\u001b\]8;;[^\u0007]*\u0007/gu, "").trimEnd();

describe("Pi's Merge Assist conflict line", () => {
  it("is the line Claude Code, Codex and OMP show, `open` linking the conflict's card where Pi draws links", () => {
    expect(formatPiConflict(NOTE, CONFLICT, { expanded: false, hyperlinks: true, mark: MARK }, theme)).toEqual([
      { detail: false, text: `◆ ${MARK} · your edit overlaps PR #398 (Sam): model.rs · GrammarMatcher  ·  ${link(CARD, "open")}` },
    ]);
  });

  it("prints the card's address after `open` where Pi draws no links", () => {
    expect(formatPiConflict(NOTE, CONFLICT, { expanded: false, hyperlinks: false, mark: MARK }, theme)).toEqual([
      { detail: false, text: `◆ ${MARK} · your edit overlaps PR #398 (Sam): model.rs · GrammarMatcher  ·  open → ${CARD}` },
    ]);
  });

  it("has no `open` while no dashboard linked the conflict", () => {
    const unlinked: ConflictNotice = { id: CONFLICT.id, text: CONFLICT.text };
    expect(formatPiConflict(NOTE, unlinked,{ expanded: false, hyperlinks: true, mark: MARK }, theme)).toEqual([
      { detail: false, text: `◆ ${MARK} · your edit overlaps PR #398 (Sam): model.rs · GrammarMatcher` },
    ]);
  });

  it("expanded, writes the link out and then the note the coding agent read, dimmed", () => {
    const rows = formatPiConflict("First line.\n\nSecond line.", CONFLICT, { expanded: true, hyperlinks: true, mark: MARK }, theme);
    expect(rows.slice(1)).toEqual([
      { detail: true, text: `<dim>open</dim> ${link(CARD, `<mdLink>${CARD}</mdLink>`)}` },
      { detail: true, text: "<dim>First line.</dim>" },
      { detail: true, text: "<dim>Second line.</dim>" },
    ]);
  });

  it("strips control characters from the conflict's words, its address and the note", () => {
    const hostile: ConflictNotice = { id: "conflict:b", text: "your edit overlaps x\u001b[2J\u0007 (Sam): a.ts", url: "http://127.0.0.1:4737/\u0007\u001b]8;;http://evil/" };
    const rows = formatPiConflict("note\u001b[31m red\u0007", hostile, { expanded: true, hyperlinks: true, mark: MARK }, theme);
    const escapes = rows.flatMap(row => row.text.match(/\u001b[^\]]|\u001b\]8;;[^\u0007]*\u0007/gu) ?? []);
    // The only escapes left are the link's own framing, around the stripped address.
    expect(new Set(escapes)).toEqual(new Set(["\u001b]8;;http://127.0.0.1:4737/]8;;http://evil/\u0007", "\u001b]8;;\u0007"]));
    expect(rows[0]!.text).toContain("your edit overlaps x[2J (Sam): a.ts");
    expect(rows.at(-1)!.text).toBe("<dim>note[31m red</dim>");
  });
});

describe("Pi's renderer for rerail.rule", () => {
  it("draws a Merge Assist note as its conflict's line instead of Pi's card of the agent's note", () => {
    const drawn = registered(false)(message({ requested: "steer", mechanism: "native-queue", sessionId: "s", rules: [], conflict: CONFLICT }), options(false), plainTheme);
    expect(drawn?.render(200).map(readable)).toEqual([` ◆ R E R A I L · your edit overlaps PR #398 (Sam): model.rs · GrammarMatcher  ·  open → ${CARD}`]);
  });

  it("declines a steer, or a conflict it cannot read, so Pi draws those as before", () => {
    const render = registered(true);
    expect(render(message({ requested: "steer", rules: [{ id: "no-any", state: "steered" }] }), options(false), plainTheme)).toBeUndefined();
    expect(render(message({ conflict: { id: "conflict:a" } }), options(false), plainTheme)).toBeUndefined();
    expect(render(message(undefined), options(false), plainTheme)).toBeUndefined();
  });

  it.each([true, false])("keeps every line within a narrow transcript, links %s, collapsed and expanded", hyperlinks => {
    const render = registered(hyperlinks);
    for (const expanded of [false, true]) {
      const lines = render(message({ conflict: CONFLICT }), options(expanded), plainTheme)!.render(20);
      expect(lines.length).toBeGreaterThan(1);
      for (const line of lines) expect(visibleWidth(line)).toBeLessThanOrEqual(20);
    }
  });
});
