// Keep native execution order. Advance the propagation frontier only when an
// accepted change reads a relation already changed in the current wave.
export function partitionWaves(rules, events) {
  const catalog = new Map(rules.map(rule => [rule.id, rule]));
  const waves = [];
  let changed = new Set();
  for (const [index, event] of events.entries()) {
    const reads = catalog.get(event.rule)?.reads ?? [];
    const changes = event.changes ?? event;
    const added = changes.added?.length ?? 0, removed = changes.removed?.length ?? 0;
    const accepted = added + removed > 0;
    if (!waves.length || (accepted && reads.some(read => changed.has(read.relation)))) {
      waves.push({ id: waves.length + 1, start: index, end: index, updated: [], checked: [], cached: 0, added: 0, removed: 0 });
      changed = new Set();
    }
    const wave = waves.at(-1);
    event.wave = wave.id;
    wave.end = index + 1;
    if (!wave.checked.includes(event.head)) wave.checked.push(event.head);
    if (event.cache_hit) wave.cached++;
    wave.added += added; wave.removed += removed;
    if (accepted) {
      changed.add(event.head);
      if (!wave.updated.includes(event.head)) wave.updated.push(event.head);
    }
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
