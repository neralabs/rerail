import { EventEmitter } from "node:events";
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

/**
 * A terminal's stdin that delivers `chunks` one at a time, each time set-up reads. `held` is what the terminal holds
 * from before: keys pressed while set-up was busy, delivered first unless something drops them.
 */
function fakeStdin(chunks: readonly string[], { ends = true, held = [] as string[] } = {}): TerminalInputStream & { raw: boolean[]; held: string[] } {
  const left = [...chunks];
  const stream = Object.assign(new EventEmitter(), {
    raw: [] as boolean[],
    held: [...held],
    setRawMode(raw: boolean) { stream.raw.push(raw); },
    setEncoding() {},
    pause() {},
    resume() {
      queueMicrotask(() => {
        const chunk = stream.held.shift() ?? left.shift();
        if (chunk !== undefined) stream.emit("data", chunk);
        else if (ends) stream.emit("end");
      });
    },
  });
  return stream;
}

const menu = { count: 3, frame: (highlight: number) => [0, 1, 2].map(index => `${index === highlight ? "❯" : " "} ${index + 1}`).concat("", "> ").join("\n") };

describe("the terminal's input", () => {
  it("moves the cursor with the arrows, redrawing the menu in place, and takes the answer at Enter", async () => {
    const stdin = fakeStdin(["\u001b[B", "\u001b[B\u001b[A", "\r"]);
    const written: string[] = [];
    const input = new TerminalInput(stdin, text => { written.push(text); });
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
    const input = new TerminalInput(stdin, text => { written.push(text); }, () => {}, () => 20);
    expect(await input.choose(long)).toBe(1);
    // Up four rows from the prompt's: the blank line, the second answer, and the wrapped first answer's two.
    expect(written[1]).toBe(`\r\u001b[4A\u001b[J${long.frame(1)}`);
  });

  it("redraws as one row per line when the terminal's width is unknown", async () => {
    const written: string[] = [];
    const input = new TerminalInput(fakeStdin(["\u001b[B", "\r"]), text => { written.push(text); }, () => {}, () => undefined);
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
    const input = new TerminalInput(fakeStdin(["1\r", "~/proj\r"]), () => {});
    expect(await input.choose(menu)).toBe(0);
    expect(await input.line("Which folder? ")).toBe("~/proj");
    expect(await input.line("Which folder? ")).toBeUndefined();

    const split = new TerminalInput(fakeStdin(["~/a\r", "\n~/b\n", "\r\n"]), () => {});
    expect(await split.line("Which folder? ")).toBe("~/a");
    expect(await split.line("Which folder? ")).toBe("~/b");
    expect(await split.line("Which folder? ")).toBe("");
  });

  it("takes no Enter for a menu from the LF of a CRLF a line ended on", async () => {
    const input = new TerminalInput(fakeStdin(["~/a\r", "\n", "\u001b[B", "\r"]), () => {});
    expect(await input.line("Which folder? ")).toBe("~/a");
    expect(await input.choose(menu)).toBe(1);
  });

  it("takes a digit then Enter, then reads the next prompt's line", async () => {
    const input = new TerminalInput(fakeStdin(["3\r", "my-app\n"]), () => {});
    expect(await input.choose(menu)).toBe(2);
    expect(await input.line("Which folder? ")).toBe("my-app");
  });

  it("answers a question only with what is typed once it is on screen, not with what was left of an earlier read", async () => {
    // A second Enter in the read that answered the first question does not answer the next one.
    const input = new TerminalInput(fakeStdin(["2\r\r", "\u001b[B", "\r", "3\rmy-app\n", "~/code/app\n"]), () => {});
    expect(await input.choose(menu)).toBe(1);
    expect(await input.choose(menu)).toBe(1);
    expect(await input.choose(menu)).toBe(2);
    expect(await input.line("Which folder? ")).toBe("~/code/app");
  });

  it("drops the keys the terminal holds before a menu or prompt is drawn, with the terminal in raw mode", async () => {
    // An Enter pressed while set-up was busy, held by the terminal until set-up reads.
    const stdin = fakeStdin(["\u001b[B", "\r", "~/code/app\n"], { held: ["\r"] });
    const written: string[] = [];
    const drops: { raw: boolean | undefined; drawn: number }[] = [];
    const input = new TerminalInput(stdin, text => { written.push(text); }, () => {}, () => undefined, () => {
      drops.push({ raw: stdin.raw.at(-1), drawn: written.length });
      stdin.held.length = 0;
    });
    expect(await input.choose(menu)).toBe(1);
    // A folder typed while set-up was busy again.
    stdin.held.push("~/typed-early\n");
    expect(await input.line("Which folder? ")).toBe("~/code/app");
    // Each drop came before anything of its question was drawn: the menu's frame, then the prompt after the menu's
    // frame, redraw and closing line break.
    expect(drops).toEqual([{ raw: true, drawn: 0 }, { raw: true, drawn: 3 }]);
    // The prompt is in raw mode only while it drops them.
    expect(stdin.raw).toEqual([true, false, true, false]);
  });

  it("cancels at Ctrl-D or end of input, and puts the terminal back", async () => {
    const stdin = fakeStdin(["\u001b[B\u0004"]);
    expect(await new TerminalInput(stdin, () => {}).choose(menu)).toBeUndefined();
    expect(stdin.raw).toEqual([true, false]);
    expect(await new TerminalInput(fakeStdin([]), () => {}).choose(menu)).toBeUndefined();
  });

  it("puts the terminal back before Ctrl-C interrupts set-up", async () => {
    const stdin = fakeStdin(["\u0003"]);
    const interrupt = vi.fn(() => { expect(stdin.raw.at(-1)).toBe(false); });
    expect(await new TerminalInput(stdin, () => {}, interrupt).choose(menu)).toBeUndefined();
    expect(interrupt).toHaveBeenCalledOnce();
  });
});
