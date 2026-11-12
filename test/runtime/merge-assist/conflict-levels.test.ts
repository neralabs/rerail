import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import type { AdapterDelivery, Conflict } from "../../../src/engine/index.js";
import { claudeHookOutput } from "../../../src/hosts/claude/index.js";
import { codexHookEvents, codexHookOutput } from "../../../src/hosts/codex/index.js";
import { SqliteSessionReader, SqliteSessionStore } from "../../../src/runtime/index.js";
import { commit, writeFiles } from "../../support/git.js";
import { mergeAssistWorkspaces, takeNotes, type Workspace } from "../../support/merge-assist/workspace.js";
import { body, grown, open, sessionRuntime } from "./support/conflict-sessions.js";

const workspace = mergeAssistWorkspaces();

const HOUR = 3_600_000;

/**
 * The base holds `files` (each a `body`), and one branch per file, `jbarnes-<n>`, changes its line 20 and builds `extra`
 * lines on it; this checkout edits line 22 of each, uncommitted: one overlap per file.
 */
async function overlaps(files: readonly string[], extra: number, base: Record<string, string> = {}): Promise<Workspace> {
  const ws = await workspace({ files: { ...Object.fromEntries(files.map(file => [file, body(nameOf(file))])), ...base } });
  files.forEach((file, index) => commit(ws.worktree(`jbarnes-${index + 1}`), { [file]: grown(nameOf(file), extra) }, `Grow ${file}`));
  writeFiles(ws.repo, Object.fromEntries(files.map(file => [file, body(nameOf(file), { 22: `  const ${nameOf(file)}22 = "mine";` })])));
  return ws;
}

const nameOf = (file: string): string => file.replace(/^.*\//u, "").replace(/\W.*$/u, "");
const byFile = (conflicts: readonly Conflict[], file: string): Conflict => {
  const found = conflicts.find(conflict => conflict.mine.location?.startsWith(`${file}:`) === true);
  if (found === undefined) throw new Error(`no conflict in ${file}`);
  return found;
};

describe("conflict levels", { timeout: 60_000 }, () => {
  it("raises a growing overlap to a checkpoint: a card for the agent to show, and the person's line", async () => {
    const ws = await overlaps(["src/retry.ts"], 30);
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const [conflict] = open((await service.scan(ws.repo)).conflicts);
    expect(conflict).toMatchObject({ level: "checkpoint", queued: false, reasons: expect.arrayContaining(["+31 lines on theirs and +1 on this branch in 1 h"]) });

    const [note, ...more] = service.pendingNotes("session-1", ws.repo);
    expect(more).toEqual([]);
    expect(note!.conflict).toEqual({ id: conflict!.id, text: "decide: retry.ts · function retry (jbarnes-1)" });
    expect(note!.content).toContain("In your next message to the user, show them this card exactly as written:");
    expect(note!.content).toContain(`◆ RERAIL · conflict #${conflict!.shortId}: retry.ts · function retry`);
    expect(note!.content).toContain("  Theirs   \"Grow src/retry.ts\"  (written by Tim Belanger · jbarnes-1)");
    expect(note!.content).toContain(`  Size     ${conflict!.reasons.join(" · ")}`);
    expect(note!.content).toContain("  → Talk to Tim Belanger about it (jbarnes-1).\n    Not a conflict? Dismiss it on the Merge Assist page of the dashboard.");
    expect(note!.content).toContain("Until they decide, stop deepening these files: `src/retry.ts`. Continue with the rest of your task.");
    expect(takeNotes(service, "session-1", ws.repo)).toHaveLength(1);
    expect(takeNotes(service, "session-1", ws.repo)).toEqual([]);
  });

  it("never drops an open conflict's level when its growth stops", async () => {
    const ws = await overlaps(["src/retry.ts"], 30);
    let now = Date.now();
    const service = ws.service({ now: () => now });
    await service.setEnabled(ws.repo, true);
    expect(open((await service.scan(ws.repo)).conflicts)[0]).toMatchObject({ level: "checkpoint" });
    now += 3 * HOUR;
    const [later] = open((await service.scan(ws.repo)).conflicts);
    // No one built on it since: the rate falls with the hours since the fork, the level holds.
    expect(later).toMatchObject({ level: "checkpoint", signals: { linesTheirs: 31, linesMine: 1, windowHours: expect.closeTo(3, 2) } });
    expect(later!.score).toBeLessThan(300);
  });

  it("keeps to the person's three checkpoints a day, queueing the rest until the next day, when it rises however quiet it got", async () => {
    const ws = await overlaps(["src/retry.ts", "src/cache.ts", "src/queue.ts", "src/store.ts"], 30);
    const start = Date.now();
    let now = start;
    const service = ws.service({ now: () => now });
    await service.setEnabled(ws.repo, true);
    const first = open((await service.scan(ws.repo)).conflicts);
    expect(first.map(conflict => [conflict.level, conflict.queued, conflict.queuedFor]).sort()).toEqual([
      ["ambient", true, "checkpoint"], ["checkpoint", false, null], ["checkpoint", false, null], ["checkpoint", false, null],
    ]);
    const queued = first.find(conflict => conflict.queued)!;
    // The queued one is an ambient overlap meanwhile: a note and a line, no card.
    const notes = service.pendingNotes("session-1", ws.repo);
    expect(notes.map(note => note.conflict?.text.startsWith("decide:"))).toEqual([false, true, true, true]);
    expect(notes[0]!.conflict!.id).toBe(queued.id);
    // A minute later, the same budget day, it still waits.
    now = start + 60_000;
    expect(open((await service.scan(ws.repo)).conflicts).find(conflict => conflict.id === queued.id)).toMatchObject({ level: "ambient", queued: true, queuedFor: "checkpoint" });

    // The next day's budget lets it rise.
    now = start + 25 * HOUR;
    const next = open((await service.scan(ws.repo)).conflicts);
    expect(next.find(conflict => conflict.id === queued.id)).toMatchObject({ level: "checkpoint", queued: false, queuedFor: null });
  });

  it("stops the line before an edit to a contested migration: Claude Code asks the person, other hosts tell the agent to", async () => {
    const migration = "db/migrations/users.sql";
    const ws = await overlaps([migration], 60);
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    expect(await service.detectAfterEdit(ws.repo, "session-claude")).toBe(true);
    const [conflict] = open((await service.status(ws.repo)).conflicts);
    expect(conflict).toMatchObject({ level: "stop", queued: false, reasons: expect.arrayContaining(["persisted schema or file format: ×4"]) });

    const edit = { toolName: "Edit", input: { file_path: join(ws.repo, migration), old_string: "a", new_string: "b" } };
    expect(service.stopNotes("session-claude", ws.repo, { toolName: "Read", input: { file_path: join(ws.repo, migration) } }, true)).toEqual([]);
    expect(service.stopNotes("session-claude", ws.repo, { toolName: "Edit", input: { file_path: join(ws.repo, "src/api/client.ts") } }, true)).toEqual([]);
    const [stop] = service.stopNotes("session-claude", ws.repo, edit, true);
    expect(stop!.content).toContain(`Rerail asked the user before this edit: \`${migration}\` is contested with other in-flight work.`);
    expect(stop!.conflict).toMatchObject({ id: conflict!.id, text: "decide: users.sql · function users (jbarnes-1)", ask: expect.stringContaining(`Rerail: ${migration} is contested with jbarnes-1 (Tim Belanger)`) });

    const delivery: AdapterDelivery = {
      sessionId: "session-claude", requested: "steer", mechanism: "additional-context", content: stop!.content, ruleIds: [], rules: [], strategy: "message", conflict: stop!.conflict!,
    };
    const output = claudeHookOutput("PreToolUse", [delivery], { links: "plain" }) as { systemMessage: string; hookSpecificOutput: Record<string, unknown> };
    expect(output.hookSpecificOutput).toMatchObject({ hookEventName: "PreToolUse", permissionDecision: "ask", permissionDecisionReason: stop!.conflict!.ask, additionalContext: stop!.content });
    expect(output.systemMessage).toBe("◆ Rerail · decide: users.sql · function users (jbarnes-1)");
    // At PostToolUse the same delivery asks nothing.
    expect(claudeHookOutput("PostToolUse", [delivery]).hookSpecificOutput).not.toHaveProperty("permissionDecision");

    service.markNotesDelivered("session-claude", ws.repo, stop!.marks);
    expect(service.stopNotes("session-claude", ws.repo, edit, true)).toEqual([]);
    // Its card went out with the stop: no checkpoint card follows in that session.
    expect(service.pendingNotes("session-claude", ws.repo)).toEqual([]);
    const [told] = service.stopNotes("session-codex", ws.repo, edit, false);
    expect(told!.content).toContain(`Don't edit \`${migration}\` now: it is contested with other in-flight work, and the user has to decide first. Ask the user now`);
    expect(told!.conflict).not.toHaveProperty("ask");
  });

  it("gives a session one stop: a second conflict waits for one, queued, and gets it at the next session's first detection", async () => {
    const files = ["db/migrations/users.sql", "db/migrations/orders.sql"];
    const outside = await overlaps(files, 60);
    const scanned = outside.service();
    await scanned.setEnabled(outside.repo, true);
    // Outside a session there is no stop to spend: both wait for one, shown as checkpoints meanwhile.
    expect(open((await scanned.scan(outside.repo)).conflicts).map(conflict => [conflict.level, conflict.queuedFor])).toEqual([["checkpoint", "stop"], ["checkpoint", "stop"]]);

    const ws = await overlaps(files, 60);
    let now = Date.now();
    const service = ws.service({ now: () => now });
    await service.setEnabled(ws.repo, true);
    expect(await service.detectAfterEdit(ws.repo, "session-1")).toBe(true);
    // The other shows as a checkpoint meanwhile: today still has one to spare.
    const first = open((await service.status(ws.repo)).conflicts);
    expect(first.map(conflict => [conflict.level, conflict.queued, conflict.queuedFor])).toEqual([["stop", false, null], ["checkpoint", true, "stop"]]);
    const waiting = first[1]!.id;
    // Another pass in the same session keeps it waiting.
    now += 31_000;
    expect(await service.detectAfterEdit(ws.repo, "session-1")).toBe(true);
    expect(open((await service.status(ws.repo)).conflicts).find(conflict => conflict.id === waiting)).toMatchObject({ level: "checkpoint", queued: true, queuedFor: "stop" });
    // The next session's first detection has a stop to spend: it rises to it.
    now += 31_000;
    expect(await service.detectAfterEdit(ws.repo, "session-2")).toBe(true);
    expect(open((await service.status(ws.repo)).conflicts).find(conflict => conflict.id === waiting)).toMatchObject({ level: "stop", queued: false, queuedFor: null });
  });

  it("scores this branch's uncommitted lines the same on every pass, and counts new ones as they come", async () => {
    const ws = await overlaps(["src/retry.ts"], 0);
    const extra = (count: number): string => Array.from({ length: count }, (_, index) => `  const mine${index} = 0;`).join("\n");
    writeFiles(ws.repo, { "src/retry.ts": body("retry", { 22: `  const retry22 = "mine";\n${extra(200)}` }) });
    let now = Date.now();
    const service = ws.service({ now: () => now });
    await service.setEnabled(ws.repo, true);
    const [first] = open((await service.scan(ws.repo)).conflicts);
    expect(first!.signals).toMatchObject({ linesMine: 201 });
    now += 60_000;
    const [again] = open((await service.scan(ws.repo)).conflicts);
    expect(again!.signals).toMatchObject({ linesMine: 201 });
    expect(again!.score).toBeCloseTo(first!.score, 6);
    writeFiles(ws.repo, { "src/retry.ts": body("retry", { 22: `  const retry22 = "mine";\n${extra(210)}` }) });
    const [grown] = open((await service.scan(ws.repo)).conflicts);
    expect(grown!.signals).toMatchObject({ linesMine: 211 });
  });

  it("reads lock-in from both sides' full hunks, not the 40-line display: a moved export is a signature change, not a retirement", async () => {
    const file = "src/api.ts";
    const base = `${Array.from({ length: 40 }, (_, index) => (index + 1 === 20 ? "export function moved() {}" : `const line${index + 1} = ${index + 1};`)).join("\n")}\n`;
    const ws = await workspace({ files: { [file]: base } });
    const theirs = base.replace("export function moved() {}", [...Array.from({ length: 45 }, (_, index) => `const filler${index} = ${index};`), "export function moved(by: number) {}"].join("\n"));
    commit(ws.worktree("jbarnes-1"), { [file]: theirs }, "Move it");
    writeFiles(ws.repo, { [file]: base.replace("const line22 = 22;", "const line22 = \"mine\";") });
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const [conflict] = open((await service.scan(ws.repo)).conflicts);
    expect(conflict!.theirs.hunk).toContain("more lines]");
    expect(conflict!.theirs.hunk).not.toContain("+export function moved(by: number) {}");
    expect(conflict!.signals).toMatchObject({ lockIn: 2, lockInClass: "exported" });
  });

  it("shows the conflicts in the session's footer: the conflict waiting on the person at a checkpoint, until they dismiss it", async () => {
    const ws = await overlaps(["src/retry.ts"], 30);
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const store = new SqliteSessionStore(ws.databasePath);
    const reader = new SqliteSessionReader(ws.databasePath);
    try {
      store.openSession({ host: "claude-code", sessionId: "session-1", cwd: join(ws.repo, "src") }, Date.now());
      expect(reader.hud("session-1")).not.toHaveProperty("mergeAssist");
      const [conflict] = open((await service.scan(ws.repo)).conflicts);
      expect(reader.hud("session-1").mergeAssist).toEqual({ conflicts: 1, decide: { id: conflict!.id, title: conflict!.title }, summary: { id: conflict!.id, text: expect.any(String) } });
      expect(takeNotes(service, "session-1", ws.repo)).toHaveLength(1);
      await service.dismissConflict(ws.repo, conflict!.id);
      expect(reader.hud("session-1")).not.toHaveProperty("mergeAssist");
    } finally {
      store.close();
      reader.close();
    }
  });

  it("in a Claude Code session: asks the person at the edit of a contested file, and nothing once they dismiss it", async () => {
    const migration = "db/migrations/users.sql";
    const ws = await overlaps([migration], 60);
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const runtime = sessionRuntime(ws, service);
    const sessionId = "claude-session";
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId, cwd: ws.repo } });
      // The session's start scans in the background, and its detection spends the session's stop.
      await vi.waitFor(async () => expect(open((await service.status(ws.repo)).conflicts)[0]).toMatchObject({ level: "stop" }), { timeout: 20_000, interval: 100 });
      const deliveries = await runtime.ingest({
        kind: "call", eventId: "edit", sessionId, toolCallId: "1", toolName: "Edit", input: { file_path: join(ws.repo, migration), old_string: "a", new_string: "b" },
      });
      const output = claudeHookOutput("PreToolUse", deliveries, { links: "plain" }) as { systemMessage: string; hookSpecificOutput: Record<string, unknown> };
      expect(output.hookSpecificOutput).toMatchObject({ permissionDecision: "ask", permissionDecisionReason: expect.stringContaining(`Rerail: ${migration} is contested with jbarnes-1`) });
      expect(output.hookSpecificOutput.additionalContext).toContain("Rerail asked the user before this edit");
      expect(output.systemMessage).toContain("◆ Rerail · decide: users.sql · function users (jbarnes-1)");

      await runtime.ingest({ kind: "result", eventId: "edited", sessionId, toolCallId: "1", toolName: "Edit", output: "ok" });

      const [conflict] = open((await service.status(ws.repo)).conflicts);
      await service.dismissConflict(ws.repo, conflict!.id);
      const again = await runtime.ingest({
        kind: "call", eventId: "edit-2", sessionId, toolCallId: "2", toolName: "Edit", input: { file_path: join(ws.repo, migration), old_string: "b", new_string: "c" },
      });
      expect(again.filter(delivery => delivery.conflict !== undefined)).toEqual([]);
      expect(claudeHookOutput("PreToolUse", again)).not.toHaveProperty("hookSpecificOutput.permissionDecision");
    } finally {
      await runtime.dispose();
    }
  });

  it("in a Codex session: tells the agent not to write a contested file through the shell, and ask the person now", async () => {
    const migration = "db/migrations/users.sql";
    const ws = await overlaps([migration], 60);
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const runtime = sessionRuntime(ws, service);
    const sessionId = "codex-session";
    const call = async (id: string, command: string): Promise<Record<string, unknown>> => {
      const deliveries: AdapterDelivery[] = [];
      for (const event of codexHookEvents({ hook_event_name: "PreToolUse", session_id: sessionId, cwd: ws.repo, turn_id: "t1", tool_name: "Bash", tool_use_id: id, tool_input: { command } })) {
        deliveries.push(...await runtime.ingest(event));
      }
      return codexHookOutput("PreToolUse", deliveries);
    };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "codex", sessionId, cwd: ws.repo } });
      await vi.waitFor(async () => expect(open((await service.status(ws.repo)).conflicts)[0]).toMatchObject({ level: "stop" }), { timeout: 20_000, interval: 100 });
      // Reading it writes nothing.
      expect(await call("read", `cat ${migration} | head -5`)).not.toHaveProperty("hookSpecificOutput.additionalContext", expect.stringContaining("Don't edit"));
      const output = await call("write", `sed -i.bak 's/users/people/' ${migration} && echo done`);
      expect(output.hookSpecificOutput).toMatchObject({ hookEventName: "PreToolUse", additionalContext: expect.stringContaining(`Don't edit \`${migration}\` now`) });
      expect(output.hookSpecificOutput).not.toHaveProperty("permissionDecision");
      expect(output.systemMessage).toContain("decide: users.sql");
    } finally {
      await runtime.dispose();
    }
  });

  it("in a Codex session: a stop at the boundary that would also carry the checkpoint card gives one card and one person's line", async () => {
    const migration = "db/migrations/users.sql";
    const ws = await overlaps([migration], 60);
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const runtime = sessionRuntime(ws, service);
    const sessionId = "codex-once";
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "codex", sessionId, cwd: ws.repo } });
      await vi.waitFor(async () => expect(open((await service.status(ws.repo)).conflicts)[0]).toMatchObject({ level: "stop" }), { timeout: 20_000, interval: 100 });
      // The checkpoint card waits for the session's next boundary; that boundary is the edit of the contested file.
      const deliveries: AdapterDelivery[] = [];
      for (const event of codexHookEvents({ hook_event_name: "PreToolUse", session_id: sessionId, cwd: ws.repo, turn_id: "t1", tool_name: "Bash", tool_use_id: "write", tool_input: { command: `sed -i.bak 's/users/people/' ${migration}` } })) {
        deliveries.push(...await runtime.ingest(event));
      }
      const output = codexHookOutput("PreToolUse", deliveries) as { systemMessage: string; hookSpecificOutput: { additionalContext: string } };
      expect(output.systemMessage.split("\n").filter(line => line.includes("decide: users.sql"))).toHaveLength(1);
      expect(output.hookSpecificOutput.additionalContext.match(/◆ RERAIL · conflict #/gu)).toHaveLength(1);
      expect(output.hookSpecificOutput.additionalContext).toContain(`Don't edit \`${migration}\` now`);
      // The stop counted as the checkpoint card: the next boundary carries neither again.
      const next = await runtime.ingest({ kind: "user-prompt", eventId: "next", sessionId, text: "Go on." });
      expect(next.filter(delivery => delivery.conflict !== undefined)).toEqual([]);
    } finally {
      await runtime.dispose();
    }
  });

  it("reads the files a shell command writes, not the ones it only reads", async () => {
    const migration = "db/migrations/users.sql";
    const ws = await overlaps([migration], 60);
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    expect(await service.detectAfterEdit(ws.repo, "session-1")).toBe(true);
    const stops = (command: string, session: string): number => service.stopNotes(session, ws.repo, { toolName: "Bash", input: { command } }, false).length;
    expect(stops(`cat ${migration} > /tmp/copy.sql`, "s1")).toBe(0);
    expect(stops(`grep users ${migration}`, "s2")).toBe(0);
    for (const [index, command] of [
      `echo "-- note" >> ${migration}`,
      `printf x | tee -a ${join(ws.repo, migration)}`,
      `perl -pi -e 's/a/b/' ${migration}`,
      `cp /tmp/other.sql ${migration}`,
      `apply_patch <<'EOF'\n*** Begin Patch\n*** Update File: ${migration}\n@@\n-a\n+b\n*** End Patch\nEOF`,
    ].entries()) expect(stops(command, `w${index}`), command).toBe(1);
  });
});
