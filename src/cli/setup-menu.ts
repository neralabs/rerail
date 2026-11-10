import { readSync } from "node:fs";
import type { ChoiceMenu } from "../runtime/repository-start.js";

/**
 * Set-up's input at a real terminal: whole lines for the folder and pack prompts, single keys for a question's menu.
 * One reader owns stdin for the whole run, so switching between the two never leaves a second reader on it. A
 * question takes only what is typed once it is on screen: what was typed while set-up was busy (the terminal holds
 * it while stdin is paused) is dropped when the next question opens, so an Enter pressed early answers nothing.
 */

/** A key the menu acts on; `other` is ignored. */
export type MenuKey = "up" | "down" | "enter" | "interrupt" | "end" | "other" | { readonly digit: number };

/** The keys an escape sequence's final character names; any other sequence is `other`. */
const ESCAPE_KEYS: Readonly<Record<string, MenuKey>> = { A: "up", B: "down" };

/** The keys one character is. Enter is CR in raw mode and LF in line mode. */
const CHARACTER_KEYS: Readonly<Record<string, MenuKey>> = {
  "\r": "enter",
  "\n": "enter",
  "\u0003": "interrupt",
  "\u0004": "end",
  k: "up",
  j: "down",
};

/** The key at the start of `bytes`, and how many characters it takes; undefined while it may be incomplete. */
export function readKey(bytes: string): { key: MenuKey; length: number } | undefined {
  if (bytes === "") return undefined;
  if (bytes.startsWith("\u001b")) return readEscape(bytes);
  // CRLF is one Enter.
  if (bytes.startsWith("\r\n")) return { key: "enter", length: 2 };
  const first = String.fromCodePoint(bytes.codePointAt(0) ?? 0);
  const key = /^[1-9]$/u.test(first) ? { digit: Number(first) } : CHARACTER_KEYS[first] ?? "other";
  return { key, length: first.length };
}

/** The key at the start of `bytes`, which start with Escape; undefined while it may be incomplete. */
function readEscape(bytes: string): { key: MenuKey; length: number } | undefined {
  const sequence = /^\u001b[[O][0-9;]*[A-Za-z~]/u.exec(bytes)?.[0];
  if (sequence !== undefined) return { key: ESCAPE_KEYS[sequence.at(-1) ?? ""] ?? "other", length: sequence.length };
  // A lone Escape, or the start of a sequence whose rest has not arrived.
  if (/^\u001b(?:[[O][0-9;]*)?$/u.test(bytes)) return undefined;
  return { key: "other", length: 1 };
}

export type MenuAction =
  | { readonly type: "move"; readonly highlight: number }
  | { readonly type: "pick"; readonly index: number }
  | { readonly type: "cancel" }
  | { readonly type: "interrupt" }
  | { readonly type: "none" };

/**
 * What a key does with the cursor on `highlight` of `count` answers. Up and down wrap around; a digit moves the
 * cursor to that answer and Enter takes it, so `2` then Enter answers as it does where answers are typed as lines.
 */
export function menuAction(key: MenuKey, highlight: number, count: number): MenuAction {
  if (typeof key === "object") return key.digit <= count ? { type: "move", highlight: key.digit - 1 } : { type: "none" };
  switch (key) {
    case "up":
      return { type: "move", highlight: (highlight - 1 + count) % count };
    case "down":
      return { type: "move", highlight: (highlight + 1) % count };
    case "enter":
      return { type: "pick", index: highlight };
    case "end":
      return { type: "cancel" };
    case "interrupt":
      return { type: "interrupt" };
    case "other":
      return { type: "none" };
  }
}

/** The stdin `TerminalInput` reads: a TTY, or a stand-in for one in tests. */
export interface TerminalInputStream extends NodeJS.EventEmitter {
  setRawMode(raw: boolean): unknown;
  setEncoding(encoding: BufferEncoding): unknown;
  pause(): unknown;
  resume(): unknown;
}

/** Up `rows` lines to the start of the first, then clear to the end of the screen. */
const redrawFrom = (rows: number): string => `\r\u001b[${rows}A\u001b[J`;

const ANSI_SEQUENCE = /\u001b\[[0-9;?]*[ -/]*[@-~]/gu;
const ZERO_WIDTH = /^[\p{Mn}\p{Me}\p{Cf}\p{Cc}]$/u;
const WIDE = /^[\p{Emoji_Presentation}\u1100-\u115f\u2e80-\u303e\u3041-\u33ff\u3400-\u4dbf\u4e00-\u9fff\ua000-\ua4cf\uac00-\ud7a3\uf900-\ufaff\ufe30-\ufe4f\uff00-\uff60\uffe0-\uffe6\u{20000}-\u{3fffd}]$/u;

/**
 * The columns `text` takes in a terminal: none for colour codes and combining marks, two for East Asian wide characters
 * and emoji, one for the rest. Ambiguous-width characters (❯ … ’) take one, as terminals draw them by default.
 */
export function displayWidth(text: string): number {
  let width = 0;
  for (const char of text.replace(ANSI_SEQUENCE, "")) width += ZERO_WIDTH.test(char) ? 0 : WIDE.test(char) ? 2 : 1;
  return width;
}

/** The rows `frame` takes on screen: a line wider than `columns` wraps onto more; one row per line when unknown. */
function frameRows(frame: string, columns: number | undefined): number {
  const lines = frame.split("\n");
  if (columns === undefined || columns <= 0) return lines.length;
  return lines.reduce((rows, line) => rows + Math.max(1, Math.ceil(displayWidth(line) / columns)), 0);
}

/** Node's TTY stdin as it is at run time: `_handle` is its libuv handle, which Node's types leave out. */
type NodeTtyStream = TerminalInputStream & { readonly _handle?: { readonly fd?: unknown } };

/**
 * Reads and drops what the terminal holds for `input` until a read finds nothing (EAGAIN), the end, or an error,
 * through the non-blocking descriptor Node reads a TTY stdin with. A stream without one holds nothing it can drop
 * here: a stand-in, or a Windows console, whose handle has no descriptor.
 */
function dropHeldInput(input: TerminalInputStream): void {
  const stream: NodeTtyStream = input;
  const fd = stream._handle?.fd;
  if (typeof fd !== "number" || fd < 0) return;
  const chunk = Buffer.alloc(1024);
  try {
    while (readSync(fd, chunk) > 0);
  } catch {
    // EAGAIN: nothing left to read.
  }
}

export class TerminalInput {
  private buffer = "";
  private ended = false;
  private listening = false;
  private wake: (() => void) | undefined;
  /** The last line or key ended on a lone CR: an LF that comes next is the rest of that Enter (CRLF split across reads). */
  private afterCr = false;

  constructor(
    private readonly input: TerminalInputStream,
    private readonly write: (text: string) => void,
    /** Ctrl-C in the menu, after the terminal is put back: what the terminal's own Ctrl-C does. */
    private readonly interrupt: () => void = () => process.kill(process.pid, "SIGINT"),
    /** The terminal's width, to count the rows a menu's wrapped lines take; undefined when unknown. */
    private readonly columns: () => number | undefined = () => undefined,
    /** Drops what the terminal holds for stdin, with the terminal in raw mode: keys pressed while set-up was busy. */
    private readonly dropHeld: () => void = () => {
      dropHeldInput(input);
    },
  ) {}

  /** Take `length` characters off the buffer. */
  private consume(length: number): void {
    this.afterCr = this.buffer[length - 1] === "\r";
    this.buffer = this.buffer.slice(length);
  }

  /** Drop the LF of a CRLF whose CR ended the last line or key. */
  private settle(): void {
    if (!this.afterCr || this.buffer === "") return;
    this.afterCr = false;
    if (this.buffer.startsWith("\n")) this.buffer = this.buffer.slice(1);
  }

  /**
   * Drops everything typed before a question is on screen: what is left of the last read, and what the terminal holds.
   * Raw mode makes a line typed without Enter readable too, so the terminal is in it while it is drained.
   */
  private dropTypedAhead(raw: boolean): void {
    this.buffer = "";
    if (!raw) this.input.setRawMode(true);
    this.dropHeld();
    if (!raw) this.input.setRawMode(false);
  }

  private readonly onData = (chunk: string): void => {
    this.buffer += chunk;
    this.wake?.();
  };

  private readonly onEnd = (): void => {
    this.ended = true;
    this.wake?.();
  };

  /** Waits for more input, reading stdin only meanwhile. */
  private async more(): Promise<void> {
    if (!this.listening) {
      this.input.setEncoding("utf8");
      this.input.on("data", this.onData);
      this.input.on("end", this.onEnd);
      this.listening = true;
    }
    await new Promise<void>(resolve => {
      this.wake = resolve;
      this.input.resume();
    });
    this.wake = undefined;
    this.input.pause();
  }

  /**
   * One line, without its line break: CR (Enter in raw mode), LF or CRLF. At end of input the last unfinished line, or
   * undefined.
   */
  async line(prompt: string): Promise<string | undefined> {
    this.dropTypedAhead(false);
    this.write(prompt);
    for (;;) {
      this.settle();
      const end = this.buffer.search(/[\r\n]/u);
      if (end >= 0) {
        const line = this.buffer.slice(0, end);
        this.consume(end + (this.buffer.startsWith("\r\n", end) ? 2 : 1));
        return line;
      }
      if (this.ended) {
        const rest = this.buffer;
        this.buffer = "";
        return rest === "" ? undefined : rest;
      }
      await this.more();
    }
  }

  /** The menu, in raw mode until an answer is taken: its index, or undefined on Ctrl-D or end of input. */
  async choose(menu: ChoiceMenu): Promise<number | undefined> {
    const restore = (): void => {
      this.input.setRawMode(false);
    };
    this.input.setRawMode(true);
    this.dropTypedAhead(true);
    process.once("exit", restore);
    try {
      return await this.answer(menu, restore);
    } finally {
      restore();
      process.removeListener("exit", restore);
    }
  }

  /** Draws the menu and moves its cursor key by key until a key ends it; end of input cancels it. */
  private async answer(menu: ChoiceMenu, restore: () => void): Promise<number | undefined> {
    let highlight = 0;
    let frame = menu.frame(highlight);
    this.write(frame);
    for (;;) {
      const key = await this.nextKey();
      const action: MenuAction = key === undefined ? { type: "cancel" } : menuAction(key, highlight, menu.count);
      if (action.type === "none") continue;
      if (action.type !== "move") return this.end(action, restore);
      highlight = action.highlight;
      frame = this.redraw(frame, menu.frame(highlight));
    }
  }

  /** The next key, waiting for it; undefined at end of input. */
  private async nextKey(): Promise<MenuKey | undefined> {
    for (;;) {
      this.settle();
      const read = readKey(this.buffer);
      if (read !== undefined) {
        this.consume(read.length);
        return read.key;
      }
      if (this.ended) return undefined;
      await this.more();
    }
  }

  /** Draws `next` over `shown` when they differ, and returns what is on screen. */
  private redraw(shown: string, next: string): string {
    if (next === shown) return shown;
    // The cursor is on the frame's last row: up the rows above it, wrapped lines' included.
    this.write(`${redrawFrom(frameRows(shown, this.columns()) - 1)}${next}`);
    return next;
  }

  /** Ends the menu on its line: the answer taken, or undefined when cancelled or interrupted (terminal put back first). */
  private end(action: Extract<MenuAction, { type: "pick" | "cancel" | "interrupt" }>, restore: () => void): number | undefined {
    this.write("\n");
    if (action.type === "pick") return action.index;
    if (action.type === "interrupt") {
      restore();
      this.interrupt();
    }
    return undefined;
  }

  close(): void {
    this.input.removeListener("data", this.onData);
    this.input.removeListener("end", this.onEnd);
    this.input.pause();
  }
}
