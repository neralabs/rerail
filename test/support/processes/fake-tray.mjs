/**
 * A stand-in for the tray program (`tray/`), for the daemon's side of its pipes (test/daemon/tray.test.ts). It puts no
 * icon up. What it does comes from its environment:
 * - FAKE_TRAY_LINES: a file it appends each menu-state line from stdin to, then `eof` when stdin closes.
 * - FAKE_TRAY_PID: a file it writes its pid to as it starts.
 * - FAKE_TRAY_REPLIES: a JSON array; entry i is written to stdout once stdin line i + 1 has arrived. An entry is null
 *   for nothing, a string, or an array of strings written as separate chunks 50 ms apart.
 * - FAKE_TRAY_STDERR: a line it writes to stderr as it starts.
 * - FAKE_TRAY_EXIT: when set, it exits with this code as it starts, after the stderr line.
 * Like the tray, it exits 0 when stdin closes.
 */
import { appendFileSync, writeFileSync } from "node:fs";
import { createInterface } from "node:readline";

const { FAKE_TRAY_LINES, FAKE_TRAY_PID, FAKE_TRAY_REPLIES, FAKE_TRAY_STDERR, FAKE_TRAY_EXIT } = process.env;

if (FAKE_TRAY_PID) writeFileSync(FAKE_TRAY_PID, String(process.pid));
if (FAKE_TRAY_STDERR) process.stderr.write(`${FAKE_TRAY_STDERR}\n`);
if (FAKE_TRAY_EXIT) process.exit(Number(FAKE_TRAY_EXIT));

const replies = FAKE_TRAY_REPLIES ? JSON.parse(FAKE_TRAY_REPLIES) : [];
let received = 0;

/** Writes `chunks` to stdout one at a time, 50 ms apart, so each arrives in a read of its own. */
function writeApart(chunks) {
  const [first, ...rest] = chunks;
  if (first === undefined) return;
  process.stdout.write(first);
  if (rest.length > 0) setTimeout(() => writeApart(rest), 50);
}

const lines = createInterface({ input: process.stdin });
lines.on("line", line => {
  if (FAKE_TRAY_LINES) appendFileSync(FAKE_TRAY_LINES, `${line}\n`);
  const reply = replies[received];
  received += 1;
  if (reply === undefined || reply === null) return;
  writeApart(Array.isArray(reply) ? reply : [reply]);
});
lines.on("close", () => {
  if (FAKE_TRAY_LINES) appendFileSync(FAKE_TRAY_LINES, "eof\n");
  process.exit(0);
});
