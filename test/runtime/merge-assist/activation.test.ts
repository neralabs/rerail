import { mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  libraryRules,
  lookupCheckout,
  parseCandidateText,
  recordCheckout,
  ruleAttachments,
  ruleSettings,
  setUpRepository,
} from "../../../src/engine/index.js";
import { setRepositoryStarted } from "../../../src/runtime/index.js";
import { activationFailures, ownRuleId, peerRuleId, reconcileCheckout } from "../../../src/runtime/merge-assist/activation.js";
import { candidate, candidateFiles, commit, forbidRule, git, mergeAssistWorkspaces, writeFiles } from "./support/merge-assist-workspace.js";

const workspace = mergeAssistWorkspaces();

const SLEEP_ID = "candidate:no-sleep-in-api-0a0b0c";
const SLEEP = candidate({ id: SLEEP_ID, author: "tbelanger", decision: "Never sleep in API handlers.", rule: forbidRule(SLEEP_ID, String.raw`\bsleep\(`) });

// Each test builds Git repositories and compiles rules in the Engine; under a full run that outlasts the default 5 s.
describe("Merge Assist activation", { timeout: 30_000 }, () => {
  it("switches a candidate's rule on for each checkout that has it, and off the repository everywhere else", async () => {
    const ws = await workspace();
    const other = ws.worktree("other", { record: true });
    const plain = ws.worktree("plain", { record: true });
    writeFiles(ws.repo, candidateFiles(SLEEP));
    writeFiles(other, candidateFiles(SLEEP));
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    // A session starting there applies that checkout's candidates.
    await service.openSession(other, "codex", "session-1");

    const mine = ownRuleId(ws.repo, SLEEP_ID);
    const theirs = ownRuleId(other, SLEEP_ID);
    expect(mine).toMatch(/^candidate:no-sleep-in-api-0a0b0c~[0-9a-f]{8}$/u);
    expect(theirs).not.toBe(mine);
    expect(ws.rulesOn(ws.repo)).toEqual([mine]);
    expect(ws.rulesOn(other)).toEqual([theirs]);
    expect(ws.rulesOn(plain)).toEqual([]);
    expect(ws.database.select().from(ruleSettings).all().filter(row => row.ownerKind === "repository").map(row => [row.ownerId, row.targetId, row.on]).sort())
      .toEqual([[ws.repositoryKey, mine, false], [ws.repositoryKey, theirs, false]].sort());

    // Not a rule, in one checkout: the file keeps the state, and the other checkout still has it.
    const rejected = await service.setCandidateState(ws.repo, SLEEP_ID, "rejected");
    expect(rejected.own).toMatchObject([{ id: SLEEP_ID, state: "rejected" }]);
    const file = parseCandidateText(readFileSync(join(ws.repo, ".rerail/candidates/no-sleep-in-api-0a0b0c.json"), "utf8"));
    expect(file).toMatchObject({ ok: true, candidate: { state: "rejected" } });
    expect(ws.rulesOn(ws.repo)).toEqual([]);
    expect(ws.rulesOn(other)).toEqual([theirs]);

    await service.setCandidateState(other, SLEEP_ID, "rejected");
    expect(ws.database.select().from(libraryRules).all()).toEqual([]);
    expect(ws.database.select().from(ruleAttachments).all()).toEqual([]);
    expect(ws.database.select().from(ruleSettings).all()).toEqual([]);

    await service.setCandidateState(ws.repo, SLEEP_ID, "active");
    expect(ws.rulesOn(ws.repo)).toEqual([mine]);
    await expect(service.setCandidateState(ws.repo, "candidate:not-here-000000", "rejected")).rejects.toMatchObject({ code: "not-found" });
  });

  it("never lets another checkout or repository with the same candidate id replace this checkout's rule", async () => {
    const ws = await workspace();
    const intruder = ws.worktree("intruder", { record: true });
    // A second repository in the same database: the rule library is shared by all of them.
    const second = join(ws.folder, "second");
    mkdirSync(second);
    git(second, "init", "-q", "-b", "main");
    commit(second, { "src/api/client.ts": "export const client = {};\n" }, "start");
    const { repositoryKey: secondKey } = recordCheckout(ws.database, second);
    setUpRepository(ws.database, secondKey, { packs: [] });
    setRepositoryStarted(ws.database, secondKey, true);
    writeFiles(ws.repo, candidateFiles(SLEEP));
    // The same id, each with a rule that would flag something else.
    writeFiles(intruder, candidateFiles({ ...SLEEP, rule: forbidRule(SLEEP_ID, String.raw`\bfetch\(`) }));
    writeFiles(second, candidateFiles({ ...SLEEP, rule: forbidRule(SLEEP_ID, String.raw`\bclient\b`) }));
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    await service.setEnabled(second, true);
    await service.openSession(intruder, "codex", "session-1");
    // This checkout reconciles again, after the others wrote theirs.
    await service.openSession(ws.repo, "codex", "session-1");

    const stored = Object.fromEntries(ws.database.select().from(libraryRules).all().map(row => [row.ruleId, JSON.stringify(row.document)]));
    expect(Object.keys(stored).sort()).toEqual([ownRuleId(ws.repo, SLEEP_ID), ownRuleId(intruder, SLEEP_ID), ownRuleId(second, SLEEP_ID)].sort());
    expect(stored[ownRuleId(ws.repo, SLEEP_ID)]).toContain(JSON.stringify(String.raw`\bsleep\(`));
    expect(stored[ownRuleId(intruder, SLEEP_ID)]).toContain(JSON.stringify(String.raw`\bfetch\(`));
    expect(stored[ownRuleId(second, SLEEP_ID)]).toContain(JSON.stringify(String.raw`\bclient\b`));
    expect(ws.rulesOn(ws.repo)).toEqual([ownRuleId(ws.repo, SLEEP_ID)]);
    expect(ws.rulesOn(intruder)).toEqual([ownRuleId(intruder, SLEEP_ID)]);
    expect(ws.rulesOn(second)).toEqual([ownRuleId(second, SLEEP_ID)]);
  });

  it("keeps a checkout's candidate rules out of the checkouts nested in it, and lets a nested one keep its own", async () => {
    const ws = await workspace();
    const nested = join(ws.repo, ".worktrees", "child");
    git(ws.repo, "worktree", "add", "-q", "-b", "child", nested, "main");
    recordCheckout(ws.database, nested);
    writeFiles(ws.repo, candidateFiles(SLEEP));
    const service = ws.service();
    await service.setEnabled(ws.repo, true);

    expect(ws.rulesOn(ws.repo)).toEqual([ownRuleId(ws.repo, SLEEP_ID)]);
    expect(ws.rulesOn(nested)).toEqual([]);

    // The nested checkout's own decision applies there and only there; its reconciling keeps the parent's rule off.
    const own = candidate({ id: "candidate:name-files-by-resource-2c3d4e", decision: "Name API files after their resource.", rule: forbidRule("candidate:name-files-by-resource-2c3d4e", String.raw`\bapi_\w+`) });
    writeFiles(nested, candidateFiles(own));
    await service.openSession(nested, "codex", "session-1");
    expect(ws.rulesOn(nested)).toEqual([ownRuleId(nested, own.id)]);
    expect(ws.rulesOn(ws.repo)).toEqual([ownRuleId(ws.repo, SLEEP_ID)]);
    await service.openSession(ws.repo, "codex", "session-1");
    expect(ws.rulesOn(nested)).toEqual([ownRuleId(nested, own.id)]);
  });

  it("keeps a candidate whose rule does not compile here as a note, and records why", async () => {
    const ws = await workspace();
    const lookahead = candidate({ id: "candidate:look-ahead-1a1a1a", rule: forbidRule("candidate:look-ahead-1a1a1a", "retry(?=\\()") });
    writeFiles(ws.repo, candidateFiles(lookahead, SLEEP));
    const service = ws.service();
    const status = await service.setEnabled(ws.repo, true);

    expect(ws.rulesOn(ws.repo)).toEqual([ownRuleId(ws.repo, SLEEP_ID)]);
    expect(status.own.map(own => [own.id, own.level])).toEqual([[lookahead.id, "note"], [SLEEP_ID, "code"]]);
    expect(Object.keys(activationFailures(ws.database, ws.repo))).toEqual([ownRuleId(ws.repo, lookahead.id)]);
  });

  it("leaves rules it did not write alone", async () => {
    const ws = await workspace();
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    // A person's own settings on rules whose ids only look like a candidate's: another case, and no tag.
    for (const targetId of ["Candidate:mine~0123abcd", "candidate:mine"]) {
      ws.database.insert(ruleSettings).values({
        ownerKind: "checkout", ownerId: ws.repo, folder: "", targetKind: "rule", targetId, on: true, mode: null, updatedAt: 1,
      }).run();
    }
    await reconcileCheckout(ws.database, ws.ruleCatalog, lookupCheckout(ws.database, ws.repo)!, 2);
    expect(ws.database.select().from(ruleSettings).all().map(row => row.targetId).sort()).toEqual(["Candidate:mine~0123abcd", "candidate:mine"]);
  });

  it("tags a peer's rule with the checkout, where it came from and its version", () => {
    const item = { candidateId: "candidate:retries-in-transport-3f9a1c", peerKind: "branch" as const, peerRef: "jbarnes-retry", candidateHash: "a".repeat(64) };
    const fromBranch = peerRuleId("/work/repo", item);
    expect(fromBranch).toMatch(/^candidate:retries-in-transport-3f9a1c~[0-9a-f]{8}$/u);
    expect(peerRuleId("/work/sibling", item)).not.toBe(fromBranch);
    expect(peerRuleId("/work/repo", { ...item, peerKind: "pull-request" })).not.toBe(fromBranch);
    expect(peerRuleId("/work/repo", { ...item, candidateHash: "b".repeat(64) })).not.toBe(fromBranch);
    expect(ownRuleId("/work/repo", item.candidateId)).not.toBe(fromBranch);
  });
});
