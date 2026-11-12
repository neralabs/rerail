import { describe, expect, it } from "vitest";
import {
  commandLine,
  placeholderWord,
  POSIX_QUOTED_BODY,
  posixQuotedValue,
  POWERSHELL_LITERAL_BODY,
  POWERSHELL_QUOTES,
  powershellLiteral,
  powershellLiteralValue,
  powershellQuotesTwice,
  shellWord,
  textPlaceholder,
} from "../../src/engine/command-line.js";

const QUOTES = ["'", "\u2018", "\u2019", "\u201A", "\u201B"];

describe("one word in each shell", () => {
  // A value, then the one word a POSIX shell (sh, bash, Git Bash) and PowerShell each read it from.
  const WORDS: ReadonlyArray<readonly [string, string, string]> = [
    ["--answer", "--answer", "--answer"],
    ["/Users/me/code/acme-web", "/Users/me/code/acme-web", "/Users/me/code/acme-web"],
    [String.raw`C:\Users\me\.rerail\bin\rerail.cmd`, String.raw`'C:\Users\me\.rerail\bin\rerail.cmd'`, String.raw`C:\Users\me\.rerail\bin\rerail.cmd`],
    // PowerShell reads a bare comma as an array, and hands a program each element as an argument of its own.
    ["typescript@0.1.1,complexity@0.1.1", "typescript@0.1.1,complexity@0.1.1", "'typescript@0.1.1,complexity@0.1.1'"],
    ["@acme/rerail-pack-sql@1.0.0", "@acme/rerail-pack-sql@1.0.0", "'@acme/rerail-pack-sql@1.0.0'"],
    ["/code/acme web", "'/code/acme web'", "'/code/acme web'"],
    ["/home/a$b/`c`", "'/home/a$b/`c`'", "'/home/a$b/`c`'"],
    ["/tmp/it's mine", String.raw`'/tmp/it'\''s mine'`, "'/tmp/it''s mine'"],
    ["/tmp/it\u2019s", "'/tmp/it\u2019s'", "'/tmp/it\u2019\u2019s'"],
    ["say \"hi\"", "'say \"hi\"'", "'say \"hi\"'"],
    ["~/code", "'~/code'", "'~/code'"],
    ["", "''", "''"],
  ];

  it.each(WORDS)("writes %j for a POSIX shell and for PowerShell", (value, posix, powershell) => {
    expect(shellWord("posix", value)).toBe(posix);
    expect(shellWord("powershell", value)).toBe(powershell);
  });

  it("starts a PowerShell line with the call operator, which runs a quoted program path; a POSIX line needs none", () => {
    const words = [String.raw`C:\Program Files\nodejs\node.exe`, String.raw`C:\rerail\cli.js`, "setup", "--packs", "a@1,b@2"];
    expect(commandLine("powershell", words)).toBe(String.raw`& 'C:\Program Files\nodejs\node.exe' C:\rerail\cli.js setup --packs 'a@1,b@2'`);
    expect(commandLine("posix", words)).toBe(String.raw`'C:\Program Files\nodejs\node.exe' 'C:\rerail\cli.js' setup --packs a@1,b@2`);
  });

  it("leaves a placeholder bare for a POSIX shell and puts it in single quotes for PowerShell, where the agent's value goes", () => {
    expect(placeholderWord("posix", "<packs>")).toBe("<packs>");
    expect(placeholderWord("powershell", "<packs>")).toBe("'<packs>'");
  });

  it("has the agent write each quote twice in text the person types, where PowerShell's single quotes would end at one", () => {
    expect(textPlaceholder("posix", "<the folder>")).toBe("<the folder>");
    expect(textPlaceholder("powershell", "<the folder>")).toBe("'<the folder>', with each ' ‘ ’ ‚ ‛ in <the folder> written twice");
  });
});

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

describe("reading a quoted word back", () => {
  const literal = new RegExp(`^'(${POWERSHELL_LITERAL_BODY})'$`, "u");
  const posixWord = new RegExp(`^'(${POSIX_QUOTED_BODY})'$`, "u");

  it.each(QUOTES)("reads back a PowerShell literal holding %s, each one written twice", quote => {
    const value = String.raw`C:\Users\O${quote}Neil ${quote}x${quote}\node.exe`;
    const body = literal.exec(powershellLiteral(value))?.[1];
    expect(body === undefined ? undefined : powershellLiteralValue(body)).toBe(value);
  });

  it("reads only what powershellLiteral writes: a quote alone, or two different quotes, ends the literal", () => {
    expect(literal.test("'O'Neil'")).toBe(false);
    expect(literal.test("'O'\u2019Neil'")).toBe(false);
    // The body pattern holds no capture group, so a reader's own groups keep their numbers.
    expect(new RegExp(`(${POWERSHELL_LITERAL_BODY})`, "u").exec("")?.length).toBe(2);
  });

  it.each(["/tmp/it's mine", "/home/a$b/`c` d", "''", "\u2019"])("reads back a POSIX word that holds %j", value => {
    const body = posixWord.exec(shellWord("posix", value))?.[1];
    expect(body === undefined ? undefined : posixQuotedValue(body)).toBe(value);
  });
});
