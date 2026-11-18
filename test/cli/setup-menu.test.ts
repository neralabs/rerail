import { EventEmitter, once } from "node:events";
import { setImmediate as setImmediatePromise } from "node:timers/promises";
import { describe, expect, it, vi } from "vitest";
import { displayWidth, menuAction, readKey, TerminalInput, type TerminalInputStream } from "../../src/cli/setup-menu.js";

describe("the set-up menu's keys", () => {
  it("reads arrows, Enter, digits, j and k, Ctrl-C and Ctrl-D", () => {
    expect(readKey("\u001b[A")).toEqual({ key: "up", length: 3 });
    expect(readKey("\u001bOB")).toEqual({ key: "down", length: 3 });
    expect(readKey("\u001b[C")).toEqual({ key: "other", length: 3 });
    expect(readKey("k")).toEqual({ key: "up", length: 1 });
    expect(readKey("j")).toEqual({ key: "down", length: 1 });
    expect(readKey("\r")).toEqual({ key: "enter", length: 1 });
    expect(readKey("\n")).toEqual({ key: "enter", length: 1 });
    expect(readKey("\r\n")).toEqual({ key: "enter", length: 2 });
    expect(readKey("2\r")).toEqual({ key: { digit: 2 }, length: 1 });
    expect(readKey("0")).toEqual({ key: "other", length: 1 });
    expect(readKey("\u0003")).toEqual({ key: "interrupt", length: 1 });
    expect(readKey("\u0004")).toEqual({ key: "end", length: 1 });
    expect(readKey("é")).toEqual({ key: "other", length: 1 });
  });

  it("waits for the rest of an escape sequence", () => {
    expect(readKey("")).toBeUndefined();
    expect(readKey("\u001b")).toBeUndefined();
    expect(readKey("\u001b[")).toBeUndefined();
    expect(readKey("\u001bx")).toEqual({ key: "other", length: 1 });
  });

  it("wraps the cursor around, moves it to a digit's answer, and takes the answer it is on at Enter", () => {
    expect(menuAction("up", 0, 3)).toEqual({ type: "move", highlight: 2 });
    expect(menuAction("down", 2, 3)).toEqual({ type: "move", highlight: 0 });
    expect(menuAction("down", 0, 3)).toEqual({ type: "move", highlight: 1 });
    expect(menuAction({ digit: 3 }, 0, 3)).toEqual({ type: "move", highlight: 2 });
    expect(menuAction({ digit: 4 }, 0, 3)).toEqual({ type: "none" });
    expect(menuAction("enter", 1, 3)).toEqual({ type: "pick", index: 1 });
    expect(menuAction("end", 1, 3)).toEqual({ type: "cancel" });
    expect(menuAction("interrupt", 1, 3)).toEqual({ type: "interrupt" });
    expect(menuAction("other", 1, 3)).toEqual({ type: "none" });
  });
});

/** How many turns of the event loop a typed chunk comes after the last, or after stdin starts reading: more than the drop's. */
const TURNS_TO_TYPE = 5;

/**
 * A terminal's stdin. `chunks` are what the person types, one at a time while stdin reads, each {@link TURNS_TO_TYPE}
 * turns of the event loop after the last; then the end of input when `ends`. `press` types more. A key pressed while
 * stdin is paused the terminal holds, and hands over those turns after stdin reads again, as a Windows console hands
 * over its keys later than a turn. `unfinished` is a line typed without Enter: the terminal hands it over only to a
 * read started in raw mode, as macOS does, which tells a read already waiting nothing when raw mode makes it readable.
 */
function fakeStdin(chunks: readonly string[], { ends = true } = {}): TerminalInputStream & { raw: boolean[]; unfinished: string; press(chunk: string): void } {
  const keys = [...chunks];
  let reading = false;
  let ended = false;
  let typing: NodeJS.Immediate | undefined;
  // Turns of the event loop, not a time: the drop here waits turns (`terminalInput`), and what this models is a key
  // that comes after them, as a person's does, however long the machine holds this process meanwhile.
  const type = (turns = 1): void => {
    if (!reading || ended || typing !== undefined || (keys.length === 0 && !ends)) return;
    typing = setImmediate(() => {
      typing = undefined;
      if (turns < TURNS_TO_TYPE) {
        type(turns + 1);
        return;
      }
      const chunk = keys.shift();
      if (chunk === undefined) {
        ended = true;
        stream.emit("end");
        return;
      }
      stream.emit("data", chunk);
      type();
    });
  };
  const stream = Object.assign(new EventEmitter(), {
    raw: [] as boolean[],
    unfinished: "",
    press(chunk: string) {
      keys.push(chunk);
      type();
    },
    setRawMode(raw: boolean) {
      stream.raw.push(raw);
    },
    setEncoding() {},
    pause() {
      reading = false;
      clearImmediate(typing);
      typing = undefined;
    },
    resume() {
      reading = true;
      const line = stream.unfinished;
      if (stream.raw.at(-1) === true && line !== "") {
        stream.unfinished = "";
        queueMicrotask(() => stream.emit("data", line));
      }
      type();
    },
  });
  return stream;
}

/** Set-up's input on `stdin`, whose drop waits a turn of the event loop for quiet, as the stand-in types in turns. */
function terminalInput(
  stdin: TerminalInputStream,
  write: (text: string) => void = () => {},
  interrupt: () => void = () => {},
  columns: () => number | undefined = () => undefined,
): TerminalInput {
  return new TerminalInput(stdin, write, interrupt, columns, async () => { await setImmediatePromise(); });
}

const menu = { count: 3, frame: (highlight: number) => [0, 1, 2].map(index => `${index === highlight ? "❯" : " "} ${index + 1}`).concat("", "> ").join("\n") };

describe("the terminal's input", () => {
  it("moves the cursor with the arrows, redrawing the menu in place, and takes the answer at Enter", async () => {
    const stdin = fakeStdin(["\u001b[B", "\u001b[B\u001b[A", "\r"]);
    const written: string[] = [];
    const input = terminalInput(stdin, text => { written.push(text); });
    expect(await input.choose(menu)).toBe(1);
    expect(stdin.raw).toEqual([true, false]);
    // Up four lines (two rows above the last, the blank line, the prompt) to the first answer, then drawn again.
    expect(written[1]).toBe(`\r\u001b[4A\u001b[J${menu.frame(1)}`);
    expect(written.at(-1)).toBe("\n");
  });

  it("redraws a menu whose rows wrap at a narrow terminal from its first row, counting the wrapped rows", async () => {
    const stdin = fakeStdin(["\u001b[B", "\r"]);
    const written: string[] = [];
    // At 20 columns: "❯ 1. Yes, scan customer-portal-frontend" (40 wide) takes two rows, each of the others one.
    const long = {
      count: 2,
      frame: (highlight: number) => [`${highlight === 0 ? "\u001b[36m❯\u001b[39m" : " "} 1. Yes, scan customer-portal-frontend`, `${highlight === 1 ? "❯" : " "} 2. No`, "", "> "].join("\n"),
    };
    const input = terminalInput(stdin, text => { written.push(text); }, () => {}, () => 20);
    expect(await input.choose(long)).toBe(1);
    // Up four rows from the prompt's: the blank line, the second answer, and the wrapped first answer's two.
    expect(written[1]).toBe(`\r\u001b[4A\u001b[J${long.frame(1)}`);
  });

  it("redraws as one row per line when the terminal's width is unknown", async () => {
    const written: string[] = [];
    const input = terminalInput(fakeStdin(["\u001b[B", "\r"]), text => { written.push(text); });
    expect(await input.choose(menu)).toBe(1);
    expect(written[1]).toBe(`\r\u001b[4A\u001b[J${menu.frame(1)}`);
  });

  it("measures text as the terminal draws it: no width for colour codes, two columns for wide characters", () => {
    expect(displayWidth("\u001b[36m❯\u001b[39m 1. Yes")).toBe(8);
    expect(displayWidth("Scan a different repo… it’s")).toBe(27);
    expect(displayWidth("日本")).toBe(4);
    expect(displayWidth("e\u0301")).toBe(1);
  });

  it("ends a line at CR, as raw mode sends Enter, at LF, and at CRLF split across reads", async () => {
    const input = terminalInput(fakeStdin(["1\r", "~/proj\r"]));
    expect(await input.choose(menu)).toBe(0);
    expect(await input.line("Which folder? ")).toBe("~/proj");
    expect(await input.line("Which folder? ")).toBeUndefined();

    const split = terminalInput(fakeStdin(["~/a\r", "\n~/b\n", "\r\n"]));
    expect(await split.line("Which folder? ")).toBe("~/a");
    expect(await split.line("Which folder? ")).toBe("~/b");
    expect(await split.line("Which folder? ")).toBe("");
  });

  it("takes no Enter for a menu from the LF of a CRLF a line ended on", async () => {
    const input = terminalInput(fakeStdin(["~/a\r", "\n", "\u001b[B", "\r"]));
    expect(await input.line("Which folder? ")).toBe("~/a");
    expect(await input.choose(menu)).toBe(1);
  });

  it("takes a digit then Enter, then reads the next prompt's line", async () => {
    const input = terminalInput(fakeStdin(["3\r", "my-app\n"]));
    expect(await input.choose(menu)).toBe(2);
    expect(await input.line("Which folder? ")).toBe("my-app");
  });

  it("answers a question only with what is typed once it is on screen, not with what was left of an earlier read", async () => {
    // A second Enter in the read that answered the first question does not answer the next one.
    const input = terminalInput(fakeStdin(["2\r\r", "\u001b[B", "\r", "3\rmy-app\n", "~/code/app\n"]));
    expect(await input.choose(menu)).toBe(1);
    expect(await input.choose(menu)).toBe(1);
    expect(await input.choose(menu)).toBe(2);
    expect(await input.line("Which folder? ")).toBe("~/code/app");
  });

  it("drops what was typed before a menu or prompt is drawn: keys pressed while set-up was busy, and a line without Enter", async () => {
    const stdin = fakeStdin([], { ends: false });
    // What stdin read, in raw mode or not, and what was written, in order.
    const seen: string[] = [];
    stdin.on("data", (chunk: string) => { seen.push(`read ${JSON.stringify(chunk)}${stdin.raw.at(-1) === true ? " in raw mode" : ""}`); });
    const input = terminalInput(stdin, text => { seen.push(`wrote ${JSON.stringify(text)}`); });
    // An Enter pressed while set-up is busy, as during the download, and read before set-up asks.
    stdin.press("\r");
    await once(stdin, "data");
    const picked = input.choose(menu);
    stdin.press("\u001b[B");
    stdin.press("\r");
    expect(await picked).toBe(1);
    // A folder typed while set-up was busy again, without Enter.
    stdin.unfinished = "~/typed-early";
    const folder = input.line("Which folder? ");
    stdin.press("~/code/app\n");
    expect(await folder).toBe("~/code/app");
    input.close();
    // Each was read, and dropped, before anything of its question was drawn; the keys that answer came after.
    expect(seen).toEqual([
      'read "\\r"',
      `wrote ${JSON.stringify(menu.frame(0))}`,
      'read "\\u001b[B" in raw mode',
      `wrote ${JSON.stringify(`\r\u001b[4A\u001b[J${menu.frame(1)}`)}`,
      'read "\\r" in raw mode',
      'wrote "\\n"',
      'read "~/typed-early" in raw mode',
      'wrote "Which folder? "',
      'read "~/code/app\\n"',
    ]);
    // The prompt is in raw mode only while it drops them.
    expect(stdin.raw).toEqual([true, false, true, false]);
  });

  it("cancels the next question at an end of input (Ctrl-D) typed while set-up was busy, rather than waiting on it", async () => {
    const stdin = fakeStdin([]);
    const input = terminalInput(stdin);
    await once(stdin, "end");
    expect(await input.choose(menu)).toBeUndefined();
    expect(await input.line("Which folder? ")).toBeUndefined();
  });

  it("cancels at Ctrl-D or end of input, and puts the terminal back", async () => {
    const stdin = fakeStdin(["\u001b[B\u0004"]);
    expect(await terminalInput(stdin).choose(menu)).toBeUndefined();
    expect(stdin.raw).toEqual([true, false]);
    expect(await terminalInput(fakeStdin([])).choose(menu)).toBeUndefined();
  });

  it("puts the terminal back before Ctrl-C interrupts set-up", async () => {
    const stdin = fakeStdin(["\u0003"]);
    const interrupt = vi.fn(() => { expect(stdin.raw.at(-1)).toBe(false); });
    expect(await terminalInput(stdin, () => {}, interrupt).choose(menu)).toBeUndefined();
    expect(interrupt).toHaveBeenCalledOnce();
  });
});
