import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { randomInt } from 'node:crypto';
import { partitionWaves } from './waves.mjs';

export const rules = await readFile(new URL('./game.rr', import.meta.url), 'utf8');
export const fact = (name, row) => `${name}(${row.map(value => JSON.stringify(value)).join(', ')}).`;

export function diffFacts(previous, current) {
  const index = relations => new Map(Object.entries(relations).flatMap(([relation, rows]) =>
    rows.map(row => [fact(relation, row), { relation, row }])));
  const before = index(previous), after = index(current);
  return {
    added: [...after].filter(([key]) => !before.has(key)).map(([, value]) => value),
    removed: [...before].filter(([key]) => !after.has(key)).map(([, value]) => value),
    unchanged: [...after.keys()].filter(key => before.has(key)).length,
  };
}

// Reconstruct a round's observable relation changes from native rule results.
// Each rule owns its contribution to a set; replacing one contribution preserves
// facts still supplied by another rule. This is trace presentation, not evaluation.
export function preparePlayback(evaluation) {
  const rowKey = row => JSON.stringify(row);
  const rules = new Map(evaluation.rules.map(rule => [rule.id, rule]));
  const contributions = new Map(evaluation.events.map(event => [event.rule, event.previous_candidates ?? []]));
  const relation = head => {
    const rows = new Map((evaluation.seeds[head] ?? []).map(row => [rowKey(row), row]));
    for (const [id, candidates] of contributions) {
      if (rules.get(id)?.head === head) for (const row of candidates) rows.set(rowKey(row), row);
    }
    return rows;
  };
  const heads = new Set(evaluation.rules.map(rule => rule.head));
  evaluation.initial = { ...evaluation.seeds, ...Object.fromEntries([...heads].map(head => [head, [...relation(head).values()]])) };
  for (const event of evaluation.events) {
    const before = relation(event.head);
    contributions.set(event.rule, event.candidates);
    const after = relation(event.head);
    event.changes = {
      added: [...after].filter(([key]) => !before.has(key)).map(([, row]) => row),
      removed: [...before].filter(([key]) => !after.has(key)).map(([, row]) => row),
    };
    event.display_outcome = event.changes.added.length || event.changes.removed.length ? 'changed' :
      event.cache_hit ? 'cached' : event.candidates.length ? 'unchanged' : 'no_match';
  }
  evaluation.waves = partitionWaves(evaluation.rules, evaluation.events);
  evaluation.wave_basis = 'Dependency barriers in recorded native execution order';
  return evaluation;
}

// This adapter knows only the next_* feedback convention, not the game state schema.
export class Engine {
  constructor() {
    this.child = spawn(fileURLToPath(new URL('./.cache/rerail-facts', import.meta.url)), [], {
      stdio: ['pipe', 'pipe', 'inherit'],
    });
    this.pending = [];
    createInterface({ input: this.child.stdout }).on('line', line => {
      const pending = this.pending.shift();
      if (!pending) return;
      try {
        const result = JSON.parse(line);
        if (result.error) pending.reject(new Error(result.error));
        else pending.resolve(result);
      } catch (error) { pending.reject(error); }
    });
    const fail = error => {
      this.failure = error;
      for (const pending of this.pending.splice(0)) pending.reject(error);
    };
    this.child.on('error', fail);
    this.child.on('exit', code => fail(new Error(`Fact engine exited (${code}). Run npm run build in tetris/.`)));
  }
  evaluate(source, options) {
    return this.evaluateDetailed(source, options).then(result => result.outputs);
  }
  evaluateDetailed(source, { trace = false } = {}) {
    if (this.failure) return Promise.reject(this.failure);
    return new Promise((resolve, reject) => {
      this.pending.push({ resolve, reject });
      this.child.stdin.write(JSON.stringify({ source, trace }) + '\n');
    });
  }
  close() { this.child.kill(); }
}

export class Game {
  constructor(engine) {
    this.engine = engine;
    this.state = '';
    this.latest = null;
    this.tail = Promise.resolve();
    this.sequence = 0;
    this.tracing = false;
    this.facts = {};
  }
  setTracing(enabled) {
    const operation = this.tail.then(() => { this.tracing = enabled; });
    this.tail = operation.catch(() => {});
    return operation;
  }
  dispatch(event, value) {
    const operation = this.tail.then(async () => {
      const input = fact(event, [value]);
      const entropy = fact('entropy', [randomInt(1, 2147483647)]);
      const started = performance.now();
      const { outputs, evaluation } = await this.engine.evaluateDetailed(`${rules}\n${this.state}\n${input}\n${entropy}`, { trace: this.tracing });
      // Replace every state relation, including relations whose output is empty.
      const state = Object.entries(outputs)
        .filter(([name]) => name.startsWith('next_'))
        .flatMap(([name, rows]) => rows.map(row => fact(name.slice(5), row))).join('\n');
      const screen = Object.fromEntries(Object.entries(outputs).filter(([name]) => name.startsWith('screen_')));
      if (evaluation) {
        preparePlayback(evaluation);
        evaluation.external = [event, 'entropy', ...Object.keys(outputs).filter(name => name.startsWith('next_')).map(name => name.slice(5))];
        evaluation.source = { name: 'game.rr', text: rules };
      }
      const facts = Object.fromEntries(Object.entries(outputs).map(([name, rows]) => [name.startsWith('next_') ? name.slice(5) : name, rows]));
      const changes = diffFacts(this.facts, facts);
      this.facts = facts;
      this.state = state;
      this.latest = {
        screen, sequence: ++this.sequence,
        trace: { input, inputs: [input, entropy], changes, evaluation,
          ms: Math.round((performance.now() - started) * 10) / 10, state,
          outputFacts: Object.values(screen).reduce((n, rows) => n + rows.length, 0) },
      };
      return this.latest;
    });
    this.tail = operation.catch(() => {});
    return operation;
  }
}
