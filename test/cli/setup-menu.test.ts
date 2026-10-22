import { EventEmitter } from "node:events";
import { describe, expect, it, vi } from "vitest";
import { menuAction, readKey, TerminalInput, type TerminalInputStream } from "../../src/cli/setup-menu.js";

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

/** A terminal's stdin that delivers `chunks` one at a time, each time set-up reads. */
function fakeStdin(chunks: readonly string[], { ends = true } = {}): TerminalInputStream & { raw: boolean[] } {
  const left = [...chunks];
  const stream = Object.assign(new EventEmitter(), {
    raw: [] as boolean[],
    setRawMode(raw: boolean) { stream.raw.push(raw); },
    setEncoding() {},
    pause() {},
    resume() {
      queueMicrotask(() => {
        const chunk = left.shift();
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

  it("takes a digit then Enter, and keeps what follows for the next prompt", async () => {
    const input = new TerminalInput(fakeStdin(["3\r", "my-app\n"]), () => {});
    expect(await input.choose(menu)).toBe(2);
    expect(await input.line("Which folder? ")).toBe("my-app");
  });

  it("reads lines typed ahead of a menu as keys, and the lines after it as lines", async () => {
    const input = new TerminalInput(fakeStdin(["2\n~/code/app\n1\n"]), () => {});
    expect(await input.choose(menu)).toBe(1);
    expect(await input.line("Which folder? ")).toBe("~/code/app");
    expect(await input.choose(menu)).toBe(0);
    expect(await input.line("Which folder? ")).toBeUndefined();
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
