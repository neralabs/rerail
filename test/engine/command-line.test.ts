import { describe, expect, it } from "vitest";
import { POWERSHELL_QUOTES, powershellLiteral, powershellQuotesTwice } from "../../src/engine/command-line.js";

const QUOTES = ["'", "\u2018", "\u2019", "\u201A", "\u201B"];

describe("PowerShell literals", () => {
  it("names the five characters PowerShell reads as a single quote", () => {
    expect([...POWERSHELL_QUOTES]).toEqual(QUOTES);
  });

  // PowerShell ends a single-quoted string at any of the five, and reads one written twice as itself.
  it.each(QUOTES)("writes %s twice, so it stays in the string", quote => {
    expect(powershellLiteral(`O${quote}Neil ${quote}x${quote}`)).toBe(`'O${quote}${quote}Neil ${quote}${quote}x${quote}${quote}'`);
  });

  it("expands nothing: `$`, backticks, `%`, double quotes and spaces stay as they are", () => {
    expect(powershellLiteral(String.raw`C:\Users\a $b ` + "`c %PATH% \"d\"")).toBe(String.raw`'C:\Users\a $b ` + "`c %PATH% \"d\"'");
  });

  it("tells an agent to write each of the five twice", () => {
    expect(powershellQuotesTwice("`<file>`")).toBe("each ' \u2018 \u2019 \u201A \u201B in `<file>` written twice");
  });
});
