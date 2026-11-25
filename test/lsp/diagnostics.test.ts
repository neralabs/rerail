import { TextDocument } from "vscode-languageserver-textdocument";
import { describe, expect, it } from "vitest";
import { JudgedText } from "../../src/lsp/diagnostics.js";

describe("JudgedText", () => {
  it("maps the engine's UTF-8 byte offsets to the editor's UTF-16 positions", () => {
    // "é" is 2 bytes and 1 unit, "😀" 4 bytes and 2 units, "→" 3 bytes and 1 unit.
    const source = 'a é "😀"\n→ b\n';
    const text = new JudgedText(TextDocument.create("file:///x.rerail", "rerail", 1, source));
    const at = (fragment: string) => text.position(Buffer.byteLength(source.slice(0, source.indexOf(fragment)), "utf8"));
    expect(at('"😀"')).toEqual({ line: 0, character: 4 });
    expect(at('"\n')).toEqual({ line: 0, character: 7 });
    expect(at("b")).toEqual({ line: 1, character: 2 });
    expect(text.position(Buffer.byteLength(source, "utf8"))).toEqual({ line: 2, character: 0 });
    // A byte inside a character is that character's start: the third byte of "😀", at unit 5.
    expect(text.position(Buffer.byteLength('a é "', "utf8") + 2)).toEqual({ line: 0, character: 5 });
  });
});
