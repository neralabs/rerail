/**
 * How the structure checks read a shell line, which they rely on to tell what CI runs. Where a line is valid shell,
 * the expected value is the commands and arguments `sh` runs for it.
 */
import { describe, expect, it } from "vitest";
import { shellCommands } from "../../scripts/check/check-structure/lib.mjs";

describe("shellCommands", () => {
  it("splits on every separator, a doubled one once", () => {
    expect(shellCommands("a b && c || d; e | f & g\nh")).toEqual([["a", "b"], ["c"], ["d"], ["e"], ["f"], ["g"], ["h"]]);
    expect(shellCommands("")).toEqual([]);
    expect(shellCommands(" ;; \n ")).toEqual([]);
  });

  it("keeps a quoted word whole: escapes inside double quotes, none inside single quotes", () => {
    expect(shellCommands(`echo "a b" 'c d' e"f"g ""`)).toEqual([["echo", "a b", "c d", "efg", ""]]);
    expect(shellCommands(String.raw`echo "a\"b\\c" 'a\nb'`)).toEqual([["echo", 'a"b\\c', String.raw`a\nb`]]);
    expect(shellCommands(`echo "a;b" 'c|d'`)).toEqual([["echo", "a;b", "c|d"]]);
    expect(shellCommands(`echo "open\\`)).toEqual([["echo", "open\\"]]);
    expect(shellCommands(`echo "open`)).toEqual([["echo", "open"]]);
  });

  it("drops a backslash inside double quotes only before a character it escapes", () => {
    expect(shellCommands(String.raw`echo "a\nb" "e\.f"`)).toEqual([["echo", String.raw`a\nb`, String.raw`e\.f`]]);
    expect(shellCommands(String.raw`echo "x\$y\`z"`)).toEqual([["echo", "x$y`z"]]);
    expect(shellCommands('echo "a\\\nb" c')).toEqual([["echo", "ab", "c"]]);
  });

  it("takes the character after a backslash outside quotes literally", () => {
    expect(shellCommands(String.raw`a\b`)).toEqual([["ab"]]);
    expect(shellCommands(String.raw`echo a\ b c\;d`)).toEqual([["echo", "a b", "c;d"]]);
    expect(shellCommands(String.raw`echo x \# y`)).toEqual([["echo", "x", "#", "y"]]);
    expect(shellCommands("echo end\\")).toEqual([["echo", "end"]]);
  });

  it("joins a line continuation", () => {
    expect(shellCommands("a \\\n b")).toEqual([["a", "b"]]);
    expect(shellCommands("a\\\nb")).toEqual([["ab"]]);
  });

  it("skips a comment only where a word could start", () => {
    expect(shellCommands("a # b c\nd")).toEqual([["a"], ["d"]]);
    expect(shellCommands("a#b c")).toEqual([["a#b", "c"]]);
    expect(shellCommands("# only")).toEqual([]);
  });

  it("skips a redirect's target, quoted or not", () => {
    expect(shellCommands("a > out b")).toEqual([["a", "b"]]);
    expect(shellCommands(`a >"out" b`)).toEqual([["a", "b"]]);
    expect(shellCommands("a < in; b")).toEqual([["a"], ["b"]]);
    expect(shellCommands("a >> out")).toEqual([["a"]]);
    expect(shellCommands("a >")).toEqual([["a"]]);
    expect(shellCommands(`a >"out file" b`)).toEqual([["a", "b"]]);
    expect(shellCommands(`a >"o;p" b`)).toEqual([["a", "b"]]);
    expect(shellCommands("a >x>y b")).toEqual([["a", "b"]]);
    expect(shellCommands("a >|clob b")).toEqual([["a", "b"]]);
    expect(shellCommands(`a <<<"$x" b`)).toEqual([["a", "b"]]);
  });

  it("takes an unquoted number before a redirect as its file descriptor", () => {
    expect(shellCommands("a 2>&1 b")).toEqual([["a", "b"]]);
    expect(shellCommands("a 2>'err' b")).toEqual([["a", "b"]]);
    expect(shellCommands("a 12>w b")).toEqual([["a", "b"]]);
    expect(shellCommands("a 2>&1|b")).toEqual([["a"], ["b"]]);
    expect(shellCommands("a b>z c")).toEqual([["a", "b", "c"]]);
    expect(shellCommands(`a "2">x b`)).toEqual([["a", "2", "b"]]);
    expect(shellCommands(String.raw`a \2>y b`)).toEqual([["a", "2", "b"]]);
  });

  it("keeps a backslash-newline as it is inside single quotes", () => {
    expect(shellCommands("echo 'a\\\nb' c")).toEqual([["echo", "a\\\nb", "c"]]);
  });

  it("leaves leading assignments out of what runs", () => {
    expect(shellCommands("A=1 B_2=x c d")).toEqual([["c", "d"]]);
    expect(shellCommands("c A=1")).toEqual([["c", "A=1"]]);
    expect(shellCommands("A=1")).toEqual([[]]);
  });
});
