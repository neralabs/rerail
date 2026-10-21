import { createReadStream, readFileSync, readdirSync } from "node:fs";
import { createInterface } from "node:readline";
import { join } from "node:path";

// These are set steps inside SQL, not additional SQL executions. Their inclusive timings
// belong to the enclosing statement and must never enter the execution budget.
export async function controlStages(directory, clauseNames = new Map()) {
  const definitions = new Map(), stages = new Map();
  for (const name of readdirSync(directory).filter(name => /^cte-control-\d+-\d+\.jsonl$/.test(name))) {
    const pid = name.split("-")[2];
    let active;
    for await (const line of createInterface({ input: createReadStream(join(directory, name)), crlfDelay: Infinity })) {
      if (!line) continue;
      const event = JSON.parse(line);
      const program = `${pid}-${event.phase}`;
      if (!definitions.has(program)) definitions.set(program, JSON.parse(readFileSync(join(directory, `cte-program-${program}.json`), "utf8")));
      const definition = definitions.get(program)[event.pc];
      if (!definition || definition.pc !== event.pc) throw new Error(`Missing CTE instruction ${program}:${event.pc}`);
      const source=definition.source?.scope==="clause" ? {...definition.source,...clauseNames.get(definition.source.statement)} : definition.source;
      const key = JSON.stringify([definition.sql, definition.next, source]);
      if (!stages.has(key)) stages.set(key, { kind: "cte-set-step", sql: definition.sql, nextSql: definition.next,
        columns: definition.sql.match(/WITH payload\(([^)]*)\)/)?.[1].split(",") ?? [],
        source, cpuNs: 0, wallNs: 0, runs: 0, incomplete: 0 });
      const stage = stages.get(key);
      if (event.started) {
        if (active) throw new Error(`Unfinished CTE instruction before ${program}:${event.pc}`);
        active = { program, pc: event.pc, tick: event.tick, stage };
      } else {
        if (!active || active.program !== program || active.pc !== event.pc || active.tick !== event.tick) throw new Error(`Unmatched CTE completion ${program}:${event.pc}`);
        stage.cpuNs += event.cpuNs;
        stage.wallNs += event.wallNs;
        stage.runs++;
        active = undefined;
      }
    }
    if (active) active.stage.incomplete++;
  }
  return [...stages.values()];
}
