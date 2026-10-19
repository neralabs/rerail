import { afterEach, expect, test } from "vitest";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import Database from "better-sqlite3";
import { defaultRerailDatabasePath } from "../src/engine/index.js";
import { setUpTestRepository } from "../test/support/repository-rules.js";
import { EVENTS_DROPPED_ENTRY_TYPE, type OmpSessionEntryLike } from "../src/runtime/index.js";
import { OmpHost } from "./support/omp-host.js";

/**
 * NER-1955: a live daemon is taken for a lost one, and the session's later tool events are dropped.
 *
 * One shell command that writes many source files in the rules' scope (a test run writing its outputs, as the NER-1910
 * study's svelte runs did) makes the daemon hold the session's request lane past the omp extension's 30 s request
 * timeout: the turn end (and, on a native host, the command's tool result) waits in `RerailRuntime.#ingest` for the
 * audits of every file the command wrote, with no bound. `NativeSessionClient` takes the timed-out request for an
 * outage (`#fail`), shuts the event stream, and drops the events queued behind it: the next turns' tool calls and
 * results, which Rerail's records then lack. The daemon stays up throughout and records the slow request itself.
 */
const EMITTED_FILES = 8_000;
// What a compiler test run leaves per sample: an `_output` module with unsorted imports and branching render code.
const EMIT_SCRIPT = `const { mkdirSync, writeFileSync } = require("node:fs");
const output = index => \`import * as $ from "svelte/internal/client";
import { untrack } from "svelte";
import "svelte/internal/disclose-version";

export default function Sample\${index}($$anchor, $$props) {
  let value = $.prop($$props, "value", 3, \${index});
  if ($$props.a && $$props.b || $$props.c) value = untrack(() => value + 1);
  else if ($$props.d) value = value - 1;
  for (const key of Object.keys($$props)) {
    if (key === "x" || key === "y") $.set(value, $$props[key]);
    else if (key.startsWith("on")) $.event(key.slice(2), $$anchor, $$props[key]);
  }
  return value;
}
\`;
mkdirSync("api/_output", { recursive: true });
for (let index = 0; index < ${EMITTED_FILES}; index += 1) writeFileSync(\`api/_output/sample-\${index}.svelte.js\`, output(index));
`;
/** The JavaScript-side packs the NER-1910 study runs attached (it also attached python and next). */
const STUDY_PACKS = ["complexity", "javascript", "node", "typescript"];

let cwd: string | undefined;
afterEach(async () => {
  if (cwd !== undefined) await rm(cwd, { recursive: true, force: true });
  cwd = undefined;
});

/** Ids of the tool calls the assistant made in `branch`, in order. */
function toolCallIds(branch: readonly OmpSessionEntryLike[]): string[] {
  return branch.flatMap(entry => {
    if (!("message" in entry) || !Array.isArray(entry.message.content)) return [];
    return entry.message.content.flatMap((part: unknown) =>
      typeof part === "object" && part !== null && "type" in part && part.type === "toolCall" && "id" in part
        && typeof part.id === "string" ? [part.id] : []);
  });
}

function recordedEventIds(databasePath: string, sessionId: string): Set<string> {
  const database = new Database(databasePath, { readonly: true });
  try {
    const rows = database.prepare("SELECT event_id FROM events WHERE session_id = ?").all(sessionId) as Array<{ event_id: string }>;
    return new Set(rows.map(row => row.event_id));
  } finally { database.close(); }
}

test("NER-1955: a shell command that writes many sources costs no session events", async () => {
  cwd = await mkdtemp(join(tmpdir(), "rerail-ner-1955-"));
  const home = join(cwd, "home");
  const databasePath = defaultRerailDatabasePath({ RERAIL_HOME: home });
  await writeFile(join(cwd, "emit-outputs.cjs"), EMIT_SCRIPT);
  setUpTestRepository(cwd, { packs: STUDY_PACKS }, databasePath);
  const host = await OmpHost.start(cwd, [
    { content: [{ type: "toolCall", name: "bash", arguments: { command: "node emit-outputs.cjs" } }] },
    { content: [{ type: "toolCall", name: "bash", arguments: { command: "echo after the test run" } }] },
    { content: [{ type: "toolCall", name: "read", arguments: { path: "api/_output/sample-0.svelte.js" } }] },
    { content: ["Done."] },
  ]);
  try {
    await host.prompt("Run the tests, then look at one output.");
    const { sessionId, branch } = await host.snapshot();
    const expected = toolCallIds(branch).flatMap(id => [`tool-call:${id}`, `tool-result:${id}`]);
    expect(expected).toHaveLength(6);
    // Events still queued in the client reach the daemon late; a dropped one never does, and its recorded gap says so.
    const deadline = Date.now() + 60_000;
    let unrecorded = expected;
    let gapRecorded = false;
    while (unrecorded.length > 0 && !gapRecorded && Date.now() < deadline) {
      const recorded = recordedEventIds(databasePath, sessionId);
      unrecorded = expected.filter(id => !recorded.has(id));
      gapRecorded = [...recorded].some(id => id.startsWith("events-dropped:"));
      if (unrecorded.length > 0 && !gapRecorded) await delay(1_000);
    }
    const stderr = host.errors.join("");
    const daemonLog = await readFile(join(home, "daemon.log"), "utf8").catch(() => "");
    const evidence = [
      "omp stderr (Rerail lines):",
      ...stderr.split("\n").filter(line => /Rerail|default-extension/u.test(line)),
      "daemon log (turn-end waits):",
      ...daemonLog.split("\n").filter(line => line.includes("waited")),
    ].join("\n");
    const gaps = branch.filter(entry => "customType" in entry && entry.customType === EVENTS_DROPPED_ENTRY_TYPE);
    expect({ outage: stderr.includes("session events are dropped"), gaps: gaps.length, unrecorded }, evidence)
      .toEqual({ outage: false, gaps: 0, unrecorded: [] });
  } finally { await host.close(); }
}, 300_000);
