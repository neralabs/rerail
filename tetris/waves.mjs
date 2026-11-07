// Keep native execution order. Start the next dependency wave when an evaluation
// reads a relation that an earlier evaluation in the current wave produced.
export function partitionWaves(rules, events) {
  const catalog = new Map(rules.map(rule => [rule.id, rule]));
  const waves = [];
  let produced = new Set();
  for (const [index, event] of events.entries()) {
    const reads = catalog.get(event.rule)?.reads ?? [];
    if (!waves.length || reads.some(read => produced.has(read.relation))) {
      waves.push({ id: waves.length + 1, start: index, end: index, updated: [], added: 0, removed: 0 });
      produced = new Set();
    }
    const wave = waves.at(-1);
    event.wave = wave.id;
    wave.end = index + 1;
    produced.add(event.head);
    const changes = event.changes ?? event;
    const added = changes.added?.length ?? 0, removed = changes.removed?.length ?? 0;
    wave.added += added; wave.removed += removed;
    if ((added || removed) && !wave.updated.includes(event.head)) wave.updated.push(event.head);
  }
  return waves;
}

// The slider is a prefix of completed evaluations. At an internal boundary,
// the next wave starts; at the end of the round, retain the final wave.
export function waveAtPosition(waves, position) {
  return waves.find(wave => position < wave.end) ?? waves.at(-1) ?? null;
}

export function frontierState(lastWave, currentWave) {
  return !lastWave ? 'unchanged' : lastWave === currentWave ? 'current' : 'previous';
}
