import type { ChoiceMenu } from "../runtime/repository-start.js";

/**
 * Set-up's input at a real terminal: whole lines for the folder and pack prompts, single keys for a question's menu.
 * One reader owns stdin for the whole run, so switching between the two never leaves a second reader on it, and
 * what is typed ahead of a prompt waits in one buffer until the prompt reads it (the terminal holds what arrives
 * while stdin is paused).
 */

/** A key the menu acts on; `other` is ignored. */
export type MenuKey = "up" | "down" | "enter" | "interrupt" | "end" | "other" | { readonly digit: number };

/** The key at the start of `bytes`, and how many characters it takes; undefined while it may be incomplete. */
export function readKey(bytes: string): { key: MenuKey; length: number } | undefined {
  if (bytes === "") return undefined;
  if (bytes.startsWith("\u001b")) {
    const sequence = /^\u001b[[O][0-9;]*[A-Za-z~]/u.exec(bytes);
    if (sequence !== null) {
      const final = sequence[0].at(-1);
      return { key: final === "A" ? "up" : final === "B" ? "down" : "other", length: sequence[0].length };
    }
    // A lone Escape, or the start of a sequence whose rest has not arrived.
    if (/^\u001b(?:[[O][0-9;]*)?$/u.test(bytes)) return undefined;
    return { key: "other", length: 1 };
  }
  // Enter is CR in raw mode and LF when typed ahead in line mode; CRLF is one Enter.
  if (bytes.startsWith("\r\n")) return { key: "enter", length: 2 };
  const first = String.fromCodePoint(bytes.codePointAt(0) ?? 0);
  const length = first.length;
  if (first === "\r" || first === "\n") return { key: "enter", length };
  if (first === "\u0003") return { key: "interrupt", length };
  if (first === "\u0004") return { key: "end", length };
  if (first === "k") return { key: "up", length };
  if (first === "j") return { key: "down", length };
  if (/^[1-9]$/u.test(first)) return { key: { digit: Number(first) }, length };
  return { key: "other", length };
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
    let highlight = 0;
    let frame = menu.frame(highlight);
    this.write(frame);
    const restore = (): void => {
      this.input.setRawMode(false);
    };
    this.input.setRawMode(true);
    process.once("exit", restore);
    try {
      for (;;) {
        this.settle();
        const read = readKey(this.buffer);
        if (read === undefined) {
          if (this.ended) {
            this.write("\n");
            return undefined;
          }
          await this.more();
          continue;
        }
        this.consume(read.length);
        const action = menuAction(read.key, highlight, menu.count);
        if (action.type === "move" && action.highlight !== highlight) {
          highlight = action.highlight;
          // The cursor is on the frame's last row: up the rows above it, wrapped lines' included.
          const rows = frameRows(frame, this.columns()) - 1;
          frame = menu.frame(highlight);
          this.write(`${redrawFrom(rows)}${frame}`);
        } else if (action.type === "pick") {
          this.write("\n");
          return action.index;
        } else if (action.type === "cancel") {
          this.write("\n");
          return undefined;
        } else if (action.type === "interrupt") {
          this.write("\n");
          restore();
          this.interrupt();
          return undefined;
        }
      }
    } finally {
      restore();
      process.removeListener("exit", restore);
    }
  }

  close(): void {
    this.input.removeListener("data", this.onData);
    this.input.removeListener("end", this.onEnd);
    this.input.pause();
  }
}
