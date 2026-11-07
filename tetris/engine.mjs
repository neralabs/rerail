import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { randomInt } from 'node:crypto';
import { GameSession } from './game-session.mjs';

export const rules = await readFile(new URL('./game.rr', import.meta.url), 'utf8');
export { fact, diffFacts, preparePlayback } from './game-session.mjs';

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

// Native host for the same session used by the standalone Web Worker.
export class Game extends GameSession {
  constructor(engine, options = {}) {
    super(engine, { rules, entropy: () => randomInt(1, 2147483647), ...options });
  }
}
