/**
 * How the structure checks read a shell line, which they rely on to tell what CI runs.
 *
 * Four constructs the tokenizer reads differently from a shell have no assertion here, so that this suite does not
 * hold it to the wrong answer: a quoted redirect target that holds a space or a separator, a file-descriptor
 * redirection (`2>&1`), a backslash inside double quotes before a character a shell does not let it escape, and a
 * backslash outside quotes other than a line continuation. https://github.com/neralabs/rerail/issues/475 lists each
 * case with what a shell does.
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
