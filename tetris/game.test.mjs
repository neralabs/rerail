import test, { after } from 'node:test';
import assert from 'node:assert/strict';
import { Engine, Game, rules, fact, diffFacts, preparePlayback } from './engine.mjs';
import { sourceSpan } from './evaluation.mjs';
import { sourceTokens } from './rr-source.mjs';
import { sourceCatalog } from './sources.mjs';
import { partitionWaves, waveAtPosition, frontierState } from './waves.mjs';

const engine = new Engine();
after(() => engine.close());
const evaluate = async (state = '', event = 'timestep(0).', seed = 7) =>
  engine.evaluate(`${rules}\n${state}\n${event}\nentropy(${seed}).`);
const initial = ({ k = 0, x = 3, y = 0, r = 0, score = 0, lines = 0, pieces = 0, mode = 'playing', seed = 1, clock = 0, timeMode = 'realtime', board = [] } = {}) => [
  fact('active', [k, x, y, r]), fact('stats', [score, lines, pieces]),
  fact('mode', [mode]), fact('rng', [seed]), fact('clock', [clock]),
  fact('time_mode', [timeMode]),
  ...board.map(row => fact('board', row)),
].join('\n');
const feedback = outputs => Object.entries(outputs).filter(([name]) => name.startsWith('next_'))
  .flatMap(([name, rows]) => rows.map(row => fact(name.slice(5), row))).join('\n');
const screenText = output => Object.fromEntries(output.screen_text);
function invariants(output) {
  for (const name of ['next_active', 'next_stats', 'next_mode', 'next_rng', 'next_clock']) assert.equal(output[name].length, 1, name);
  const board = output.screen_cell.filter(row => row[0] === 'board');
  assert.equal(board.length, 200);
  assert.equal(new Set(board.map(row => `${row[1]},${row[2]}`)).size, 200);
  assert.equal(output.screen_cell.filter(row => row[0] === 'preview').length, 16);
  for (const [x, y] of output.next_board) assert.ok(x >= 0 && x < 10 && y >= 0 && y < 20);
  assert.equal(new Set(output.next_board.map(row => `${row[0]},${row[1]}`)).size, output.next_board.length);
}

test('bootstrap derives all state and a complete fact screen', async () => {
  const out = await evaluate();
  assert.deepEqual(out.next_active, [[0, 3, 0, 0]]);
  assert.deepEqual(out.next_stats, [[0, 0, 0]]);
  assert.equal(out.screen_cell.filter(row => row[4] === 'active').length, 4);
  assert.equal(out.screen_cell.filter(row => row[4] === 'ghost').length, 4);
  invariants(out);
});

test('horizontal movement respects walls and occupied cells', async () => {
  let state = initial();
  for (let i = 0; i < 7; i++) state = feedback(await evaluate(state, 'keypress("ArrowLeft").'));
  let out = await evaluate(state, 'keypress("ArrowLeft").');
  assert.deepEqual(out.next_active, [[0, 0, 0, 0]]);
  out = await evaluate(initial({ board: [[2, 1, 1]] }), 'keypress("ArrowLeft").');
  assert.deepEqual(out.next_active, [[0, 3, 0, 0]]);
});

test('timesteps accumulate until the gravity threshold', async () => {
  let out = await evaluate(initial(), 'timestep(699).');
  assert.deepEqual(out.next_active, [[0, 3, 0, 0]]);
  assert.deepEqual(out.next_clock, [[699]]);
  out = await evaluate(feedback(out), 'timestep(1).');
  assert.deepEqual(out.next_active, [[0, 3, 1, 0]]);
  assert.deepEqual(out.next_clock, [[0]]);
});

test('level changes gravity speed', async () => {
  const out = await evaluate(initial({ lines: 10 }), 'timestep(645).');
  assert.deepEqual(out.next_active, [[0, 3, 1, 0]]);
  assert.equal(screenText(out).level, '2');
});

test('rotation uses both directions and kicks away from the wall', async () => {
  const clockwise = await evaluate(initial({ k: 2 }), 'keypress("ArrowUp").');
  assert.deepEqual(clockwise.next_active, [[2, 3, 0, 1]]);
  const back = await evaluate(feedback(clockwise), 'keypress("KeyZ").');
  assert.deepEqual(back.next_active, [[2, 3, 0, 0]]);
  const wall = await evaluate(initial({ x: -2, r: 1 }), 'keypress("ArrowUp").');
  assert.deepEqual(wall.next_active, [[0, 0, 0, 2]]);
});

test('soft drop awards one point and floor contact locks', async () => {
  const moved = await evaluate(initial(), 'keypress("ArrowDown").');
  assert.deepEqual(moved.next_active, [[0, 3, 1, 0]]);
  assert.deepEqual(moved.next_stats, [[1, 0, 0]]);
  const locked = await evaluate(initial({ y: 18 }), 'keypress("ArrowDown").');
  assert.equal(locked.next_board.length, 4);
  assert.deepEqual(locked.next_stats, [[0, 0, 1]]);
  invariants(locked);
});

test('hard drop lands above the floor and spawns the preview piece', async () => {
  const before = await evaluate(initial());
  const out = await evaluate(feedback(before), 'keypress("Space").');
  assert.deepEqual(out.next_board, [[3, 19, 0], [4, 19, 0], [5, 19, 0], [6, 19, 0]]);
  assert.deepEqual(out.next_active, [[1, 3, 0, 0]]);
  assert.deepEqual(out.next_stats, [[36, 0, 1]]);
  invariants(out);
});

test('line clearing compacts survivors and scores the line', async () => {
  const board = Array.from({ length: 10 }, (_, x) => [x, 19, 3]).filter(([x]) => x !== 4 && x !== 5);
  board.push([0, 17, 6]);
  const out = await evaluate(initial({ k: 1, board }), 'keypress("Space").');
  assert.deepEqual(out.next_board, [[0, 18, 6], [4, 19, 1], [5, 19, 1]]);
  assert.deepEqual(out.next_stats, [[136, 1, 1]]);
  invariants(out);
});

test('hard drop stops at the first obstacle even when space exists beneath it', async () => {
  const out = await evaluate(initial({ board: [[3, 10, 2]] }), 'keypress("Space").');
  for (const x of [3, 4, 5, 6]) assert.ok(out.next_board.some(([bx, by, k]) => bx === x && by === 9 && k === 0));
  assert.deepEqual(out.next_stats, [[16, 0, 1]]);
  invariants(out);
});

test('a vertical I clears four rows together', async () => {
  const board = [];
  for (let y = 16; y < 20; y++) for (let x = 0; x < 10; x++) if (x !== 4) board.push([x, y, 2]);
  const out = await evaluate(initial({ x: 2, r: 1, board }), 'keypress("Space").');
  assert.deepEqual(out.next_board, []);
  assert.deepEqual(out.next_stats, [[832, 4, 1]]);
  invariants(out);
});

test('pause freezes game state and the gravity clock', async () => {
  const paused = await evaluate(initial({ clock: 300 }), 'keypress("KeyP").');
  assert.deepEqual(paused.next_mode, [['paused']]);
  let state = feedback(paused);
  for (const event of ['timestep(900).', 'keypress("ArrowLeft").', 'keypress("Space").']) state = feedback(await evaluate(state, event));
  const out = await evaluate(state, 'keypress("KeyP").');
  assert.deepEqual(out.next_mode, [['playing']]);
  assert.deepEqual(out.next_active, [[0, 3, 0, 0]]);
  assert.deepEqual(out.next_clock, [[300]]);
  assert.deepEqual(out.next_board, []);
});

test('blocked spawn ends the game and restart clears all state', async () => {
  const over = await evaluate(initial({ x: 0, board: [[4, 0, 2]] }), 'keypress("Space").');
  assert.deepEqual(over.next_mode, [['over']]);
  assert.equal(screenText(over).overlay, 'GAME OVER');
  const frozen = await evaluate(feedback(over), 'timestep(1000).');
  assert.deepEqual(frozen.next_board, over.next_board);
  const reset = await evaluate(feedback(over), 'keypress("KeyR").', 9);
  assert.deepEqual(reset.next_active, [[2, 3, 0, 0]]);
  assert.deepEqual(reset.next_board, []);
  assert.deepEqual(reset.next_stats, [[0, 0, 0]]);
  assert.deepEqual(reset.next_mode, [['playing']]);
  invariants(reset);
});

test('unknown keys preserve state', async () => {
  const out = await evaluate(initial({ clock: 42 }), 'keypress("KeyQ").');
  assert.deepEqual(out.next_active, [[0, 3, 0, 0]]);
  assert.deepEqual(out.next_clock, [[42]]);
  invariants(out);
});

test('replay remains bounded and each frame has exactly one fact per cell', async () => {
  let out = await evaluate();
  for (let i = 0; i < 40; i++) {
    out = await evaluate(feedback(out), `keypress("${i % 3 === 0 ? 'ArrowLeft' : i % 3 === 1 ? 'ArrowUp' : 'Space'}").`);
    invariants(out);
  }
});

test('generic driver feeds next_* state back and serializes concurrent input', async () => {
  const game = new Game(engine);
  await game.dispatch('keypress', 'KeyR');
  const results = await Promise.all([game.dispatch('keypress', 'ArrowLeft'), game.dispatch('timestep', 100)]);
  assert.ok(results[1].sequence > results[0].sequence);
  assert.match(game.state, /active\(/);
  assert.equal(results[1].screen.screen_cell.length, 216);
  assert.equal(Object.keys(results[1].screen).every(name => name.startsWith('screen_')), true);
});

test('manual mode rejects timer facts and advances only on explicit step keys', async () => {
  let out = await evaluate(initial(), 'keypress("KeyM").');
  assert.deepEqual(out.next_time_mode, [['manual']]);
  out = await evaluate(feedback(out), 'timestep(1000).');
  assert.deepEqual(out.next_clock, [[0]]);
  assert.deepEqual(out.next_active, [[0, 3, 0, 0]]);
  for (let i = 0; i < 6; i++) out = await evaluate(feedback(out), 'keypress("KeyN").');
  assert.deepEqual(out.next_clock, [[600]]);
  assert.deepEqual(out.next_active, [[0, 3, 0, 0]]);
  out = await evaluate(feedback(out), 'keypress("KeyN").');
  assert.deepEqual(out.next_active, [[0, 3, 1, 0]]);
  assert.deepEqual(out.next_clock, [[0]]);
  invariants(out);
});

test('manual gravity step follows the level and restart preserves the time mode', async () => {
  const stepped = await evaluate(initial({ timeMode: 'manual', lines: 30 }), 'keypress("KeyG").');
  assert.deepEqual(stepped.next_active, [[0, 3, 1, 0]]);
  const restarted = await evaluate(feedback(stepped), 'keypress("KeyR").');
  assert.deepEqual(restarted.next_time_mode, [['manual']]);
  const realtime = await evaluate(feedback(restarted), 'keypress("KeyM").');
  assert.deepEqual(realtime.next_time_mode, [['realtime']]);
  const fallen = await evaluate(feedback(realtime), 'timestep(700).');
  assert.equal(fallen.next_active[0][2], 1);
});

test('fact differences retain removed rows and leave unchanged rows unmarked', () => {
  const delta = diffFacts({ board: [[0, 1, 2]], clock: [[0]], mode: [['playing']] },
    { board: [[0, 2, 2]], clock: [[0]], mode: [['playing']] });
  assert.deepEqual(delta.added, [{ relation: 'board', row: [0, 2, 2] }]);
  assert.deepEqual(delta.removed, [{ relation: 'board', row: [0, 1, 2] }]);
  assert.equal(delta.unchanged, 2);
});

test('native trace records actual insertions, duplicate rejection, and empty matches', async () => {
  const source = `
    .decl test_input(n: number)
    test_input(1). test_input(2).
    .decl result(n: number)
    result(n) :- test_input(n).
    result(n) :- test_input(n), n > 1.
    result(n) :- test_input(n), n > 9.
    .output result
  `;
  const { outputs, evaluation } = await engine.evaluateDetailed(source, { trace: true });
  assert.deepEqual(outputs.result, [[1], [2]]);
  assert.equal(evaluation.backend, 'Rerail SQL executor');
  assert.equal(evaluation.strategy, 'memoized');
  assert.equal(evaluation.events.length, 3);
  assert.deepEqual(evaluation.events.map(event => event.outcome), ['changed', 'unchanged', 'no_match']);
  assert.deepEqual(evaluation.events[0].added, [[1], [2]]);
  assert.deepEqual(evaluation.events[1].candidates, [[2]]);
  assert.equal(evaluation.events[1].inserted, 0);
  assert.deepEqual(evaluation.events[1].added, []);
  assert.deepEqual(evaluation.events[2].candidates, []);
  for (const event of evaluation.events) assert.equal(evaluation.rules[event.rule].head, event.head);
});

test('tracing preserves gameplay outputs and its event deltas reconstruct all outputs', async () => {
  const source = `${rules}\n${initial({ timeMode: 'manual' })}\nkeypress("KeyG").\nentropy(7).`;
  const expected = await engine.evaluate(source);
  const { outputs, evaluation } = await engine.evaluateDetailed(source, { trace: true });
  assert.deepEqual(outputs, expected);
  assert.equal(evaluation.events.length, evaluation.rules.length);
  const relationRows = new Map(Object.entries(evaluation.seeds).map(([name, rows]) => [name, new Map(rows.map(row => [JSON.stringify(row), row]))]));
  for (const event of evaluation.events) {
    if (!relationRows.has(event.head)) relationRows.set(event.head, new Map());
    const rows = relationRows.get(event.head);
    for (const row of event.removed) rows.delete(JSON.stringify(row));
    for (const row of event.added) rows.set(JSON.stringify(row), row);
  }
  for (const [name, rows] of Object.entries(outputs)) {
    assert.deepEqual(new Set([...(relationRows.get(name)?.values() ?? [])].map(JSON.stringify)), new Set(rows.map(JSON.stringify)), name);
  }
});

test('native tracing is optional and can be enabled between queued game rounds', async () => {
  const game = new Game(engine);
  const plain = await game.dispatch('keypress', 'KeyR');
  assert.equal(plain.trace.evaluation, null);
  await game.setTracing(true);
  const traced = await game.dispatch('keypress', 'KeyM');
  assert.ok(traced.trace.evaluation.events.length > 0);
  assert.ok(traced.trace.changes.removed.some(fact => fact.relation === 'time_mode'));
  await game.setTracing(false);
  assert.equal((await game.dispatch('keypress', 'KeyN')).trace.evaluation, null);
});

test('native cache skips SQL for identical inputs and invalidates equal-size changed sets and negation', async () => {
  const program = (value, blocked = false) => `
    .decl item(n: number)
    .decl blocked(n: number)
    item(${value}). ${blocked ? `blocked(${value}).` : ''}
    .decl available(n: number)
    available(n) :- item(n), !blocked(n).
    .output available
  `;
  const first = await engine.evaluateDetailed(program(1), { trace: true });
  assert.equal(first.evaluation.events[0].evaluated, true);
  const same = await engine.evaluateDetailed(program(1), { trace: true });
  assert.equal(same.evaluation.events[0].cache_hit, true);
  assert.equal(same.evaluation.events[0].evaluated, false);
  assert.deepEqual(same.outputs, first.outputs);
  const changed = await engine.evaluateDetailed(program(2), { trace: true });
  assert.equal(changed.evaluation.events[0].cache_hit, false);
  assert.deepEqual(changed.outputs.available, [[2]]);
  const negated = await engine.evaluateDetailed(program(2, true), { trace: true });
  assert.equal(negated.evaluation.events[0].cache_hit, false);
  assert.deepEqual(negated.outputs.available, []);
});

test('aggregate dependencies invalidate even when the number of input facts stays the same', async () => {
  const program = value => `
    .decl sample(v: number)
    sample(1). sample(${value}).
    .decl gate(v: number)
    gate(1).
    .decl total(v: number)
    total(n) :- gate(1), n = sum v : { sample(v) }.
    .output total
  `;
  const first = await engine.evaluateDetailed(program(2), { trace: true });
  assert.deepEqual(first.outputs.total, [[3]]);
  const same = await engine.evaluateDetailed(program(2), { trace: true });
  assert.ok(same.evaluation.events.some(event => event.cache_hit));
  const changed = await engine.evaluateDetailed(program(5), { trace: true });
  assert.deepEqual(changed.outputs.total, [[6]]);
  assert.ok(changed.evaluation.events.some(event => event.evaluated));
});

test('every traced game rule maps to its original source clause', async () => {
  const game = new Game(engine);
  await game.setTracing(true);
  const result = await game.dispatch('keypress', 'KeyR');
  const evaluation = result.trace.evaluation;
  assert.equal(evaluation.source.name, 'game.rr');
  assert.equal(evaluation.source.text, rules);
  for (const rule of evaluation.rules) {
    assert.ok(Number.isInteger(rule.source_line), `Missing source for rule ${rule.id}`);
    const source = sourceSpan(rules, rule.source_line);
    assert.ok(source, `Missing source span for rule ${rule.id}`);
    assert.ok(source.text.includes(':-'));
    if (!rule.head.startsWith('rr_')) assert.ok(source.text.startsWith(`${rule.head}(`), `${rule.id}: ${source.text}`);
  }
});

test('generated aggregate helpers retain the source of their parent clause', async () => {
  const source = [
    '.decl observed(v: number)', 'observed(1). observed(2).',
    '.decl allowed(v: number)', 'allowed(2).',
    '.decl enabled(v: number)', 'enabled(1).',
    '.decl answer(v: number)',
    'answer(n) :- enabled(1), n = count : { observed(v), allowed(v) }.',
    '.output answer',
  ].join('\n');
  const { outputs, evaluation } = await engine.evaluateDetailed(source, { trace: true });
  assert.deepEqual(outputs.answer, [[1]]);
  assert.ok(evaluation.rules.some(rule => rule.head.startsWith('rr_')));
  for (const rule of evaluation.rules) {
    assert.equal(rule.source_line, 8);
    assert.equal(sourceSpan(source, rule.source_line).text, source.split('\n')[7]);
  }
});

test('playback starts with the current screen even when prior rounds were not traced', async () => {
  const game = new Game(engine);
  await game.dispatch('keypress', 'KeyR');
  await game.dispatch('keypress', 'KeyM');
  const before = game.latest.screen;
  await game.setTracing(true);
  const result = await game.dispatch('keypress', 'KeyG');
  const evaluation = result.trace.evaluation;
  assert.deepEqual(new Set(evaluation.initial.screen_cell.map(JSON.stringify)), new Set(before.screen_cell.map(JSON.stringify)));
  assert.deepEqual(new Set(evaluation.initial.screen_text.map(JSON.stringify)), new Set(before.screen_text.map(JSON.stringify)));
  assert.equal(evaluation.initial.screen_cell.length, 216);
  assert.ok(evaluation.events.some(event => event.previous_candidates.length));
  const rows = new Map(Object.entries(evaluation.initial).map(([name, rows]) => [name, new Map(rows.map(row => [JSON.stringify(row), row]))]));
  for (const event of evaluation.events) {
    if (!rows.has(event.head)) rows.set(event.head, new Map());
    for (const row of event.changes.removed) rows.get(event.head).delete(JSON.stringify(row));
    for (const row of event.changes.added) rows.get(event.head).set(JSON.stringify(row), row);
  }
  for (const [name, values] of Object.entries(result.screen)) {
    assert.deepEqual(new Set([...rows.get(name).values()].map(JSON.stringify)), new Set(values.map(JSON.stringify)), name);
  }
  assert.ok(evaluation.events.some(event => event.changes.removed.length > 0));
  assert.ok(evaluation.events.filter(event => event.cache_hit).every(event => !event.changes.added.length && !event.changes.removed.length));
});

test('playback preserves a shared fact until its last contributing rule retracts it', () => {
  const evaluation = preparePlayback({
    rules: [{ id: 0, head: 'shared' }, { id: 1, head: 'shared' }], seeds: {},
    events: [
      { rule: 0, head: 'shared', previous_candidates: [[1]], candidates: [], cache_hit: false },
      { rule: 1, head: 'shared', previous_candidates: [[1]], candidates: [], cache_hit: false },
    ],
  });
  assert.deepEqual(evaluation.initial.shared, [[1]]);
  assert.deepEqual(evaluation.events[0].changes, { added: [], removed: [] });
  assert.deepEqual(evaluation.events[1].changes, { added: [], removed: [[1]] });
  assert.equal(evaluation.events[1].display_outcome, 'changed');
});

test('source highlighting preserves strings and comments and distinguishes .rr constructs', () => {
  const line = 'answer(n) :- sample(n), n >= 10, label("// quoted"). // real comment';
  const tokens = sourceTokens(line);
  assert.equal(tokens.map(token => token.text).join(''), line);
  assert.ok(tokens.some(token => token.kind === 'relation' && token.text === 'answer'));
  assert.ok(tokens.some(token => token.kind === 'number' && token.text === '10'));
  assert.ok(tokens.some(token => token.kind === 'operator' && token.text === ':-'));
  assert.ok(tokens.some(token => token.kind === 'string' && token.text === '"// quoted"'));
  assert.ok(tokens.some(token => token.kind === 'comment' && token.text === '// real comment'));
  const state = { blockComment: false };
  sourceTokens('/* start', state);
  assert.equal(sourceTokens('continued */ answer(n).', state)[0].kind, 'comment');
  assert.equal(state.blockComment, false);
});

test('source catalog exposes the running game and pinned .rr engine files only', async () => {
  const catalog = sourceCatalog(rules);
  const index = await catalog.list();
  assert.ok(index.files.length > 100);
  assert.ok(index.files.every(file => file.path.endsWith('.rr')));
  assert.equal((await catalog.source('game.rr')).text, rules);
  assert.match((await catalog.source('docs/rule-syntax/lib/core.rr')).text, /\.decl/);
  assert.equal(await catalog.source('../../.env'), null);
});

test('dependency waves preserve recorded order and separate dependent evaluations', () => {
  const rules = [
    { id: 0, head: 'a', reads: [{ relation: 'input', kind: 'positive' }] },
    { id: 1, head: 'b', reads: [{ relation: 'a', kind: 'negative' }] },
    { id: 2, head: 'c', reads: [{ relation: 'input', kind: 'positive' }] },
    { id: 3, head: 'd', reads: [{ relation: 'c', kind: 'aggregate' }, { relation: 'b', kind: 'positive' }] },
  ];
  const events = rules.map(rule => ({ rule: rule.id, head: rule.head, changes: { added: [], removed: [] } }));
  events[0].changes.added = [[1]];
  events[2].changes.added = [[2]];
  const waves = partitionWaves(rules, events);
  assert.deepEqual(events.map(event => event.rule), [0, 1, 2, 3]);
  assert.deepEqual(events.map(event => event.wave), [1, 2, 2, 3]);
  assert.deepEqual(waves.map(wave => [wave.start, wave.end]), [[0, 1], [1, 3], [3, 4]]);
  assert.deepEqual(waves[1].updated, ['c']);
  assert.equal(waveAtPosition(waves, 0).id, 1);
  assert.equal(waveAtPosition(waves, 2).id, 2);
  assert.equal(waveAtPosition(waves, 3).id, 3);
  assert.equal(waveAtPosition(waves, 4).id, 3);
});

test('frontier states distinguish initial facts, current updates, and earlier updates', () => {
  assert.equal(frontierState(undefined, 1), 'unchanged');
  assert.equal(frontierState(2, 2), 'current');
  assert.equal(frontierState(1, 2), 'previous');
  assert.deepEqual(partitionWaves([], []), []);
  assert.equal(waveAtPosition([], 0), null);
});

test('native game waves cover every evaluation and contain no dependency on an earlier member', async () => {
  const game = new Game(engine);
  await game.dispatch('keypress', 'KeyR');
  await game.setTracing(true);
  const { trace: { evaluation } } = await game.dispatch('keypress', 'ArrowLeft');
  assert.ok(evaluation.waves.length > 1);
  assert.equal(evaluation.waves[0].start, 0);
  assert.equal(evaluation.waves.at(-1).end, evaluation.events.length);
  for (const [index, wave] of evaluation.waves.entries()) {
    if (index) assert.equal(wave.start, evaluation.waves[index - 1].end);
    const produced = new Set();
    for (const event of evaluation.events.slice(wave.start, wave.end)) {
      const rule = evaluation.rules.find(rule => rule.id === event.rule);
      assert.equal(event.wave, wave.id);
      assert.ok(rule.reads.every(read => !produced.has(read.relation)));
      produced.add(event.head);
    }
  }
});
