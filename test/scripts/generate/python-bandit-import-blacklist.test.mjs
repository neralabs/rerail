// scripts/generate/packs/python-bandit-import-blacklist.mjs reads no tools, so this checks what CI can: each committed
// Bandit import-blacklist port is exactly what the generator writes, in the folder its candidate flag names.

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { BLACKLIST, PACK_DIRECTORY, content, documentPath, render, ruleId } from "../../../scripts/generate/packs/python-bandit-import-blacklist.mjs";

const committed = entry => readFileSync(join(PACK_DIRECTORY, documentPath(entry)), "utf8");

describe("the Bandit import-blacklist generator", () => {
  it.each(BLACKLIST.map(entry => [ruleId(entry), entry]))("writes the committed %s", (_id, entry) => {
    const text = committed(entry);
    expect(render(entry, JSON.parse(text))).toBe(text);
    expect(existsSync(join(PACK_DIRECTORY, documentPath({ ...entry, candidate: !entry.candidate })))).toBe(false);
  });

  it("gives a changed document the next revision and a new one revision 1", () => {
    const [entry] = BLACKLIST;
    const document = JSON.parse(committed(entry));
    expect(JSON.parse(render({ ...entry, message: "Changed." }, document)).revision).toBe(document.revision + 1);
    expect(JSON.parse(render(entry, undefined)).revision).toBe(1);
  });

  it("matches a from-import of each qualname's parent module, one alternative per parent", () => {
    const httpoxy = BLACKLIST.find(entry => entry.code === "B412");
    const parents = content(httpoxy).clauses[0].forbid.ast.rule.any.slice(2).map(arm => arm.all[0].has.regex);
    expect(parents).toEqual(["^[.\\s]*wsgiref\\s*\\.\\s*handlers$", "^[.\\s]*twisted\\s*\\.\\s*web\\s*\\.\\s*twcgi$"]);
  });
});
