import { describe, expect, it } from "vitest";
import { MAX_NAMED_FILES, NamedWrites } from "../../../../src/engine/code-rule/attribution/named-writes.js";

describe("NamedWrites", () => {
  it("names the sessions whose finished edit-tool call named a file and ran across a stamp", () => {
    const named = new NamedWrites();
    named.begin("a", "write-a", 10, ["/repo/src/a.py"]);
    named.finish("a", "write-a", 20, false);
    named.begin("b", "write-b", 15, ["/repo/src/a.py", "/repo/src/b.py"]);
    named.finish("b", "write-b", 30, false);
    expect([...named.wroteAt("/repo/src/a.py", 18)].sort()).toEqual(["a", "b"]);
    expect([...named.wroteAt("/repo/src/a.py", 25)]).toEqual(["b"]);
    expect([...named.wroteAt("/repo/src/a.py", 5)]).toEqual([]);
    expect([...named.wroteAt("/repo/src/c.py", 18)]).toEqual([]);
  });

  it("names no write of a call still running, one that failed, or one dropped at its turn's end", () => {
    const named = new NamedWrites();
    named.begin("a", "running", 10, ["/repo/src/a.py"]);
    named.begin("a", "failed", 10, ["/repo/src/b.py"]);
    named.finish("a", "failed", 20, true);
    expect([...named.wroteAt("/repo/src/a.py", 15)]).toEqual([]);
    expect([...named.wroteAt("/repo/src/b.py", 15)]).toEqual([]);
    named.drop("a");
    named.finish("a", "running", 20, false);
    expect([...named.wroteAt("/repo/src/a.py", 15)]).toEqual([]);
  });

  it("keeps each session's last call per file, and what a session finished after it drops its running calls", () => {
    const named = new NamedWrites();
    named.begin("a", "first", 10, ["/repo/src/a.py"]);
    named.finish("a", "first", 20, false);
    named.begin("a", "second", 30, ["/repo/src/a.py"]);
    named.finish("a", "second", 40, false);
    named.drop("a");
    expect([...named.wroteAt("/repo/src/a.py", 15)]).toEqual([]);
    expect([...named.wroteAt("/repo/src/a.py", 35)]).toEqual(["a"]);
  });

  it(`remembers at most ${MAX_NAMED_FILES} files, forgetting the one least recently named`, () => {
    const named = new NamedWrites();
    for (let file = 0; file <= MAX_NAMED_FILES; file += 1) {
      named.begin("a", `write-${file}`, 10, [`/repo/f${file}.py`]);
      named.finish("a", `write-${file}`, 20, false);
    }
    expect([...named.wroteAt("/repo/f0.py", 15)]).toEqual([]);
    expect([...named.wroteAt("/repo/f1.py", 15)]).toEqual(["a"]);
    expect([...named.wroteAt(`/repo/f${MAX_NAMED_FILES}.py`, 15)]).toEqual(["a"]);
  });
});
