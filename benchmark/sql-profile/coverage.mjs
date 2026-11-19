import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

// A separate receipt detects a missing entire catalogue plus stream, not just a partial stream.
export function captureCoverage(directory, sessions) {
  const receipts = new Map(), problems = [];
  for (const file of readdirSync(directory).filter(n => /^coverage-\d+\.jsonl$/.test(n))) {
    for (const line of readFileSync(join(directory, file), "utf8").split("\n").filter(Boolean)) {
      const event = JSON.parse(line);
      if (event.version !== 1 || !["opened", "closed"].includes(event.event)
          || file !== `coverage-${event.pid}.jsonl`) throw new Error(`Invalid connection lifecycle receipt: ${file}`);
      const events = receipts.get(event.session) ?? [];
      events.push(event); receipts.set(event.session, events);
    }
  }
  const bySession = new Map(sessions.map(s => [s.session.replace(/\.json$/, ""), s]));
  for (const name of new Set([...receipts.keys(), ...bySession.keys()])) {
    const events = receipts.get(name) ?? [], session = bySession.get(name);
    if (!session || events.length !== 2 || events[0].event !== "opened" || events[1].event !== "closed"
        || events.some(e => e.pid !== session.pid)) problems.push(`Incomplete connection lifecycle: ${name}.`);
  }
  if (!receipts.size) problems.push("Missing connection lifecycle receipts.");
  return { complete: problems.length === 0, problems };
}
