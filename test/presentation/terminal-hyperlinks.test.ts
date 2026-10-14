import { describe, expect, it } from "vitest";
import { terminalDrawsHyperlinks } from "../../src/presentation/terminal-hyperlinks.js";

describe("terminalDrawsHyperlinks", () => {
  it("draws links only in a terminal known to draw them, so an unknown one gets the URL printed", () => {
    expect(terminalDrawsHyperlinks({ TERM_PROGRAM: "iTerm.app" })).toBe(true);
    expect(terminalDrawsHyperlinks({ TERM_PROGRAM: "Orca" })).toBe(true);
    expect(terminalDrawsHyperlinks({ TERM: "xterm-ghostty" })).toBe(true);
    expect(terminalDrawsHyperlinks({ TERM_PROGRAM: "Apple_Terminal" })).toBe(false);
    expect(terminalDrawsHyperlinks({ TERM_PROGRAM: "WarpTerminal" })).toBe(false);
    expect(terminalDrawsHyperlinks({})).toBe(false);
  });

  it("forwards links through tmux from 3.4 only, and never through GNU screen", () => {
    const iterm = { ITERM_SESSION_ID: "w0t0p0", TMUX: "/tmp/tmux-501/default,1,0", TERM_PROGRAM: "tmux" };
    expect(terminalDrawsHyperlinks({ ...iterm, TERM_PROGRAM_VERSION: "3.4" })).toBe(true);
    expect(terminalDrawsHyperlinks({ ...iterm, TERM_PROGRAM_VERSION: "3.3a" })).toBe(false);
    expect(terminalDrawsHyperlinks({ ORCA_TERMINAL_HANDLE: "term_1", TMUX: "/tmp/tmux-501/default,1,0", TERM_PROGRAM: "tmux", TERM_PROGRAM_VERSION: "3.5a" })).toBe(true);
    expect(terminalDrawsHyperlinks({ TERM_PROGRAM: "iTerm.app", STY: "1234.pts-0" })).toBe(false);
  });

  it("lets the person decide, with turning links off winning", () => {
    expect(terminalDrawsHyperlinks({ RERAIL_FORCE_HYPERLINKS: "1" })).toBe(true);
    expect(terminalDrawsHyperlinks({ PI_FORCE_HYPERLINKS: "1" })).toBe(true);
    expect(terminalDrawsHyperlinks({ TERM_PROGRAM: "iTerm.app", PI_NO_HYPERLINKS: "1" })).toBe(false);
    expect(terminalDrawsHyperlinks({ RERAIL_FORCE_HYPERLINKS: "1", RERAIL_NO_HYPERLINKS: "1" })).toBe(false);
  });
});
